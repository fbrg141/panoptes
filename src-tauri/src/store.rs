//! Panoptes' own data, in a SQLite database in the app data folder. Harnesses
//! keep their conversations; this holds what Panoptes adds on top (pins,
//! projects and lanes now, then groups, sessions and tags, see
//! `docs/domain-model.md`).
//!
//! The schema version is SQLite's `user_version`. Opening the store runs
//! every migration it is missing, each in its own transaction, after copying
//! the database aside. A damaged database, one written by a newer Panoptes,
//! or a migration that fails stops the open with an error: the store never
//! starts empty in place of data it could not read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};

const DATABASE: &str = "panoptes.db";
/// The JSON file that held pins before the store existed.
const LEGACY_INDEX: &str = "index.json";
/// What `index.json` is renamed to once its pins are in the database.
const IMPORTED_INDEX: &str = "index.json.migrated";

/// How long to wait for another Panoptes that holds the database.
const BUSY_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(200)
} else {
    Duration::from_secs(5)
};
const IN_USE: &str = "Another Panoptes is using it. Quit that one and try again";

/// One schema step.
#[derive(Clone, Copy)]
struct Migration {
    /// Changes the schema, inside the step's transaction. `dir` is the app
    /// data folder, for steps that import files from it.
    apply: fn(&Transaction, &Path) -> anyhow::Result<()>,
    /// Runs once the step has committed, for file changes that must not
    /// happen unless it did.
    committed: fn(&Path),
}

/// Schema version `n` is reached by running `MIGRATIONS[n - 1]`. Append new
/// steps; never edit or reorder one that has shipped.
const MIGRATIONS: &[Migration] = &[
    Migration {
        apply: v1_pins,
        committed: retire_legacy_index,
    },
    Migration {
        apply: v2_projects,
        committed: |_| {},
    },
    Migration {
        apply: v3_lanes,
        committed: |_| {},
    },
];

fn v1_pins(tx: &Transaction, dir: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE pins (
            session_key TEXT PRIMARY KEY NOT NULL
        ) STRICT;",
    )?;
    for key in legacy_pins(dir)? {
        tx.execute(
            "INSERT OR IGNORE INTO pins (session_key) VALUES (?1)",
            [key],
        )?;
    }
    Ok(())
}

/// Registered git repositories. `AUTOINCREMENT` so a removed project's id is
/// never given to another one: lanes and sessions will point at it.
/// The files to copy are their own rows, in order, rather than a JSON list,
/// so SQLite keeps them unique and drops them with their project.
fn v2_projects(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE projects (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL CHECK (name <> ''),
            repo TEXT NOT NULL UNIQUE,
            base_branch TEXT NOT NULL CHECK (base_branch <> ''),
            setup TEXT CHECK (setup <> ''),
            created_at INTEGER NOT NULL
        ) STRICT;
        CREATE TABLE project_copy_files (
            project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            path TEXT NOT NULL CHECK (path <> ''),
            PRIMARY KEY (project_id, position),
            UNIQUE (project_id, path)
        ) STRICT;",
    )?;
    Ok(())
}

/// Durable units of work. A lane can eventually have one worktree per
/// project in a multi-repo group; #20 deliberately creates only one.
/// Worktree rows survive archiving so a checkpoint can be restored later.
fn v3_lanes(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE lanes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
            name TEXT NOT NULL CHECK (name <> ''),
            task TEXT NOT NULL CHECK (task <> ''),
            branch TEXT NOT NULL CHECK (branch <> ''),
            state TEXT NOT NULL CHECK (state IN ('active', 'archived')),
            created_at INTEGER NOT NULL,
            UNIQUE (project_id, name)
        ) STRICT;
        CREATE TABLE lane_worktrees (
            lane_id INTEGER NOT NULL REFERENCES lanes (id) ON DELETE CASCADE,
            project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
            position INTEGER NOT NULL,
            path TEXT NOT NULL UNIQUE,
            main_checkout INTEGER NOT NULL CHECK (main_checkout IN (0, 1)),
            checkpoint_ref TEXT CHECK (checkpoint_ref IS NULL OR checkpoint_ref <> ''),
            PRIMARY KEY (lane_id, project_id),
            UNIQUE (lane_id, position)
        ) STRICT;
        CREATE INDEX lanes_by_project ON lanes (project_id, state, created_at);",
    )?;
    Ok(())
}

