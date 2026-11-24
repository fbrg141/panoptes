//! Durable lane turn boundaries and their per-worktree snapshots.

use anyhow::{Context, bail};
use rusqlite::{Error as SqlError, OptionalExtension};
use serde::Serialize;

use super::{Lane, Store, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Boundary {
    Unavailable,
    Observed,
    Exact,
}

impl Boundary {
    fn sql(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Observed => "observed",
            Self::Exact => "exact",
        }
    }

    fn from_sql(value: &str, column: &str) -> rusqlite::Result<Self> {
        match value {
            "unavailable" => Ok(Self::Unavailable),
            "observed" => Ok(Self::Observed),
            "exact" => Ok(Self::Exact),
            _ => Err(SqlError::InvalidColumnName(column.into())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TurnOutcome {
    Done,
    Failed,
    Stopped,
}

impl TurnOutcome {
    fn sql(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    fn from_sql(value: &str) -> rusqlite::Result<Self> {
        match value {
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "stopped" => Ok(Self::Stopped),
            _ => Err(SqlError::InvalidColumnName("outcome".into())),
        }
    }
}

/// One lane-level turn. `prompt` is a short label in Panoptes' own store,
/// never the event log; worktree trees and refs remain separate rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub lane_id: i64,
    pub turn: u32,
    pub session_id: i64,
    pub message_id: Option<String>,
    pub prompt: String,
    pub queued: bool,
    pub started_at: i64,
    pub settled_at: Option<i64>,
    pub outcome: Option<TurnOutcome>,
    pub worktrees: Vec<TurnWorktree>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnWorktree {
    pub project_id: i64,
    pub before_ref: String,
    pub after_ref: String,
    pub before_tree: Option<String>,
    pub after_tree: Option<String>,
    pub boundary: Boundary,
    pub human_edits_before: bool,
    pub recovered: bool,
    pub error: Option<String>,
    pub after_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotResult {
    pub project_id: i64,
    pub tree: String,
    pub human_edits_before: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleResult {
    pub project_id: i64,
    pub tree: String,
    pub recovered: bool,
}

/// Names both generated refs for one turn worktree. Persist these before
/// writing any ref so startup recovery can distinguish Panoptes refs.
pub(crate) fn turn_refs(lane: &Lane, turn: u32, project_id: i64) -> (String, String) {
    let prefix = if lane.project_id == project_id {
        format!("refs/panoptes/lanes/{}/turns/{turn}", lane.name)
    } else {
        format!(
            "refs/panoptes/lanes/{}/turns/{turn}/projects/{project_id}",
            lane.name
        )
    };
    (format!("{prefix}/before"), format!("{prefix}/after"))
}

impl Store {
    /// Reserves the next lane-wide turn and all deterministic worktree ref
    /// names before any Git ref is written.
    pub(crate) fn create_turn(
        &self,
        lane_id: i64,
        session_id: i64,
        message_id: Option<&str>,
        prompt: &str,
        queued: bool,
    ) -> anyhow::Result<Turn> {
        let lane = self.lane(lane_id)?;
        if lane.worktrees.is_empty() {
            bail!("A turn needs at least one worktree");
        }
        let prompt = prompt.trim().lines().next().unwrap_or_default().trim();
        let prompt = if prompt.is_empty() { "Run" } else { prompt };
        let tx = self.write()?;
        let next: u32 = tx.query_row(
            "SELECT COALESCE(MAX(turn), 0) + 1 FROM turns WHERE lane_id = ?1",
            [lane_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO turns
             (lane_id, turn, session_id, message_id, prompt, queued, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                lane_id,
                next,
                session_id,
                message_id,
                prompt,
                queued,
                now_ms()
            ],
        )?;
        for worktree in &lane.worktrees {
            let (before_ref, after_ref) = turn_refs(&lane, next, worktree.project_id);
            tx.execute(
                "INSERT INTO turn_worktrees
                 (lane_id, turn, project_id, before_ref, after_ref, boundary)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'unavailable')",
                rusqlite::params![lane_id, next, worktree.project_id, before_ref, after_ref],
            )?;
        }
        tx.commit()?;
        self.turn(lane_id, next)
    }

    /// The last completed after ref/tree for this worktree, used both as the
    /// snapshot's parent and to detect human edits between turns.
    pub(crate) fn previous_after(
        &self,
        lane_id: i64,
        turn: u32,
        project_id: i64,
    ) -> anyhow::Result<Option<(String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT after_ref, after_tree FROM turn_worktrees
                 WHERE lane_id = ?1 AND project_id = ?2 AND turn < ?3
                   AND after_tree IS NOT NULL
                 ORDER BY turn DESC LIMIT 1",
                rusqlite::params![lane_id, project_id, turn],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub(crate) fn set_turn_before(
        &self,
        lane_id: i64,
        turn: u32,
        result: &SnapshotResult,
        boundary: Boundary,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let changed = self.conn.execute(
            "UPDATE turn_worktrees SET before_tree = ?4, boundary = ?5,
                human_edits_before = ?6, error = ?7
             WHERE lane_id = ?1 AND turn = ?2 AND project_id = ?3",
            rusqlite::params![
                lane_id,
                turn,
                result.project_id,
                result.tree,
                boundary.sql(),
                result.human_edits_before,
                error,
            ],
        )?;
        if changed == 0 {
            bail!("That turn worktree no longer exists");
        }
        Ok(())
    }

    pub(crate) fn fail_turn_before(
        &self,
        lane_id: i64,
        turn: u32,
        project_id: i64,
        error: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turn_worktrees SET before_tree = NULL,
                boundary = 'unavailable', error = ?4
             WHERE lane_id = ?1 AND turn = ?2 AND project_id = ?3",
            rusqlite::params![lane_id, turn, project_id, error],
        )?;
        Ok(())
    }

    pub(crate) fn mark_turn_boundary(
        &self,
        lane_id: i64,
        turn: u32,
        boundary: Boundary,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turn_worktrees SET boundary = ?3 WHERE lane_id = ?1 AND turn = ?2",
            rusqlite::params![lane_id, turn, boundary.sql()],
        )?;
        Ok(())
    }

    pub(crate) fn set_turn_queued(
        &self,
        lane_id: i64,
        turn: u32,
        queued: bool,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turns SET queued = ?3 WHERE lane_id = ?1 AND turn = ?2",
            rusqlite::params![lane_id, turn, queued],
        )?;
        Ok(())
    }

    pub(crate) fn set_turn_message_id(
        &self,
        lane_id: i64,
        turn: u32,
        message_id: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turns SET message_id = ?3
             WHERE lane_id = ?1 AND turn = ?2 AND message_id IS NULL",
            rusqlite::params![lane_id, turn, message_id],
        )?;
        Ok(())
    }

    /// Records one after snapshot. Calls are idempotent for an already
    /// completed row, so duplicate settled/exited events cannot replace the
    /// original boundary. Earlier before errors are retained.
    pub(crate) fn set_turn_after(
        &self,
        lane_id: i64,
        turn: u32,
        result: &SettleResult,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turn_worktrees SET after_tree = ?4, recovered = ?5,
                after_error = CASE
                    WHEN ?6 IS NULL THEN after_error
                    WHEN after_error IS NULL THEN ?6
                    ELSE after_error || '; ' || ?6 END
             WHERE lane_id = ?1 AND turn = ?2 AND project_id = ?3
               AND after_tree IS NULL",
            rusqlite::params![
                lane_id,
                turn,
                result.project_id,
                result.tree,
                result.recovered,
                error,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn fail_turn_after(
        &self,
        lane_id: i64,
        turn: u32,
        project_id: i64,
        error: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE turn_worktrees SET after_error = ?4
             WHERE lane_id = ?1 AND turn = ?2 AND project_id = ?3",
            rusqlite::params![lane_id, turn, project_id, error],
        )?;
        Ok(())
    }

    /// Finishes an unfinished turn once every worktree has either an after
    /// tree or an explicit after error.
    pub(crate) fn settle_turn(
        &self,
        lane_id: i64,
        turn: u32,
        outcome: TurnOutcome,
    ) -> anyhow::Result<()> {
        let tx = self.write()?;
        let incomplete: i64 = tx.query_row(
            "SELECT COUNT(*) FROM turn_worktrees
             WHERE lane_id = ?1 AND turn = ?2
               AND after_tree IS NULL AND after_error IS NULL",
            rusqlite::params![lane_id, turn],
            |row| row.get(0),
        )?;
        if incomplete > 0 {
            bail!("That turn still has {incomplete} unfinished worktrees");
        }
        tx.execute(
            "UPDATE turns SET outcome = ?3, settled_at = ?4
             WHERE lane_id = ?1 AND turn = ?2 AND outcome IS NULL",
            rusqlite::params![lane_id, turn, outcome.sql(), now_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn discard_turn(&self, lane_id: i64, turn: u32) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM turns WHERE lane_id = ?1 AND turn = ?2",
            rusqlite::params![lane_id, turn],
        )?;
        Ok(())
    }

    pub(crate) fn turn(&self, lane_id: i64, turn: u32) -> anyhow::Result<Turn> {
        let mut result = self
            .conn
            .query_row(
                "SELECT session_id, message_id, prompt, queued, started_at, settled_at, outcome
                 FROM turns WHERE lane_id = ?1 AND turn = ?2",
                rusqlite::params![lane_id, turn],
                |row| {
                    let outcome: Option<String> = row.get(6)?;
                    Ok(Turn {
                        lane_id,
                        turn,
                        session_id: row.get(0)?,
                        message_id: row.get(1)?,
                        prompt: row.get(2)?,
                        queued: row.get(3)?,
                        started_at: row.get(4)?,
                        settled_at: row.get(5)?,
                        outcome: outcome.as_deref().map(TurnOutcome::from_sql).transpose()?,
                        worktrees: Vec::new(),
                    })
                },
            )
            .optional()?
            .context("That turn no longer exists")?;
        result.worktrees = self.turn_worktrees(lane_id, turn)?;
        Ok(result)
    }

    pub(crate) fn unfinished_turns(&self) -> anyhow::Result<Vec<Turn>> {
        let ids = self
            .conn
            .prepare_cached("SELECT lane_id, turn FROM turns WHERE outcome IS NULL")?
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u32>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut turns = Vec::with_capacity(ids.len());
        for (lane_id, turn) in ids {
            turns.push(self.turn(lane_id, turn)?);
        }
        Ok(turns)
    }

    pub(crate) fn owned_turn_refs(&self) -> anyhow::Result<Vec<String>> {
        let refs = self
            .conn
            .prepare_cached(
                "SELECT before_ref FROM turn_worktrees
                 UNION SELECT after_ref FROM turn_worktrees",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(refs)
    }

    fn turn_worktrees(&self, lane_id: i64, turn: u32) -> anyhow::Result<Vec<TurnWorktree>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT project_id, before_ref, after_ref, before_tree, after_tree,
                    boundary, human_edits_before, recovered, error, after_error
             FROM turn_worktrees WHERE lane_id = ?1 AND turn = ?2 ORDER BY project_id",
        )?;
        let rows = statement
            .query_map(rusqlite::params![lane_id, turn], |row| {
                let boundary: String = row.get(5)?;
                Ok(TurnWorktree {
                    project_id: row.get(0)?,
                    before_ref: row.get(1)?,
                    after_ref: row.get(2)?,
                    before_tree: row.get(3)?,
                    after_tree: row.get(4)?,
                    boundary: Boundary::from_sql(&boundary, "boundary")?,
                    human_edits_before: row.get(6)?,
                    recovered: row.get(7)?,
                    error: row.get(8)?,
                    after_error: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
