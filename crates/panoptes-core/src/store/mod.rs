//! Panoptes' own data, in a SQLite database in the app data folder. Harnesses
//! keep their conversations; this holds what Panoptes adds on top (pins,
//! projects, lanes, session profiles and model-picker preferences, and the
//! sessions that run in lanes now, then groups, see `docs/domain-model.md`).
//!
//! The schema version is SQLite's `user_version`. Opening the store runs
//! every migration it is missing, each in its own transaction, after copying
//! the database aside. A damaged database, one written by a newer Panoptes,
//! or a migration that fails stops the open with an error: the store never
//! starts empty in place of data it could not read.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

mod legacy;
mod migrate;
mod migrations;

use migrate::{check, explain, migrate};
use migrations::{MIGRATIONS, Migration};

const DATABASE: &str = "panoptes.db";
/// How long to wait for another Panoptes that holds the database.
const BUSY_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(5)
};
const DEFAULT_CLEANUP_IDLE_DAYS: u32 = 4;
pub const MAX_CLEANUP_IDLE_DAYS: u32 = 365;

/// A registered git repository and the settings every new worktree of it
/// is made with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    /// Path of the main checkout.
    pub repo: String,
    #[serde(flatten)]
    pub settings: ProjectSettings,
}

/// What can be changed about a project after it is registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub name: String,
    /// What new worktrees branch from.
    pub base_branch: String,
    /// Untracked files copied from the main checkout into new worktrees,
    /// relative to it.
    pub copy_files: Vec<String>,
    /// Run in a new worktree once it is made, such as `pnpm install`.
    pub setup: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaneState {
    Active,
    Archived,
}

impl LaneState {
    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneWorktree {
    pub project_id: i64,
    pub path: String,
    pub main_checkout: bool,
    /// Private ref holding the final dirty tree, when archived as a checkpoint.
    pub checkpoint_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLaneWorktree {
    pub project_id: i64,
    pub path: String,
    pub main_checkout: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lane {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub task: String,
    pub branch: String,
    pub state: LaneState,
    pub worktrees: Vec<LaneWorktree>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLane {
    pub project_id: i64,
    pub name: String,
    pub task: String,
    pub branch: String,
    pub worktrees: Vec<NewLaneWorktree>,
}

/// A Panoptes session: its own identity, the lane it belongs to, and the
/// harness conversation it wraps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: i64,
    pub lane_id: i64,
    pub harness: String,
    /// The harness's session id and locator, once it has reported them.
    pub harness_id: Option<String>,
    pub locator: Option<String>,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub created_at: i64,
    /// Additional system-prompt instructions retained across process restarts.
    pub instructions: Option<String>,
}

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
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the store in `dir`, creating it or migrating it to the current
    /// schema as needed.
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        Self::open_with(dir, MIGRATIONS)
    }

    fn open_with(dir: &Path, migrations: &[Migration]) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Could not create {}", dir.display()))?;
        let path = dir.join(DATABASE);
        let mut conn = Connection::open(&path)
            .with_context(|| format!("Could not open {}", path.display()))?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        let result = check(&conn).and_then(|version| migrate(&mut conn, dir, version, migrations));
        result.with_context(|| format!("Could not load Panoptes data from {}", path.display()))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(Self { conn })
    }

