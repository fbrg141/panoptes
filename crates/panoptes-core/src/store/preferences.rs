use std::collections::BTreeSet;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use super::{Store, now_ms};

pub(super) const DEFAULT_CLEANUP_IDLE_DAYS: u32 = 4;

pub(super) const MAX_CLEANUP_IDLE_DAYS: u32 = 365;

/// A reusable choice for a new session. Empty fields keep harness defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: i64,
    pub name: String,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    /// Appended to the harness's system prompt when a new session starts.
    pub instructions: Option<String>,
}

/// The editable fields of an [`AgentProfile`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub name: String,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub instructions: Option<String>,
}

/// One independently persisted notification switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationField {
    Enabled,
    Finished,
    NeedsAttention,
}

/// When Panoptes posts a macOS notification about a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPreferences {
    /// Post notifications at all.
    pub enabled: bool,
    /// When a run finishes (Done).
    pub finished: bool,
    /// When a session waits for the user (Waiting).
    pub needs_attention: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            finished: true,
            needs_attention: true,
        }
    }
}

impl NotificationPreferences {
    pub fn set(&mut self, field: NotificationField, value: bool) {
        match field {
            NotificationField::Enabled => self.enabled = value,
            NotificationField::Finished => self.finished = value,
            NotificationField::NeedsAttention => self.needs_attention = value,
        }
    }
}

/// Everything Panoptes adds to session setup and model presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub profiles: Vec<AgentProfile>,
    /// Model ids in the order they were favorited.
    pub favorite_models: Vec<String>,
    pub hidden_models: Vec<String>,
    /// Inactive worktree lanes offered by cleanup after this many days.
    pub cleanup_idle_days: u32,
    /// Machine-local dismissal of the unguarded-tools warning.
    pub permission_warning_dismissed: bool,
    pub notifications: NotificationPreferences,
}

impl Store {
    pub(crate) fn pinned(&self) -> anyhow::Result<BTreeSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT session_key FROM pins")?;
        let keys = statement.query_map([], |row| row.get(0))?;
        Ok(keys.collect::<Result<_, _>>()?)
    }

    pub(crate) fn set_pinned(&self, key: &str, pinned: bool) -> anyhow::Result<()> {
        let sql = if pinned {
            "INSERT OR IGNORE INTO pins (session_key) VALUES (?1)"
        } else {
            "DELETE FROM pins WHERE session_key = ?1"
        };
        self.conn.execute(sql, [key])?;
        Ok(())
    }

    pub(crate) fn preferences(&self) -> anyhow::Result<Preferences> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, name, model_id, thinking_level, instructions
             FROM profiles ORDER BY position, id",
        )?;
        let profiles = statement
            .query_map([], |row| {
                Ok(AgentProfile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    model_id: row.get(2)?,
                    thinking_level: row.get(3)?,
                    instructions: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut statement = self.conn.prepare_cached(
            "SELECT model_id FROM model_preferences
             WHERE favorite_position IS NOT NULL ORDER BY favorite_position, model_id",
        )?;
        let favorite_models = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        let mut statement = self.conn.prepare_cached(
            "SELECT model_id FROM model_preferences WHERE hidden = 1 ORDER BY model_id",
        )?;
        let hidden_models = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        let cleanup_idle_days = self.conn.query_row(
            "SELECT idle_days FROM lane_cleanup_preferences WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let (permission_warning_dismissed, notifications) = self.conn.query_row(
            "SELECT permission_warning_dismissed, notify_enabled, notify_finished,
                    notify_needs_attention
             FROM ui_preferences WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    NotificationPreferences {
                        enabled: row.get(1)?,
                        finished: row.get(2)?,
                        needs_attention: row.get(3)?,
                    },
                ))
            },
        )?;
        Ok(Preferences {
            profiles,
            favorite_models,
            hidden_models,
            cleanup_idle_days,
            permission_warning_dismissed,
            notifications,
        })
    }

    pub(crate) fn save_profile(&self, id: Option<i64>, input: &ProfileInput) -> anyhow::Result<()> {
        let name = input.name.trim();
        if name.is_empty() {
            bail!("A profile needs a name");
        }
        let optional = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let model_id = optional(&input.model_id);
        let thinking_level = optional(&input.thinking_level);
        let instructions = optional(&input.instructions);
        let tx = self.write()?;
        if let Some(id) = id {
            let changed = tx.execute(
                "UPDATE profiles SET name = ?1, model_id = ?2, thinking_level = ?3,
                 instructions = ?4 WHERE id = ?5",
                rusqlite::params![name, model_id, thinking_level, instructions, id],
            )?;
            if changed == 0 {
                bail!("That profile no longer exists");
            }
        } else {
            tx.execute(
                "INSERT INTO profiles
                 (name, model_id, thinking_level, instructions, position, created_at)
                 VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(position) + 1, 0) FROM profiles), ?5)",
                rusqlite::params![name, model_id, thinking_level, instructions, now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn delete_profile(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub(crate) fn set_model_favorite(&self, model_id: &str, favorite: bool) -> anyhow::Result<()> {
        let tx = self.write()?;
        if favorite {
            tx.execute(
                "INSERT INTO model_preferences (model_id, favorite_position)
                 VALUES (?1, (SELECT COALESCE(MAX(favorite_position) + 1, 0) FROM model_preferences))
                 ON CONFLICT (model_id) DO UPDATE SET favorite_position = excluded.favorite_position
                 WHERE model_preferences.favorite_position IS NULL",
                [model_id],
            )?;
        } else {
            tx.execute(
                "UPDATE model_preferences SET favorite_position = NULL WHERE model_id = ?1",
                [model_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn set_model_hidden(&self, model_id: &str, hidden: bool) -> anyhow::Result<()> {
        if hidden {
            self.conn.execute(
                "INSERT INTO model_preferences (model_id, hidden) VALUES (?1, 1)
                 ON CONFLICT (model_id) DO UPDATE SET hidden = 1",
                [model_id],
            )?;
        } else {
            self.conn.execute(
                "UPDATE model_preferences SET hidden = 0 WHERE model_id = ?1",
                [model_id],
            )?;
        }
        Ok(())
    }

    pub(crate) fn dismiss_permission_warning(&self) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE ui_preferences SET permission_warning_dismissed = 1 WHERE singleton = 1",
            [],
        )?;
        Ok(())
    }

    pub(crate) fn set_notification(
        &self,
        field: NotificationField,
        value: bool,
    ) -> anyhow::Result<()> {
        // A toggle owns one column. Writing a snapshot of all three would let
        // a quick second click restore the first click's old value.
        let column = match field {
            NotificationField::Enabled => "notify_enabled",
            NotificationField::Finished => "notify_finished",
            NotificationField::NeedsAttention => "notify_needs_attention",
        };
        self.conn.execute(
            &format!("UPDATE ui_preferences SET {column} = ?1 WHERE singleton = 1"),
            [value],
        )?;
        Ok(())
    }

    pub(crate) fn set_cleanup_idle_days(&self, days: u32) -> anyhow::Result<()> {
        if !(1..=MAX_CLEANUP_IDLE_DAYS).contains(&days) {
            bail!("Idle lane threshold must be between 1 and {MAX_CLEANUP_IDLE_DAYS} days");
        }
        self.conn.execute(
            "UPDATE lane_cleanup_preferences SET idle_days = ?1 WHERE singleton = 1",
            [days],
        )?;
        Ok(())
    }
}