/// Pins from `index.json`, if there is one. An unreadable file fails the
/// migration rather than losing the pins.
fn legacy_pins(dir: &Path) -> anyhow::Result<BTreeSet<String>> {
    #[derive(Deserialize)]
    struct IndexFile {
        #[serde(default)]
        pinned: BTreeSet<String>,
    }
    let path = dir.join(LEGACY_INDEX);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", path.display()));
        }
    };
    let file: IndexFile = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} could not be read, so its pins were not imported. Move it away to start without them",
            path.display()
        )
    })?;
    Ok(file.pinned)
}

/// The pins are in the database now; keep the old file for reference under
/// a name that is never imported again.
fn retire_legacy_index(dir: &Path) {
    let legacy = dir.join(LEGACY_INDEX);
    match std::fs::rename(&legacy, dir.join(IMPORTED_INDEX)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("Could not rename {}: {error}", legacy.display()),
    }
}

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

/// The write lock belongs to the lane, not to any one process. Sessions and
/// its queue arrive in #22; until then there are no sessions that can hold it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteLock {
    pub holder: Option<String>,
    pub queue: Vec<String>,
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
    pub write_lock: WriteLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLane {
    pub project_id: i64,
    pub name: String,
    pub task: String,
    pub branch: String,
    pub worktrees: Vec<NewLaneWorktree>,
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
        let tx = self.conn.unchecked_transaction()?;
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
        let tx = self.conn.unchecked_transaction()?;
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
        let tx = self.conn.unchecked_transaction()?;
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
        let tx = self.conn.unchecked_transaction()?;
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

    pub fn restore_lane(&self, id: i64) -> anyhow::Result<Lane> {
        let changed = self.conn.execute(
            "UPDATE lanes SET state = 'active' WHERE id = ?1 AND state = 'archived'",
            [id],
        )?;
        if changed == 0 {
            bail!("That lane is not archived");
        }
        self.lane(id)
    }

    /// Forgets a project with no lanes. Its repository and worktrees on disk
    /// are not touched. A project with lanes stays registered so their records
    /// and recovery refs cannot silently lose the repository they refer to.
    pub fn remove_project(&self, id: i64) -> anyhow::Result<()> {
        let lanes: i64 = self.conn.query_row(
            "SELECT count(*) FROM lanes WHERE project_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if lanes > 0 {
            bail!("That project still has lanes and cannot be removed");
        }
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }
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
        write_lock: WriteLock::default(),
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

/// The database's schema version, after making sure it is a sound database
/// this build can read.
fn check(conn: &Connection) -> anyhow::Result<u32> {
    let status: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| explain(error, "The file is not a readable database"))?;
    if status != "ok" {
        bail!("The database is damaged: {status}");
    }
    user_version(conn)
}

fn user_version(conn: &Connection) -> anyhow::Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// `error` with `otherwise` as its reason, unless it only means another
/// Panoptes holds the database: that is not damage and must not read as it.
fn explain(error: rusqlite::Error, otherwise: &'static str) -> anyhow::Error {
    let in_use = matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    );
    anyhow::Error::new(error).context(if in_use { IN_USE } else { otherwise })
}

fn refuse_newer(version: u32, latest: u32) -> anyhow::Result<()> {
    if version > latest {
        bail!(
            "It was written by a newer version of Panoptes (schema {version}; this version reads up to {latest})"
        );
    }
    Ok(())
}