    /// A transaction that takes the write lock at once. Every operation that
    /// checks something and then writes uses it, so a second Panoptes on the
    /// same store waits for the whole check-and-write, never between the two.
    fn write(&self) -> anyhow::Result<Transaction<'_>> {
        Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)
            .map_err(|error| explain(error, "Could not start a store change"))
    }

    pub fn pinned(&self) -> anyhow::Result<BTreeSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT session_key FROM pins")?;
        let keys = statement.query_map([], |row| row.get(0))?;
        Ok(keys.collect::<Result<_, _>>()?)
    }

    pub fn set_pinned(&self, key: &str, pinned: bool) -> anyhow::Result<()> {
        let sql = if pinned {
            "INSERT OR IGNORE INTO pins (session_key) VALUES (?1)"
        } else {
            "DELETE FROM pins WHERE session_key = ?1"
        };
        self.conn.execute(sql, [key])?;
        Ok(())
    }

    pub fn preferences(&self) -> anyhow::Result<Preferences> {
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
        let permission_warning_dismissed = self.conn.query_row(
            "SELECT permission_warning_dismissed FROM ui_preferences WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(Preferences {
            profiles,
            favorite_models,
            hidden_models,
            cleanup_idle_days,
            permission_warning_dismissed,
        })
    }

    pub fn save_profile(&self, id: Option<i64>, input: &ProfileInput) -> anyhow::Result<()> {
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

    pub fn delete_profile(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_model_favorite(&self, model_id: &str, favorite: bool) -> anyhow::Result<()> {
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

    pub fn set_model_hidden(&self, model_id: &str, hidden: bool) -> anyhow::Result<()> {
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

    pub fn dismiss_permission_warning(&self) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE ui_preferences SET permission_warning_dismissed = 1 WHERE singleton = 1",
            [],
        )?;
        Ok(())
    }

    pub fn set_cleanup_idle_days(&self, days: u32) -> anyhow::Result<()> {
        if !(1..=MAX_CLEANUP_IDLE_DAYS).contains(&days) {
            bail!("Idle lane threshold must be between 1 and {MAX_CLEANUP_IDLE_DAYS} days");
        }
        self.conn.execute(
            "UPDATE lane_cleanup_preferences SET idle_days = ?1 WHERE singleton = 1",
            [days],
        )?;
        Ok(())
    }

    /// Every project, by name.
    pub fn projects(&self) -> anyhow::Result<Vec<Project>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, repo, name, base_branch, setup FROM projects ORDER BY name COLLATE NOCASE, id",
        )?;
        let mut projects = statement
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    settings: ProjectSettings {
                        name: row.get(2)?,
                        base_branch: row.get(3)?,
                        setup: row.get(4)?,
                        copy_files: Vec::new(),
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut files = self.conn.prepare_cached(
            "SELECT project_id, path FROM project_copy_files ORDER BY project_id, position",
        )?;
        let files = files.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))?;
        for file in files {
            let (id, path) = file?;
            if let Some(project) = projects.iter_mut().find(|p| p.id == id) {
                project.settings.copy_files.push(path);
            }
        }
        Ok(projects)
    }

    /// Registers the repository whose main checkout is `repo`. Fails if it is
    /// already registered.
    pub fn add_project(&self, repo: &str, settings: &ProjectSettings) -> anyhow::Result<Project> {
        let tx = self.write()?;
        let existing: Option<String> = tx
            .query_row("SELECT name FROM projects WHERE repo = ?1", [repo], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(name) = existing {
            bail!("{repo} is already registered as {name}");
        }
        tx.execute(
            "INSERT INTO projects (name, repo, base_branch, setup, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                settings.name,
                repo,
                settings.base_branch,
                settings.setup,
                now_ms()
            ],
        )?;
        let id = tx.last_insert_rowid();
        write_copy_files(&tx, id, &settings.copy_files)?;
        tx.commit()?;
        Ok(Project {
            id,
            repo: repo.to_string(),
            settings: settings.clone(),
        })
    }

    pub fn update_project(&self, id: i64, settings: &ProjectSettings) -> anyhow::Result<Project> {
        let tx = self.write()?;
        let repo: Option<String> = tx
            .query_row(
                "UPDATE projects SET name = ?2, base_branch = ?3, setup = ?4 WHERE id = ?1
                 RETURNING repo",
                rusqlite::params![id, settings.name, settings.base_branch, settings.setup],
                |row| row.get(0),
            )
            .optional()?;
        let Some(repo) = repo else {
            bail!("That project is no longer registered");
        };
        tx.execute("DELETE FROM project_copy_files WHERE project_id = ?1", [id])?;
        write_copy_files(&tx, id, &settings.copy_files)?;
        tx.commit()?;
        Ok(Project {
            id,
            repo,
            settings: settings.clone(),
        })
    }

    pub fn project(&self, id: i64) -> anyhow::Result<Project> {
        self.projects()?
            .into_iter()
            .find(|project| project.id == id)
            .context("That project is no longer registered")
    }

    pub fn lanes(&self) -> anyhow::Result<Vec<Lane>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, project_id, name, task, branch, state
             FROM lanes ORDER BY created_at, id",
        )?;
        let lanes = statement
            .query_map([], lane_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        lanes
            .into_iter()
            .map(|mut lane| {
                lane.worktrees = self.lane_worktrees(lane.id)?;
                Ok(lane)
            })
            .collect()
    }

    pub fn lane(&self, id: i64) -> anyhow::Result<Lane> {
        let mut lane = self
            .conn
            .query_row(
                "SELECT id, project_id, name, task, branch, state FROM lanes WHERE id = ?1",
                [id],
                lane_from_row,
            )
            .optional()?
            .context("That lane no longer exists")?;
        lane.worktrees = self.lane_worktrees(id)?;
        Ok(lane)
    }

    /// Creation is the activity baseline for a lane that has no session yet.
    pub fn lane_creation_times(&self) -> anyhow::Result<Vec<(i64, i64)>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT id, created_at FROM lanes")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    #[cfg(test)]
    pub fn set_lane_created_at(&self, id: i64, created_at: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE lanes SET created_at = ?2 WHERE id = ?1",
            rusqlite::params![id, created_at],
        )?;
        Ok(())
    }

    fn lane_worktrees(&self, lane_id: i64) -> anyhow::Result<Vec<LaneWorktree>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT project_id, path, main_checkout, checkpoint_ref
             FROM lane_worktrees WHERE lane_id = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([lane_id], |row| {
            Ok(LaneWorktree {
                project_id: row.get(0)?,
                path: row.get(1)?,
                main_checkout: row.get(2)?,
                checkpoint_ref: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Saves a lane only after its worktree and setup have succeeded. The
    /// schema accepts several worktrees; the service currently supplies one.
    pub fn add_lane(&self, new: &NewLane) -> anyhow::Result<Lane> {
        let tx = self.write()?;
        for worktree in &new.worktrees {
            ensure_path_free(&tx, &worktree.path, None)?;
        }
        tx.execute(
            "INSERT INTO lanes (project_id, name, task, branch, state, created_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
            rusqlite::params![new.project_id, new.name, new.task, new.branch, now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO lane_worktrees
                 (lane_id, project_id, position, path, main_checkout)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (position, worktree) in new.worktrees.iter().enumerate() {
                insert.execute(rusqlite::params![
                    id,
                    worktree.project_id,
                    position as i64,
                    worktree.path,
                    worktree.main_checkout,
                ])?;
            }
        }
        tx.commit()?;
        self.lane(id)
    }

    pub fn archive_lane(&self, id: i64, checkpoint_ref: Option<&str>) -> anyhow::Result<Lane> {
        let tx = self.write()?;
        let changed = tx.execute(
            "UPDATE lanes SET state = 'archived' WHERE id = ?1 AND state = 'active'",
            [id],
        )?;
        if changed == 0 {
            bail!("That lane is not active");
        }
        tx.execute(
            "UPDATE lane_worktrees SET checkpoint_ref = ?2 WHERE lane_id = ?1",
            rusqlite::params![id, checkpoint_ref],
        )?;
        tx.commit()?;
        self.lane(id)
    }

    /// Notes, on a lane that is still active, the checkpoint ref an archive
    /// in progress has saved its work to (or forgets it). If that archive is
    /// interrupted after removing the folder, the next one finds the ref
    /// here.
    pub fn set_lane_checkpoint(&self, id: i64, checkpoint_ref: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE lane_worktrees SET checkpoint_ref = ?2 WHERE lane_id = ?1",
            rusqlite::params![id, checkpoint_ref],
        )?;
        Ok(())
    }

    pub fn restore_lane(&self, id: i64) -> anyhow::Result<Lane> {
        let tx = self.write()?;
        for worktree in self.lane_worktrees(id)? {
            ensure_path_free(&tx, &worktree.path, Some(id))?;
        }
        let changed = tx.execute(
            "UPDATE lanes SET state = 'active' WHERE id = ?1 AND state = 'archived'",
            [id],
        )?;
        if changed == 0 {
            bail!("That lane is not archived");
        }
        // The checkpoint is back in the worktree; the lane no longer rests
        // on it.
        tx.execute(
            "UPDATE lane_worktrees SET checkpoint_ref = NULL WHERE lane_id = ?1",
            [id],
        )?;
        tx.commit()?;
        self.lane(id)
    }

    /// Permanently forgets a lane and its Panoptes session records. The
    /// harness conversations remain in the harness's own files. Git cleanup
    /// is completed by the service before this transaction starts.
    pub fn delete_lane(&self, id: i64) -> anyhow::Result<()> {
        let tx = self.write()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM lanes WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if !exists {
            bail!("That lane no longer exists");
        }
        tx.execute("DELETE FROM sessions WHERE lane_id = ?1", [id])?;
        tx.execute("DELETE FROM lanes WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Forgets a project with no lanes. Its repository and worktrees on disk
    /// are not touched. A project with lanes stays registered so their records
    /// and recovery refs cannot silently lose the repository they refer to.
    /// Every Panoptes session, oldest first, archived ones included.
    pub fn sessions(&self) -> anyhow::Result<Vec<SessionRecord>> {
        let mut statement = self.conn.prepare_cached(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at, id"
        ))?;
        let records = statement
            .query_map([], session_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|record| self.with_tags(record))
            .collect()
    }

    pub fn session(&self, id: i64) -> anyhow::Result<SessionRecord> {
        let record = self
            .conn
            .query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                [id],
                session_from_row,
            )
            .optional()?
            .context("That session no longer exists")?;
        self.with_tags(record)
    }

    /// The Panoptes session wrapping a harness conversation, if any.
    pub fn session_for(&self, harness: &str, harness_id: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE harness = ?1 AND harness_id = ?2",
                [harness, harness_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn with_tags(&self, mut record: SessionRecord) -> anyhow::Result<SessionRecord> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT tag FROM session_tags WHERE session_id = ?1 ORDER BY tag")?;
        record.tags = statement
            .query_map([record.id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(record)
    }

    /// Gives a session its Panoptes id before any harness process exists.
    /// Records accumulate; the service permits only one live process per lane.
    #[cfg(test)]
    pub fn add_session(&self, lane_id: i64, harness: &str) -> anyhow::Result<SessionRecord> {
        self.add_session_configured(lane_id, harness, None)
    }

    pub fn add_session_configured(
        &self,
        lane_id: i64,
        harness: &str,
        instructions: Option<&str>,
    ) -> anyhow::Result<SessionRecord> {
        let tx = self.write()?;
        if self.lane(lane_id)?.state != LaneState::Active {
            bail!("That lane is archived; restore it to start a session");
        }
        let instructions = instructions.map(str::trim).filter(|text| !text.is_empty());
        tx.execute(
            "INSERT INTO sessions (lane_id, harness, instructions, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![lane_id, harness, instructions, now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        self.session(id)
    }

    /// Adopts an existing harness conversation into a lane without starting
    /// a process or touching the harness's files. A legacy pin follows the
    /// conversation into its new Panoptes record.
    pub fn adopt_session(
        &self,
        lane_id: i64,
        harness: &str,
        harness_id: &str,
        locator: &str,
    ) -> anyhow::Result<SessionRecord> {
        let tx = self.write()?;
        if self.lane(lane_id)?.state != LaneState::Active {
            bail!("That lane is archived; restore it before adopting a session");
        }
        if self.session_for(harness, harness_id)?.is_some() {
            bail!("That conversation already belongs to a lane");
        }
        let legacy_key = format!("{harness}:{harness_id}");
        let pinned = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM pins WHERE session_key = ?1)",
            [&legacy_key],
            |row| row.get::<_, bool>(0),
        )?;
        tx.execute(
            "INSERT INTO sessions
             (lane_id, harness, harness_id, locator, pinned, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![lane_id, harness, harness_id, locator, pinned, now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute("DELETE FROM pins WHERE session_key = ?1", [&legacy_key])?;
        tx.commit()?;
        self.session(id)
    }

    /// Records which harness conversation a session wraps, once the harness
    /// has reported it.
    pub fn set_harness_ref(&self, id: i64, harness_id: &str, locator: &str) -> anyhow::Result<()> {
        let locator = (!locator.is_empty()).then_some(locator);
        self.conn.execute(
            "UPDATE sessions SET harness_id = ?2, locator = ?3 WHERE id = ?1",
            rusqlite::params![id, harness_id, locator],
        )?;
        Ok(())
    }

    pub fn set_session_title(&self, id: i64, title: Option<&str>) -> anyhow::Result<()> {
        let title = title.map(str::trim).filter(|title| !title.is_empty());
        self.update_session(id, "UPDATE sessions SET title = ?2 WHERE id = ?1", title)
    }

    pub fn set_session_pinned(&self, id: i64, pinned: bool) -> anyhow::Result<()> {
        self.update_session(id, "UPDATE sessions SET pinned = ?2 WHERE id = ?1", pinned)
    }

    /// Archiving a session hides it; its lane is untouched.
    pub fn set_session_archived(&self, id: i64, archived: bool) -> anyhow::Result<()> {
        self.update_session(
            id,
            "UPDATE sessions SET archived = ?2 WHERE id = ?1",
            archived,
        )
    }

    pub fn set_session_tags(&self, id: i64, tags: &[String]) -> anyhow::Result<()> {
        let tx = self.write()?;
        if tx.execute("DELETE FROM session_tags WHERE session_id = ?1", [id])? == 0
            && !tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM sessions WHERE id = ?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )?
        {
            bail!("That session no longer exists");
        }
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO session_tags (session_id, tag) VALUES (?1, ?2)",
        )?;
        for tag in tags
            .iter()
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
        {
            insert.execute(rusqlite::params![id, tag])?;
        }
        drop(insert);
        tx.commit()?;
        Ok(())
    }

    /// Forgets a session whose harness never started, so no record points
    /// at nothing.
    pub fn discard_session(&self, id: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM sessions WHERE id = ?1 AND harness_id IS NULL",
            [id],
        )?;
        Ok(())
    }

    fn update_session(
        &self,
        id: i64,
        sql: &str,
        value: impl rusqlite::ToSql,
    ) -> anyhow::Result<()> {
        if self.conn.execute(sql, rusqlite::params![id, value])? == 0 {
            bail!("That session no longer exists");
        }
        Ok(())
    }

    pub fn remove_project(&self, id: i64) -> anyhow::Result<()> {
        let tx = self.write()?;
        let lanes: i64 = tx.query_row(
            "SELECT count(*) FROM lanes WHERE project_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if lanes > 0 {
            bail!("That project still has lanes and cannot be removed");
        }
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}

/// Refuses a worktree path that an active lane (other than `except`) uses.
fn ensure_path_free(tx: &Transaction, path: &str, except: Option<i64>) -> anyhow::Result<()> {
    let used: bool = tx.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM lane_worktrees JOIN lanes ON lanes.id = lane_worktrees.lane_id
             WHERE lane_worktrees.path = ?1 AND lanes.state = 'active'
               AND lanes.id IS NOT ?2)",
        rusqlite::params![path, except],
        |row| row.get(0),
    )?;
    if used {
        bail!("Another active lane already uses {path}");
    }
    Ok(())
}

fn lane_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Lane> {
    let state: String = row.get(5)?;
    Ok(Lane {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        task: row.get(3)?,
        branch: row.get(4)?,
        state: LaneState::parse(&state)?,
        worktrees: Vec::new(),
    })
}

const SESSION_COLUMNS: &str =
    "id, lane_id, harness, harness_id, locator, title, pinned, archived, created_at, instructions";

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        id: row.get(0)?,
        lane_id: row.get(1)?,
        harness: row.get(2)?,
        harness_id: row.get(3)?,
        locator: row.get(4)?,
        title: row.get(5)?,
        tags: Vec::new(),
        pinned: row.get(6)?,
        archived: row.get(7)?,
        created_at: row.get(8)?,
        instructions: row.get(9)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn write_copy_files(tx: &Transaction, id: i64, files: &[String]) -> anyhow::Result<()> {
    let mut insert = tx.prepare_cached(
        "INSERT INTO project_copy_files (project_id, position, path) VALUES (?1, ?2, ?3)",
    )?;
    for (position, path) in files.iter().enumerate() {
        insert.execute(rusqlite::params![id, position as i64, path])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
