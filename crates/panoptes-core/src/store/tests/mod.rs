use super::legacy::{IMPORTED_INDEX, LEGACY_INDEX};
use super::migrate::{IN_USE, partial_backup, user_version};
use super::*;
use anyhow::bail;
use rusqlite::ErrorCode;
use std::collections::BTreeSet;

mod lanes;
mod migrations;
mod preferences;
mod projects;
mod sessions;
mod turns;

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

/// A second Panoptes on the same store waits for a whole check-and-write
/// rather than slipping in between the check and the write.
#[test]
fn a_store_change_takes_the_write_lock_before_it_checks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let other = Connection::open(dir.path().join(DATABASE)).unwrap();
    other.busy_timeout(Duration::ZERO).unwrap();
    let tx = store.write().unwrap();
    let error = other
        .execute("INSERT INTO pins (session_key) VALUES ('pi:b')", [])
        .unwrap_err();
    assert_eq!(error.sqlite_error_code(), Some(ErrorCode::DatabaseBusy));
    drop(tx);
    other
        .execute("INSERT INTO pins (session_key) VALUES ('pi:b')", [])
        .unwrap();
}

fn lane_in(store: &Store) -> Lane {
    let project = store
        .add_project("/code/web", &settings("web", &[]))
        .unwrap();
    store
        .add_lane(&NewLane {
            project_id: project.id,
            name: "fix-login".into(),
            task: "Fix login".into(),
            branch: "panoptes/fix-login".into(),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: "/lanes/fix-login/web".into(),
                main_checkout: false,
            }],
        })
        .unwrap()
}

fn another_lane_in(store: &Store, lane: &Lane) -> Lane {
    store
        .add_lane(&NewLane {
            project_id: lane.project_id,
            name: "another-lane".into(),
            task: "Another lane".into(),
            branch: "panoptes/another-lane".into(),
            worktrees: vec![NewLaneWorktree {
                project_id: lane.project_id,
                path: "/lanes/another-lane/web".into(),
                main_checkout: false,
            }],
        })
        .unwrap()
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
        "INSERT INTO turns (lane_id, turn, session_id, prompt, queued, started_at) VALUES (1, 1, 1, '', 0, 0)",
        "INSERT INTO turns (lane_id, turn, session_id, prompt, queued, started_at, outcome) VALUES (1, 1, 1, 'P', 0, 0, 'maybe')",
        "INSERT INTO turn_worktrees (lane_id, turn, project_id, before_ref, after_ref, boundary) VALUES (1, 1, 1, 'b', 'a', 'guess')",
    ];
    for sql in bad {
        assert!(store.conn.execute(sql, []).is_err(), "{sql}");
    }
}
