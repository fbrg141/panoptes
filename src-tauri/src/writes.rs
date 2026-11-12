//! Which lanes a session changes besides its own. A lane has one session,
//! but nothing stops that session's agent from editing another lane's
//! worktree. This is read from recorded facts, never guessed from a folder:
//! a finished edit tool call that names a file in the worktree.

use std::path::{Component, Path, PathBuf};

use panoptes_harness::{ToolCall, ToolKind, ToolOutput, ToolStatus};

use crate::store::{Lane, LaneState};

/// The files a finished edit call changed, as the harness named them.
pub fn edited_paths(call: &ToolCall) -> Vec<String> {
    if call.kind != ToolKind::Edit || call.status != ToolStatus::Done {
        return Vec::new();
    }
    match &call.output {
        Some(ToolOutput::Edits { edits, .. }) if !edits.is_empty() => {
            edits.iter().map(|edit| edit.path.clone()).collect()
        }
        _ => call.subject.iter().cloned().collect(),
    }
}

/// The active lanes other than `own` whose worktree holds one of `paths`.
/// Relative paths are read from `cwd`, where the session runs. A path is
/// credited to the most specific worktree that holds it, so a session's own
/// worktree never counts as another lane's.
pub fn lanes_touched(paths: &[String], cwd: &Path, own: Option<i64>, lanes: &[Lane]) -> Vec<i64> {
    let roots: Vec<(i64, PathBuf)> = lanes
        .iter()
        .filter(|lane| lane.state == LaneState::Active)
        .flat_map(|lane| {
            lane.worktrees
                .iter()
                .map(move |tree| (lane.id, resolve(Path::new(&tree.path), Path::new("/"))))
        })
        .collect();
    let mut touched = Vec::new();
    for path in paths {
        let path = resolve(Path::new(path), cwd);
        let holder = roots
            .iter()
            .filter(|(_, root)| inside(&path, root))
            .max_by_key(|(_, root)| root.components().count());
        if let Some(&(lane, _)) = holder
            && Some(lane) != own
            && !touched.contains(&lane)
        {
            touched.push(lane);
        }
    }
    touched
}

/// An absolute, lexically normalised path: `~` is the home folder, a
/// relative path is read from `cwd`, and `.` and `..` are folded away.
fn resolve(path: &Path, cwd: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~") {
        Ok(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| path.to_path_buf()),
        Err(_) => path.to_path_buf(),
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

/// Whether `path` is `root` or inside it, also once symlinks are resolved
/// (macOS reports `/var` folders as `/private/var`).
fn inside(path: &Path, root: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    let Ok(root) = std::fs::canonicalize(root) else {
        return false;
    };
    // The file itself may be gone (a delete) or not yet exist; its nearest
    // existing folder decides.
    path.ancestors()
        .find_map(|ancestor| std::fs::canonicalize(ancestor).ok())
        .is_some_and(|real| real.starts_with(&root) || path.starts_with(&root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::LaneWorktree;
    use panoptes_harness::FileEdit;
    use serde_json::Value;

    fn lane(id: i64, path: &str, state: LaneState) -> Lane {
        Lane {
            id,
            project_id: 1,
            name: format!("lane-{id}"),
            task: format!("Task {id}"),
            branch: format!("panoptes/lane-{id}"),
            state,
            worktrees: vec![LaneWorktree {
                project_id: 1,
                path: path.into(),
                main_checkout: false,
                checkpoint_ref: None,
            }],
        }
    }

    fn call(kind: ToolKind, status: ToolStatus, subject: Option<&str>) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: "edit".into(),
            kind,
            subject: subject.map(String::from),
            input: Value::Null,
            status,
            output: None,
            started_at: None,
            ended_at: None,
        }
    }

    #[test]
    fn only_finished_edits_name_files() {
        let mut edit = call(ToolKind::Edit, ToolStatus::Done, Some("a.rs"));
        assert_eq!(edited_paths(&edit), ["a.rs"]);
        edit.output = Some(ToolOutput::Edits {
            edits: vec![FileEdit {
                path: "/x/b.rs".into(),
                old_text: None,
                new_text: String::new(),
            }],
            text: None,
        });
        assert_eq!(edited_paths(&edit), ["/x/b.rs"]);
        assert!(edited_paths(&call(ToolKind::Edit, ToolStatus::Running, Some("a.rs"))).is_empty());
        assert!(edited_paths(&call(ToolKind::Edit, ToolStatus::Failed, Some("a.rs"))).is_empty());
        assert!(edited_paths(&call(ToolKind::Read, ToolStatus::Done, Some("a.rs"))).is_empty());
    }

    #[test]
    fn edits_count_against_other_active_lanes_only() {
        let lanes = [
            lane(1, "/lanes/a/app", LaneState::Active),
            lane(2, "/lanes/b/app", LaneState::Active),
            lane(3, "/lanes/c/app", LaneState::Archived),
        ];
        let cwd = Path::new("/lanes/a/app");
        let paths = [
            "src/own.rs".to_string(),
            "../../b/app/src/new.rs".to_string(),
            "/lanes/b/app/README.md".to_string(),
            "/lanes/c/app/old.rs".to_string(),
            "/elsewhere/file.rs".to_string(),
        ];
        assert_eq!(lanes_touched(&paths, cwd, Some(1), &lanes), [2]);
        // A conversation no lane has adopted writes to every lane it touches.
        assert_eq!(lanes_touched(&paths, cwd, None, &lanes), [1, 2]);
    }

    #[test]
    fn the_most_specific_worktree_holds_a_path() {
        // A main-checkout lane at the repository, and a worktree inside it.
        let lanes = [
            lane(1, "/code/app", LaneState::Active),
            lane(2, "/code/app/.worktrees/b", LaneState::Active),
        ];
        let paths = ["/code/app/.worktrees/b/src/x.rs".to_string()];
        assert_eq!(
            lanes_touched(&paths, Path::new("/"), Some(2), &lanes),
            Vec::<i64>::new()
        );
        assert_eq!(lanes_touched(&paths, Path::new("/"), Some(1), &lanes), [2]);
    }

    #[test]
    fn a_sibling_folder_with_a_shared_prefix_is_not_inside() {
        let lanes = [lane(2, "/lanes/b", LaneState::Active)];
        let paths = ["/lanes/b-other/x.rs".to_string()];
        assert!(lanes_touched(&paths, Path::new("/"), None, &lanes).is_empty());
    }
}