fn migrate(
    conn: &mut Connection,
    dir: &Path,
    version: u32,
    migrations: &[Migration],
) -> anyhow::Result<()> {
    let latest = migrations.len() as u32;
    refuse_newer(version, latest)?;
    if version == latest {
        return Ok(());
    }
    if version > 0 {
        back_up(conn, dir, version)?;
    }
    // Bundled SQLite enforces foreign keys by default, and a step that
    // rebuilds a table (create, copy, drop, rename) needs them off. The
    // pragma does nothing inside a transaction, so it is turned off here and
    // each step is checked before it commits instead.
    conn.pragma_update(None, "foreign_keys", false)?;
    loop {
        // IMMEDIATE takes the write lock up front and the version is read
        // again under it, so a second Panoptes opening the same store waits,
        // then skips the steps the first one ran.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| explain(error, "Could not start a migration"))?;
        let current = user_version(&tx)?;
        refuse_newer(current, latest)?;
        if current == latest {
            return Ok(());
        }
        if current == 0 && dir.join(IMPORTED_INDEX).exists() {
            bail!(
                "The database is empty, but Panoptes data was stored in it before ({IMPORTED_INDEX} is next to it). Rename {IMPORTED_INDEX} back to {LEGACY_INDEX} to import those pins again, or move it away to start without them"
            );
        }
        let target = current + 1;
        let step = migrations[current as usize];
        (step.apply)(&tx, dir)
            .and_then(|()| check_foreign_keys(&tx))
            .with_context(|| format!("Migration to schema {target} failed"))?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
        (step.committed)(dir);
    }
}

/// Fails if a step left a row pointing at one that does not exist, which it
/// could while foreign keys were off.
fn check_foreign_keys(tx: &Transaction) -> anyhow::Result<()> {
    let mut statement = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let parent: String = row.get(2)?;
        bail!("It left a row in {table} pointing to a missing row in {parent}");
    }
    Ok(())
}

/// Copies the database to `panoptes.db.v<version>.bak` before migrating away
/// from `version`. The copy is written under a temporary name and only then
/// replaces an older backup of the same version, so a failed copy never
/// costs the previous one. If another Panoptes migrated the store in the
/// meantime, the copy no longer holds `version` and is dropped: that one
/// took its own backup first.
fn back_up(conn: &Connection, dir: &Path, version: u32) -> anyhow::Result<()> {
    let backup = dir.join(format!("{DATABASE}.v{version}.bak"));
    let partial = partial_backup(dir);
    let result = copy_holds(conn, &partial, version).and_then(|holds| {
        if holds {
            std::fs::rename(&partial, &backup)?;
        }
        Ok(())
    });
    let _ = std::fs::remove_file(&partial);
    result.with_context(|| format!("Could not back up to {}", backup.display()))
}

/// Per process, so two copies being written at once don't collide.
fn partial_backup(dir: &Path) -> PathBuf {
    dir.join(format!("{DATABASE}.{}.partial", std::process::id()))
}

