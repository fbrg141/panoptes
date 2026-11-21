use anyhow::{Context, bail};
use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;

use super::{Store, now_ms};

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

impl Store {
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
