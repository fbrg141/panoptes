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

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use rusqlite::{Connection, Transaction, TransactionBehavior};

mod lanes;
mod legacy;
mod migrate;
mod migrations;
mod preferences;
mod projects;
mod sessions;

pub use lanes::{Lane, LaneState, LaneWorktree, NewLane, NewLaneWorktree};
pub use preferences::{AgentProfile, Preferences, ProfileInput};
pub use projects::{Project, ProjectSettings};

use migrate::{check, explain, migrate};
use migrations::{MIGRATIONS, Migration};

const DATABASE: &str = "panoptes.db";
/// How long to wait for another Panoptes that holds the database.
const BUSY_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(5)
};

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
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
