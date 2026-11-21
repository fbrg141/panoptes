use std::path::Path;

use anyhow::bail;

use super::branch::{BranchCheck, Removal, check_lane_branch};
use super::{ArchivePreservation, LaneWork};
use crate::git::{ref_exists, run};
use crate::store::{Lane, LaneWorktree, Project};

pub(super) fn sole_worktree(lane: &Lane) -> anyhow::Result<&LaneWorktree> {
    if lane.worktrees.len() != 1 {
        bail!("This operation needs exactly one worktree");
    }
    Ok(&lane.worktrees[0])
}

/// Every change in the tree, whatever the user's status settings say: with
/// `status.showUntrackedFiles=no` a plain `git status` hides new files, and
/// removing the worktree would then delete them.
pub(super) fn worktree_status(path: &Path) -> anyhow::Result<String> {
    run(
        path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )
}

/// The work in an active lane's worktree, or `None` when it can't be read.
pub(crate) fn work(project: &Project, lane: &Lane) -> Option<LaneWork> {
    let path = Path::new(&sole_worktree(lane).ok()?.path);
    let status = worktree_status(path).ok()?;
    let changed_files = status.lines().filter(|line| !line.is_empty()).count() as u32;
    let range = format!("{}..HEAD", project.settings.base_branch);
    let commits_ahead = run(path, &["rev-list", "--count", &range])
        .ok()
        .and_then(|count| count.parse().ok());
    Some(LaneWork {
        changed_files,
        commits_ahead,
    })
}

/// Read-only preflight for cleanup suggestions. `None` means the lane needs
/// its individual recovery flow (or cannot be read), so bulk cleanup leaves it
/// alone. Archiving repeats every check before removing the worktree.
pub(crate) fn archive_preservation(project: &Project, lane: &Lane) -> Option<ArchivePreservation> {
    let worktree = sole_worktree(lane).ok()?;
    if worktree.main_checkout {
        return None;
    }
    let path = Path::new(&worktree.path);
    let repo = Path::new(&project.repo);
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !repo.is_dir() {
                return None;
            }
            if let Some(reference) = worktree.checkpoint_ref.as_deref()
                && !ref_exists(repo, reference).ok()?
            {
                return None;
            }
            return Some(ArchivePreservation::Clean);
        }
        Err(_) => return None,
    }
    match check_lane_branch(path, lane, Removal::Archive).ok()? {
        BranchCheck::Ready => {}
        BranchCheck::Detached(_) => return None,
    }
    if worktree_status(path).ok()?.is_empty() {
        Some(ArchivePreservation::Clean)
    } else {
        Some(ArchivePreservation::Checkpoint)
    }
}