/// Writes a copy of the database to `path` and says whether it holds
/// schema `version`.
fn copy_holds(conn: &Connection, path: &Path, version: u32) -> anyhow::Result<bool> {
    // VACUUM INTO refuses to write over an existing file.
    let _ = std::fs::remove_file(path);
    conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
    let copy = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(user_version(&copy)? == version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(dir: &Path) -> u32 {
        let conn = Connection::open(dir.join(DATABASE)).unwrap();
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn step(apply: fn(&Transaction, &Path) -> anyhow::Result<()>) -> Migration {
        Migration {
            apply,
            committed: |_| {},
        }
    }

    fn broken(_: &Transaction, _: &Path) -> anyhow::Result<()> {
        bail!("no room")
    }

    fn v2_notes(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
        tx.execute_batch("CREATE TABLE notes (body TEXT NOT NULL) STRICT;")?;
        Ok(())
    }

    fn v2_parents(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
        tx.execute_batch(
            "CREATE TABLE parents (id INTEGER PRIMARY KEY) STRICT;
            CREATE TABLE children (
                parent INTEGER NOT NULL REFERENCES parents (id) ON DELETE CASCADE
            ) STRICT;
            INSERT INTO parents (id) VALUES (1);
            INSERT INTO children (parent) VALUES (1);",
        )?;
        Ok(())
    }

    /// SQLite's way to change a table: build a new one, copy, drop, rename.
    fn v3_rebuild_parents(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
        tx.execute_batch(
            "CREATE TABLE parents_new (id INTEGER PRIMARY KEY, name TEXT) STRICT;
            INSERT INTO parents_new (id) SELECT id FROM parents;
            DROP TABLE parents;
            ALTER TABLE parents_new RENAME TO parents;",
        )?;
        Ok(())
    }

    fn v3_orphan_child(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
        tx.execute_batch("INSERT INTO children (parent) VALUES (9);")?;
        Ok(())
    }

    /// The tests of the migration machinery build on a store at schema 1,
    /// so they keep their meaning as real steps are added.
    fn v1() -> Migration {
        MIGRATIONS[0]
    }

    fn settings(name: &str, copy_files: &[&str]) -> ProjectSettings {
        ProjectSettings {
            name: name.into(),
            base_branch: "main".into(),
            copy_files: copy_files.iter().map(|f| f.to_string()).collect(),
            setup: None,
        }
    }

    fn count(store: &Store, table: &str) -> u32 {
        store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn a_new_store_starts_at_the_latest_schema() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(store.pinned().unwrap().is_empty());
        assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
    }

    #[test]
    fn pins_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.set_pinned("pi:a", true).unwrap();
        store.set_pinned("pi:b", true).unwrap();
        store.set_pinned("pi:b", true).unwrap();
        store.set_pinned("pi:a", false).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
    }

    #[test]
    fn pins_in_index_json_are_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_INDEX);
        std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:b"]}"#).unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.pinned().unwrap(),
            BTreeSet::from(["pi:a".into(), "pi:b".into()])
        );
        assert!(!legacy.exists());
        assert!(dir.path().join("index.json.migrated").exists());

        // Unpinning sticks, and a stray index.json is not imported again.
        store.set_pinned("pi:a", false).unwrap();
        drop(store);
        std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:c"]}"#).unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
    }

    #[test]
    fn an_unreadable_index_json_stops_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_INDEX);
        std::fs::write(&legacy, "{not json").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("Migration to schema 1 failed"), "{error}");
        assert!(error.contains("its pins were not imported"), "{error}");
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "{not json");
        assert_eq!(version(dir.path()), 0);
    }

    #[test]
    fn a_failed_migration_stops_and_keeps_the_data() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1()])
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();

        let error = Store::open_with(dir.path(), &[v1(), step(broken)])
            .err()
            .unwrap();
        let error = format!("{error:#}");
        assert!(
            error.contains("Migration to schema 2 failed: no room"),
            "{error}"
        );
        assert_eq!(version(dir.path()), 1);

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
    }

    #[test]
    fn migrating_backs_up_the_database_first() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1()])
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();

        let store = Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();
        assert_eq!(version(dir.path()), 2);
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));

        let backup = Connection::open(dir.path().join("panoptes.db.v1.bak")).unwrap();
        let backed_up: Vec<String> = backup
            .prepare("SELECT session_key FROM pins")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(backed_up, ["pi:a"]);
        let backup_version: u32 = backup
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_version, 1);
        assert!(!partial_backup(dir.path()).exists());
    }

    #[test]
    fn a_store_from_a_newer_panoptes_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();
        let error = format!("{:#}", Store::open_with(dir.path(), &[v1()]).err().unwrap());
        assert!(error.contains("newer version of Panoptes"), "{error}");
        assert_eq!(version(dir.path()), 2);
    }

    #[test]
    fn a_damaged_database_is_refused_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DATABASE);
        std::fs::write(&path, b"this is not a database at all, not even close").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("not a readable database"), "{error}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"this is not a database at all, not even close"
        );
    }

    #[test]
    fn steps_another_panoptes_already_ran_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        // Both read version 0; the other one migrates first.
        let mut late = Connection::open(dir.path().join(DATABASE)).unwrap();
        Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();

        migrate(&mut late, dir.path(), 0, &[v1(), step(v2_notes)]).unwrap();
        assert_eq!(version(dir.path()), 2);
    }

    #[test]
    fn a_backup_is_kept_only_if_it_holds_the_version_it_is_named_after() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1()])
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();
        // Both read version 1; the other one backs up and migrates first.
        let mut late = Connection::open(dir.path().join(DATABASE)).unwrap();
        Store::open_with(dir.path(), &[v1(), step(v2_notes)]).unwrap();

        migrate(&mut late, dir.path(), 1, &[v1(), step(v2_notes)]).unwrap();
        let backup = Connection::open(dir.path().join("panoptes.db.v1.bak")).unwrap();
        assert_eq!(user_version(&backup).unwrap(), 1);
        assert!(!partial_backup(dir.path()).exists());
    }

    #[test]
    fn a_failed_backup_keeps_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1()]).unwrap();
        let backup = dir.path().join("panoptes.db.v1.bak");
        std::fs::write(&backup, "an earlier backup").unwrap();
        // A folder in the way makes the copy fail.
        std::fs::create_dir(partial_backup(dir.path())).unwrap();

        let error = Store::open_with(dir.path(), &[v1(), step(v2_notes)])
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains("Could not back up"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "an earlier backup"
        );
        assert_eq!(version(dir.path()), 1);
    }

    #[test]
    fn steps_can_rebuild_a_table_without_losing_the_rows_that_point_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let migrations = [v1(), step(v2_parents), step(v3_rebuild_parents)];
        let store = Store::open_with(dir.path(), &migrations).unwrap();

        let children: u32 = store
            .conn
            .query_row("SELECT count(*) FROM children", [], |row| row.get(0))
            .unwrap();
        assert_eq!(children, 1);
        let enforced: bool = store
            .conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert!(enforced);
    }

    #[test]
    fn a_step_that_leaves_a_broken_reference_fails() {
        let dir = tempfile::tempdir().unwrap();
        let migrations = [v1(), step(v2_parents), step(v3_orphan_child)];
        let error = format!(
            "{:#}",
            Store::open_with(dir.path(), &migrations).err().unwrap()
        );
        assert!(
            error.contains("Migration to schema 3 failed: It left a row in children"),
            "{error}"
        );
        assert_eq!(version(dir.path()), 2);
    }

    #[test]
    fn index_json_is_retired_once_its_pins_are_committed() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_INDEX);
        std::fs::write(&legacy, r#"{"pinned":["pi:a"]}"#).unwrap();

        // A later step failing does not undo the import, so the file goes.
        Store::open_with(dir.path(), &[v1(), step(broken)])
            .err()
            .unwrap();
        assert_eq!(version(dir.path()), 1);
        assert!(!legacy.exists());
        assert!(dir.path().join(IMPORTED_INDEX).exists());
    }

    #[test]
    fn an_emptied_database_is_refused_once_pins_were_imported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LEGACY_INDEX), r#"{"pinned":["pi:a"]}"#).unwrap();
        drop(Store::open(dir.path()).unwrap());
        std::fs::write(dir.path().join(DATABASE), "").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("The database is empty"), "{error}");
        assert_eq!(version(dir.path()), 0);
    }

    #[test]
    fn a_store_held_by_another_panoptes_is_reported_as_in_use() {
        let dir = tempfile::tempdir().unwrap();
        drop(Store::open(dir.path()).unwrap());
        let other = Connection::open(dir.path().join(DATABASE)).unwrap();
        other.execute_batch("BEGIN EXCLUSIVE;").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains(IN_USE), "{error}");
        assert!(!error.contains("not a readable database"), "{error}");
    }

    #[test]
    fn a_v1_store_migrates_to_projects_and_lanes_and_keeps_its_pins() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1()])
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
        assert!(store.projects().unwrap().is_empty());
        assert!(store.lanes().unwrap().is_empty());
        assert!(dir.path().join("panoptes.db.v1.bak").exists());
    }

    #[test]
    fn projects_survive_reopening_with_their_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut web = settings("web", &[".env", "config/local.toml"]);
        web.setup = Some("pnpm install".into());
        let added = store.add_project("/code/web", &web).unwrap();
        store
            .add_project("/code/api", &settings("api", &[]))
            .unwrap();
        drop(store);

        let store = Store::open(dir.path()).unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(
            projects
                .iter()
                .map(|p| p.settings.name.as_str())
                .collect::<Vec<_>>(),
            ["api", "web"]
        );
        assert_eq!(projects[1], added);
        assert_eq!(projects[1].repo, "/code/web");
        assert_eq!(
            projects[1].settings.copy_files,
            [".env", "config/local.toml"]
        );
        assert_eq!(projects[1].settings.setup.as_deref(), Some("pnpm install"));
        assert!(projects[0].settings.copy_files.is_empty());
    }

    #[test]
    fn a_repository_is_registered_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .add_project("/code/web", &settings("web", &[]))
            .unwrap();
        let error = store
            .add_project("/code/web", &settings("again", &[]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("already registered as web"), "{error}");
        assert_eq!(store.projects().unwrap().len(), 1);
    }

    #[test]
    fn updating_a_project_replaces_its_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let id = store
            .add_project("/code/web", &settings("web", &[".env", "a"]))
            .unwrap()
            .id;
        let mut changed = settings("Web app", &["b", ".env"]);
        changed.base_branch = "develop".into();
        changed.setup = Some("make".into());
        let updated = store.update_project(id, &changed).unwrap();
        assert_eq!(updated.repo, "/code/web");
        assert_eq!(store.projects().unwrap(), [updated]);

        store.remove_project(id).unwrap();
        let error = store.update_project(id, &changed).unwrap_err().to_string();
        assert!(error.contains("no longer registered"), "{error}");
    }

    #[test]
    fn removing_a_project_drops_its_files_and_never_reuses_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let first = store
            .add_project("/code/web", &settings("web", &[".env"]))
            .unwrap();
        store.remove_project(first.id).unwrap();
        assert!(store.projects().unwrap().is_empty());
        assert_eq!(count(&store, "project_copy_files"), 0);

        let second = store
            .add_project("/code/web", &settings("web", &[".env"]))
            .unwrap();
        assert!(second.id > first.id);
    }

    #[test]
    fn lanes_and_multiple_worktrees_survive_reopening_and_archiving() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let project = store
            .add_project("/code/web", &settings("web", &[]))
            .unwrap();
        let api = store
            .add_project("/code/api", &settings("api", &[]))
            .unwrap();
        let new = NewLane {
            project_id: project.id,
            name: "fix-login".into(),
            task: "Fix login".into(),
            branch: "panoptes/fix-login".into(),
            worktrees: vec![
                NewLaneWorktree {
                    project_id: project.id,
                    path: "/lanes/fix-login/web".into(),
                    main_checkout: false,
                },
                NewLaneWorktree {
                    project_id: api.id,
                    path: "/lanes/fix-login/api".into(),
                    main_checkout: true,
                },
            ],
        };
        let added = store.add_lane(&new).unwrap();
        assert_eq!(added.state, LaneState::Active);
        assert_eq!(added.worktrees.len(), 2);
        assert_eq!(added.write_lock.holder, None);
        assert!(added.write_lock.queue.is_empty());
        drop(store);

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.lanes().unwrap(), std::slice::from_ref(&added));
        let archived = store
            .archive_lane(added.id, Some("refs/panoptes/lanes/1/archive"))
            .unwrap();
        assert_eq!(archived.state, LaneState::Archived);
        assert_eq!(
            archived.worktrees[0].checkpoint_ref.as_deref(),
            Some("refs/panoptes/lanes/1/archive")
        );
        assert_eq!(
            store.restore_lane(added.id).unwrap().state,
            LaneState::Active
        );

        let error = store.remove_project(project.id).unwrap_err().to_string();
        assert!(error.contains("still has lanes"), "{error}");
    }

    #[test]
    fn lane_names_and_worktree_paths_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let project = store
            .add_project("/code/web", &settings("web", &[]))
            .unwrap();
        let new = NewLane {
            project_id: project.id,
            name: "fix-login".into(),
            task: "Fix login".into(),
            branch: "panoptes/fix-login".into(),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: "/lanes/fix-login/web".into(),
                main_checkout: false,
            }],
        };
        store.add_lane(&new).unwrap();
        assert!(store.add_lane(&new).is_err());
        let mut other = new.clone();
        other.name = "other".into();
        other.branch = "panoptes/other".into();
        assert!(store.add_lane(&other).is_err());
    }

    #[test]
    fn the_schema_refuses_rows_that_break_its_rules() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let bad = [
            "INSERT INTO projects (name, repo, base_branch, created_at) VALUES ('', '/a', 'main', 0)",
            "INSERT INTO projects (name, repo, base_branch, setup, created_at) VALUES ('a', '/a', 'main', '', 0)",
            "INSERT INTO project_copy_files (project_id, position, path) VALUES (99, 0, '.env')",
            "INSERT INTO lanes (project_id, name, task, branch, state, created_at) VALUES (99, 'a', 'A', 'b', 'active', 0)",
            "INSERT INTO lanes (project_id, name, task, branch, state, created_at) VALUES (1, '', 'A', 'b', 'active', 0)",
        ];
        for sql in bad {
            assert!(store.conn.execute(sql, []).is_err(), "{sql}");
        }
    }
}
