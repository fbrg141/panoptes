use std::path::{Path, PathBuf};

use anyhow::Context;

use super::DeleteResult;
use super::archive::remove_worktree;
use super::branch::{BranchCheck, Removal, check_lane_branch};
use super::inspect::sole_worktree;
use crate::git::{ref_exists, run};
use crate::store::{Lane, Project};

/// Deletion may skip branch recreation only after the user has confirmed the
/// detached-HEAD dialog. The clean-tree and reachability checks are repeated
/// here so a change made while that dialog was open cannot be deleted.
pub(crate) fn delete_detached(
    project: &Project,
    lane: &Lane,
    allow_detached: bool,
) -> anyhow::Result<DeleteResult> {
    let worktree = sole_worktree(lane)?;
    if worktree.main_checkout {
        return Ok(DeleteResult::Deleted);
    }
    let path = PathBuf::from(&worktree.path);
    let repo = Path::new(&project.repo);
    if !repo.is_dir() {
        // The repository was moved or deleted: its branch and refs went
        // with it, and only the worktree folder can be left. It is removed
        // only if it is a worktree (its `.git` is a file), never a checkout.
        if path.join(".git").is_file() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("Could not remove {}", path.display()))?;
        }
        return Ok(DeleteResult::Deleted);
    }
    if std::fs::symlink_metadata(&path).is_ok() {
        match check_lane_branch(&path, lane, Removal::Delete)? {
            BranchCheck::Ready => {}
            BranchCheck::Detached(detached) if allow_detached && detached.is_safe_to_remove() => {}
            BranchCheck::Detached(detached) => {
                return Ok(DeleteResult::DetachedHead(detached));
            }
        }
        // Deletion is explicitly destructive, including uncommitted files.
        remove_worktree(repo, &path, true)?;
    } else {
        run(repo, &["worktree", "prune"])?;
    }

    let branch = format!("refs/heads/{}", lane.branch);
    if ref_exists(repo, &branch)? {
        run(repo, &["branch", "-D", &lane.branch])?;
    }
    let prefix = format!("refs/panoptes/lanes/{}/", lane.name);
    let references = run(repo, &["for-each-ref", "--format=%(refname)", &prefix])?;
    for reference in references.lines().filter(|line| !line.is_empty()) {
        run(repo, &["update-ref", "-d", reference])?;
    }
    Ok(DeleteResult::Deleted)
}
