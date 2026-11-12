//! Which lanes a session changes besides its own. A lane has one session,
//! but nothing stops that session's agent from editing another lane's
//! worktree. These are recorded facts, never a guess from a folder: an edit
//! tool that names a file in the worktree, or a worktree that changed during
//! a run that ran commands while its own session was idle.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use panoptes_harness::{ToolCall, ToolKind, ToolOutput, ToolStatus};

use crate::lanes::LaneChanges;
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

/// The lanes whose fingerprint differs between the start and end of a run.
/// A lane missing from either side is left out: it was not comparable.
pub fn changed_lanes(before: &HashMap<i64, String>, after: &HashMap<i64, String>) -> Vec<i64> {
    let mut changed: Vec<i64> = after
        .iter()
        .filter(|(lane, print)| before.get(lane).is_some_and(|earlier| earlier != *print))
        .map(|(&lane, _)| lane)
        .collect();
    changed.sort_unstable();
    changed
}

/// A conversation that changed a lane before it had a session of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefSource {
    pub title: String,
    /// The task of the lane that conversation belongs to, if any.
    pub lane: Option<String>,
    /// Where the harness keeps the conversation, for the new agent to read.
    pub conversation: Option<String>,
}

/// What a handoff brief is written from.
pub struct Brief<'a> {
    pub task: &'a str,
    pub branch: &'a str,
    pub base: &'a str,
    pub worktree: &'a str,
    pub changes: &'a LaneChanges,
    pub sources: &'a [BriefSource],
}

/// Lists past this many lines are cut, with a count of the rest.
const BRIEF_LIST_LIMIT: usize = 40;

/// The first prompt for a lane's own session when other conversations
/// started its work. Only recorded facts go in: the lane, what git says it
/// holds, and where the earlier conversations are. Nothing is summarised on
/// an agent's word.
pub fn handoff_brief(brief: &Brief) -> String {
    let mut text = String::new();
    let mut line = |s: &str| {
        text.push_str(s);
        text.push('\n');
    };
    line(
        "Continue the work in this lane. It was started from another conversation; from now on this conversation is the only one that changes this worktree.",
    );
    line("");
    line(&format!("Task: {}", brief.task));
    line(&format!("Branch: {} (from {})", brief.branch, brief.base));
    line(&format!("Worktree: {}", brief.worktree));
    line("");
    let files = &brief.changes.files;
    let commits = &brief.changes.commits;
    if files.is_empty() && commits.is_empty() {
        line(&format!(
            "Nothing is changed or committed yet compared with {}.",
            brief.base
        ));
    }
    if !files.is_empty() {
        line(&format!("Uncommitted changes ({}):", files.len()));
        list(&mut line, files);
    }
    if !commits.is_empty() {
        if !files.is_empty() {
            line("");
        }
        line(&format!(
            "Commits ahead of {} ({}):",
            brief.base,
            commits.len()
        ));
        list(&mut line, commits);
    }
    if !brief.sources.is_empty() {
        line("");
        line("Started from:");
        for source in brief.sources {
            let lane = source
                .lane
                .as_deref()
                .map(|task| format!(", in the lane for “{task}”"))
                .unwrap_or_default();
            let file = source
                .conversation
                .as_deref()
                .map(|path| format!(". Conversation: {path}"))
                .unwrap_or_default();
            line(&format!("- “{}”{lane}{file}", source.title));
        }
        line("");
        line("Read that conversation if you need the reasoning behind the changes.");
    }
    line(
        "Before changing anything, check the current diff, then tell me what is done and what is left.",
    );
    text.trim_end().to_string()
}

fn list(line: &mut impl FnMut(&str), items: &[String]) {
    for item in items.iter().take(BRIEF_LIST_LIMIT) {
        line(&format!("    {item}"));
    }
    if items.len() > BRIEF_LIST_LIMIT {
        line(&format!(
            "    … and {} more",
            items.len() - BRIEF_LIST_LIMIT
        ));
    }
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

    #[test]
    fn a_brief_holds_the_lane_its_changes_and_where_it_came_from() {
        let changes = LaneChanges {
            files: vec![" M src/lib.rs".into(), "?? src/new.rs".into()],
            commits: vec!["abc1234 Add the parser".into()],
        };
        let sources = [BriefSource {
            title: "Plan the parser".into(),
            lane: Some("Refactor the lexer".into()),
            conversation: Some("/home/me/.sessions/a.jsonl".into()),
        }];
        let text = handoff_brief(&Brief {
            task: "Add a parser",
            branch: "panoptes/add-a-parser",
            base: "main",
            worktree: "/lanes/add-a-parser/app",
            changes: &changes,
            sources: &sources,
        });
        assert!(text.contains("Task: Add a parser"));
        assert!(text.contains("Branch: panoptes/add-a-parser (from main)"));
        assert!(text.contains("Uncommitted changes (2):\n     M src/lib.rs\n    ?? src/new.rs"));
        assert!(text.contains("Commits ahead of main (1):\n    abc1234 Add the parser"));
        assert!(text.contains(
            "- “Plan the parser”, in the lane for “Refactor the lexer”. Conversation: /home/me/.sessions/a.jsonl"
        ));
        assert!(!text.ends_with('\n'));
    }

    #[test]
    fn a_brief_for_an_untouched_lane_says_so_and_cuts_long_lists() {
        let files: Vec<String> = (0..45).map(|i| format!("?? f{i}")).collect();
        let empty = handoff_brief(&Brief {
            task: "T",
            branch: "b",
            base: "main",
            worktree: "/w",
            changes: &LaneChanges::default(),
            sources: &[],
        });
        assert!(empty.contains("Nothing is changed or committed yet compared with main."));
        assert!(!empty.contains("Started from"));
        let long = handoff_brief(&Brief {
            task: "T",
            branch: "b",
            base: "main",
            worktree: "/w",
            changes: &LaneChanges {
                files,
                commits: Vec::new(),
            },
            sources: &[],
        });
        assert!(long.contains("    ?? f39\n    … and 5 more"));
        assert!(!long.contains("f40"));
    }

    #[test]
    fn only_lanes_seen_on_both_sides_can_change() {
        let before = HashMap::from([(1, "a".to_string()), (2, "b".to_string())]);
        let after = HashMap::from([
            (1, "a".to_string()),
            (2, "c".to_string()),
            (3, "d".to_string()),
        ]);
        assert_eq!(changed_lanes(&before, &after), [2]);
    }
}
