//! Presentation rules for the read-only Git worktree inventory.

use panoptes_core::cleanup::WorktreeInfo;
use panoptes_core::store::LaneState;

use super::format::format_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeTone {
    Neutral,
    Managed,
    Archived,
    Locked,
    Stale,
}

pub fn is_stale(worktree: &WorktreeInfo) -> bool {
    worktree.prunable || !worktree.exists
}

pub fn label(worktree: &WorktreeInfo) -> &'static str {
    if is_stale(worktree) {
        "Stale registration"
    } else if worktree.locked {
        "Locked"
    } else if worktree.bare {
        "Bare repository"
    } else if worktree.main_checkout {
        "Main checkout"
    } else {
        match worktree.lane_state {
            Some(LaneState::Active) => "Active lane",
            Some(LaneState::Archived) => "Archived lane",
            None => "Unmanaged",
        }
    }
}

pub fn tone(worktree: &WorktreeInfo) -> WorktreeTone {
    if is_stale(worktree) {
        WorktreeTone::Stale
    } else if worktree.locked {
        WorktreeTone::Locked
    } else {
        match worktree.lane_state {
            Some(LaneState::Active) => WorktreeTone::Managed,
            Some(LaneState::Archived) => WorktreeTone::Archived,
            None => WorktreeTone::Neutral,
        }
    }
}

pub fn size(worktree: &WorktreeInfo) -> String {
    if !worktree.exists {
        "Not on disk".into()
    } else {
        worktree
            .bytes
            .map_or_else(|| "Size unknown".into(), format_bytes)
    }
}

pub fn detail(worktree: &WorktreeInfo) -> String {
    let mut parts = vec![worktree.project.clone()];
    if let Some(lane) = &worktree.lane {
        parts.push(lane.clone());
    }
    if let Some(branch) = &worktree.branch {
        parts.push(branch.clone());
    } else if worktree.detached {
        parts.push("detached HEAD".into());
    }
    if let Some(head) = &worktree.head {
        parts.push(head.chars().take(7).collect());
    }
    match worktree.dirty {
        Some(true) => parts.push("dirty".into()),
        Some(false) => parts.push("clean".into()),
        None if !worktree.exists => {}
        None => parts.push("status unknown".into()),
    }
    if worktree.locked {
        parts.push("locked".into());
    }
    if worktree.prunable {
        parts.push("prunable".into());
    }
    if worktree.bare {
        parts.push("bare".into());
    }
    parts.join(" \u{b7} ")
}

pub fn filter(
    worktrees: &[WorktreeInfo],
    project_id: i64,
    all_projects: bool,
    managed_only: bool,
) -> Vec<&WorktreeInfo> {
    worktrees
        .iter()
        .filter(|worktree| all_projects || worktree.project_id == project_id)
        .filter(|worktree| !managed_only || worktree.managed)
        .collect()
}

pub fn usage(worktrees: &[&WorktreeInfo]) -> (u64, u64) {
    (
        worktrees.len() as u64,
        worktrees.iter().filter_map(|worktree| worktree.bytes).sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree(project_id: i64, managed: bool) -> WorktreeInfo {
        WorktreeInfo {
            project_id,
            project: format!("Project {project_id}"),
            path: format!("/tmp/worktree-{project_id}"),
            exists: true,
            bytes: Some(2_048),
            managed,
            main_checkout: !managed,
            lane_state: managed.then_some(LaneState::Active),
            lane: managed.then(|| "Fix it".into()),
            branch: Some("trunk".into()),
            head: Some("abcdef123456".into()),
            ..Default::default()
        }
    }

    #[test]
    fn filters_by_project_and_management() {
        let worktrees = vec![worktree(1, true), worktree(1, false), worktree(2, true)];
        assert_eq!(filter(&worktrees, 1, true, false).len(), 3);
        assert_eq!(filter(&worktrees, 1, false, false).len(), 2);
        assert_eq!(filter(&worktrees, 1, true, true).len(), 2);
        assert_eq!(filter(&worktrees, 2, false, true).len(), 1);
    }

    #[test]
    fn rows_describe_state_size_and_usage() {
        let mut managed = worktree(1, true);
        managed.dirty = Some(false);
        assert_eq!(label(&managed), "Active lane");
        assert_eq!(tone(&managed), WorktreeTone::Managed);
        assert_eq!(size(&managed), "2 KB");
        assert_eq!(
            detail(&managed),
            "Project 1 \u{b7} Fix it \u{b7} trunk \u{b7} abcdef1 \u{b7} clean"
        );
        assert_eq!(usage(&[&managed]), (1, 2_048));

        let mut stale = worktree(2, false);
        stale.exists = false;
        stale.bytes = None;
        assert_eq!(label(&stale), "Stale registration");
        assert_eq!(tone(&stale), WorktreeTone::Stale);
        assert_eq!(size(&stale), "Not on disk");
        assert_eq!(detail(&stale), "Project 2 \u{b7} trunk \u{b7} abcdef1");
    }
}
