use anyhow::{Context, bail};
use rusqlite::OptionalExtension;
use serde::Serialize;

use super::{LaneState, Store, now_ms};

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

impl Store {
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
