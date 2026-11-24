use std::path::Path;

use rusqlite::Transaction;

use super::legacy::{legacy_pins, retire_legacy_index};
use super::preferences::{
    DEFAULT_CLEANUP_IDLE_DAYS, DEFAULT_FONT_SIZE, MAX_CLEANUP_IDLE_DAYS, MAX_FONT_SIZE,
    MIN_FONT_SIZE,
};

/// One schema step.
#[derive(Clone, Copy)]
pub(super) struct Migration {
    /// Changes the schema, inside the step's transaction. `dir` is the app
    /// data folder, for steps that import files from it.
    pub(super) apply: fn(&Transaction, &Path) -> anyhow::Result<()>,
    /// Runs once the step has committed, for file changes that must not
    /// happen unless it did.
    pub(super) committed: fn(&Path),
}

/// Schema version `n` is reached by running `MIGRATIONS[n - 1]`. Append new
/// steps; never edit or reorder one that has shipped.
pub(super) const MIGRATIONS: &[Migration] = &[
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
    Migration {
        apply: v9_allowed_actions,
        committed: |_| {},
    },
    Migration {
        apply: v10_notification_preferences,
        committed: |_| {},
    },
    Migration {
        apply: v11_appearance_preferences,
        committed: |_| {},
    },
    Migration {
        apply: v12_notification_sound,
        committed: |_| {},
    },
    Migration {
        apply: v13_turn_checkpoints,
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

/// What the user allowed, so the same action is not asked about again. The
/// scope is the lane a session works in, or the session itself when it has
/// no lane; the subject names what the allow covers.
fn v9_allowed_actions(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE allowed_actions (
            scope TEXT NOT NULL,
            subject TEXT NOT NULL,
            PRIMARY KEY (scope, subject)
        ) STRICT;",
    )?;
    Ok(())
}

/// Whether Panoptes posts a system notification when a run finishes or a
/// session needs the user.
fn v10_notification_preferences(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "ALTER TABLE ui_preferences
            ADD COLUMN notify_enabled INTEGER NOT NULL DEFAULT 1 CHECK (notify_enabled IN (0, 1));
         ALTER TABLE ui_preferences
            ADD COLUMN notify_finished INTEGER NOT NULL DEFAULT 1 CHECK (notify_finished IN (0, 1));
         ALTER TABLE ui_preferences
            ADD COLUMN notify_needs_attention INTEGER NOT NULL DEFAULT 1
                CHECK (notify_needs_attention IN (0, 1));",
    )?;
    Ok(())
}

/// Appearance stays system-controlled until the user chooses a dark mode.
/// Font size is the percentage used by the window's existing zoom control.
fn v11_appearance_preferences(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(&format!(
        "ALTER TABLE ui_preferences ADD COLUMN dark_mode INTEGER
         CHECK (dark_mode IS NULL OR dark_mode IN (0, 1));
         ALTER TABLE ui_preferences ADD COLUMN font_size INTEGER NOT NULL
         DEFAULT {DEFAULT_FONT_SIZE}
         CHECK (font_size BETWEEN {MIN_FONT_SIZE} AND {MAX_FONT_SIZE} AND font_size % 10 = 0);"
    ))?;
    Ok(())
}

/// Notifications can also make a sound.
fn v12_notification_sound(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "ALTER TABLE ui_preferences
            ADD COLUMN notify_sound INTEGER NOT NULL DEFAULT 1 CHECK (notify_sound IN (0, 1));",
    )?;
    Ok(())
}

/// Durable, lane-level turn boundaries. Worktree trees and refs are separate
/// rows because a future multi-repo lane cannot claim one atomic snapshot.
/// Human edits between turns are inferred from trees, not copied from events.
fn v13_turn_checkpoints(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE turns (
            lane_id INTEGER NOT NULL REFERENCES lanes (id) ON DELETE CASCADE,
            turn INTEGER NOT NULL,
            session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
            message_id TEXT CHECK (message_id IS NULL OR message_id <> ''),
            prompt TEXT NOT NULL CHECK (prompt <> ''),
            queued INTEGER NOT NULL DEFAULT 0 CHECK (queued IN (0, 1)),
            started_at INTEGER NOT NULL,
            settled_at INTEGER,
            outcome TEXT CHECK (outcome IS NULL OR outcome IN ('done', 'failed', 'stopped')),
            PRIMARY KEY (lane_id, turn)
        ) STRICT;
        CREATE TABLE turn_worktrees (
            lane_id INTEGER NOT NULL,
            turn INTEGER NOT NULL,
            project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE RESTRICT,
            before_ref TEXT NOT NULL CHECK (before_ref <> ''),
            after_ref TEXT NOT NULL CHECK (after_ref <> ''),
            before_tree TEXT CHECK (before_tree IS NULL OR before_tree <> ''),
            after_tree TEXT CHECK (after_tree IS NULL OR after_tree <> ''),
            boundary TEXT NOT NULL CHECK (boundary IN ('unavailable', 'observed', 'exact')),
            human_edits_before INTEGER NOT NULL DEFAULT 0 CHECK (human_edits_before IN (0, 1)),
            recovered INTEGER NOT NULL DEFAULT 0 CHECK (recovered IN (0, 1)),
            error TEXT CHECK (error IS NULL OR error <> ''),
            after_error TEXT CHECK (after_error IS NULL OR after_error <> ''),
            PRIMARY KEY (lane_id, turn, project_id),
            FOREIGN KEY (lane_id, turn) REFERENCES turns (lane_id, turn) ON DELETE CASCADE
        ) STRICT;
        CREATE INDEX turns_by_lane ON turns (lane_id, turn);",
    )?;
    Ok(())
}
