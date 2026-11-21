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
    Migration {
        apply: v4_sessions,
        committed: |_| {},
    },
    Migration {
        apply: v5_profiles_and_model_preferences,
        committed: |_| {},
    },
    Migration {
        apply: v6_session_instructions,
        committed: |_| {},
    },
    Migration {
        apply: v7_lane_cleanup_preferences,
        committed: |_| {},
    },
    Migration {
        apply: v8_permission_warning_preference,
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
/// That is also why `path` is not UNIQUE: an archived main-checkout lane
/// keeps the repository's path. Only active lanes must not share a path,
/// which `Store` checks when a lane is added or restored.
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
            path TEXT NOT NULL,
            main_checkout INTEGER NOT NULL CHECK (main_checkout IN (0, 1)),
            checkpoint_ref TEXT CHECK (checkpoint_ref IS NULL OR checkpoint_ref <> ''),
            PRIMARY KEY (lane_id, project_id),
            UNIQUE (lane_id, position)
        ) STRICT;
        CREATE INDEX lanes_by_project ON lanes (project_id, state, created_at);
        CREATE INDEX lane_worktrees_by_path ON lane_worktrees (path);",
    )?;
    Ok(())
}

/// Panoptes sessions. Each belongs to one lane, runs in that lane's
/// worktree, and points at a harness conversation by `harness_id` and
/// `locator`, which stay NULL until the harness reports them on the first
/// start. `title` is Panoptes' own name for it; NULL shows the harness's.
fn v4_sessions(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lane_id INTEGER NOT NULL REFERENCES lanes (id) ON DELETE RESTRICT,
            harness TEXT NOT NULL CHECK (harness <> ''),
            harness_id TEXT CHECK (harness_id <> ''),
            locator TEXT CHECK (locator <> ''),
            title TEXT CHECK (title <> ''),
            pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
            archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
            created_at INTEGER NOT NULL,
            UNIQUE (harness, harness_id)
        ) STRICT;
        CREATE INDEX sessions_by_lane ON sessions (lane_id, created_at);
        CREATE TABLE session_tags (
            session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
            tag TEXT NOT NULL CHECK (tag <> ''),
            PRIMARY KEY (session_id, tag)
        ) STRICT;",
    )?;
    Ok(())
}

/// Panoptes' reusable session setup and model-picker presentation. None of
/// this rewrites harness configuration or its conversations.
fn v5_profiles_and_model_preferences(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE CHECK (name <> ''),
            model_id TEXT CHECK (model_id IS NULL OR model_id <> ''),
            thinking_level TEXT CHECK (thinking_level IS NULL OR thinking_level <> ''),
            instructions TEXT CHECK (instructions IS NULL OR instructions <> ''),
            position INTEGER NOT NULL UNIQUE,
            created_at INTEGER NOT NULL
        ) STRICT;
        CREATE TABLE model_preferences (
            model_id TEXT PRIMARY KEY NOT NULL CHECK (model_id <> ''),
            favorite_position INTEGER UNIQUE,
            hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1))
        ) STRICT;",
    )?;
    Ok(())
}

/// The instructions chosen at creation stay with the session when its
/// harness process is reopened. They do not follow later profile edits.
fn v6_session_instructions(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "ALTER TABLE sessions ADD COLUMN instructions TEXT
         CHECK (instructions IS NULL OR instructions <> '');",
    )?;
    Ok(())
}

const DEFAULT_CLEANUP_IDLE_DAYS: u32 = 4;
pub const MAX_CLEANUP_IDLE_DAYS: u32 = 365;

/// Lane cleanup is opt-in at the archive button. This setting only controls
/// which inactive worktrees the cleanup dialog offers.
fn v7_lane_cleanup_preferences(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(&format!(
        "CREATE TABLE lane_cleanup_preferences (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            idle_days INTEGER NOT NULL CHECK (idle_days BETWEEN 1 AND {MAX_CLEANUP_IDLE_DAYS})
        ) STRICT;
        INSERT INTO lane_cleanup_preferences (singleton, idle_days)
        VALUES (1, {DEFAULT_CLEANUP_IDLE_DAYS});"
    ))?;
    Ok(())
}

fn v8_permission_warning_preference(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE ui_preferences (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            permission_warning_dismissed INTEGER NOT NULL DEFAULT 0 CHECK (permission_warning_dismissed IN (0, 1))
        ) STRICT;
        INSERT INTO ui_preferences (singleton) VALUES (1);",
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
mod tests;
