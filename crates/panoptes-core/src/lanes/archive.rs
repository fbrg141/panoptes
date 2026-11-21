use std::path::{Path, PathBuf};

use anyhow::bail;

use super::branch::{BranchCheck, Removal, check_lane_branch, create_branch_here};
use super::checkpoint::{checkpoint, reject_new_gitlinks, snapshot_tree};
use super::copy_files::preserve_copy_files;
use super::inspect::{sole_worktree, worktree_status};
use super::{ArchiveResult, ArchiveStrategy};
use crate::git::{ref_exists, run};
use crate::store::{Lane, Project};

/// `recreate_branch` is the user's explicit choice in the detached-HEAD
/// dialog. It attaches the lane branch to HEAD before the normal archive
/// checks, without ever resetting a branch that is ahead or has diverged.
pub(crate) fn archive_recovering(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
    recreate_branch: bool,
    record: &dyn Fn(Option<&str>) -> anyhow::Result<()>,
) -> anyhow::Result<ArchiveResult> {
    let worktree = sole_worktree(lane)?;
    if worktree.main_checkout {
        return Ok(ArchiveResult::Archived {
            checkpoint_ref: None,
        });
    }
    let path = PathBuf::from(&worktree.path);
    let repo = Path::new(&project.repo);
    // A folder deleted outside Panoptes holds no work to preserve. Prune
    // Git's record of it so the branch can be checked out again later.
    if std::fs::symlink_metadata(&path).is_err() {
        run(repo, &["worktree", "prune"])?;
        // Unless Panoptes itself removed it: an archive that stopped after
        // removing the folder had already recorded its checkpoint, and that
        // checkpoint is the lane's work.
        let checkpoint_ref = match worktree.checkpoint_ref.as_deref() {
            Some(reference) if ref_exists(repo, reference)? => Some(reference.to_string()),
            _ => None,
        };
        return Ok(ArchiveResult::Archived { checkpoint_ref });
    }
    match check_lane_branch(&path, lane, Removal::Archive)? {
        BranchCheck::Ready => {}
        BranchCheck::Detached(_) if recreate_branch => create_branch_here(&path, lane)?,
        BranchCheck::Detached(detached) => return Ok(ArchiveResult::DetachedHead(detached)),
    }
    let before = worktree_status(&path)?;
    let is_dirty = !before.is_empty();
    if is_dirty && strategy.is_none() {
        return Ok(ArchiveResult::Dirty);
    }

    let mut checkpoint_ref = None;
    let mut checkpoint_tree = None;
    if is_dirty {
        match strategy {
            Some(ArchiveStrategy::Commit) => {
                run(&path, &["add", "-A"])?;
                reject_new_gitlinks(&path, None)?;
                run(
                    &path,
                    &["commit", "-m", &format!("Panoptes: archive {}", lane.name)],
                )?;
            }
            Some(ArchiveStrategy::Stash) => {
                run(
                    &path,
                    &[
                        "stash",
                        "push",
                        "--include-untracked",
                        "-m",
                        &format!("Panoptes lane {}", lane.name),
                    ],
                )?;
            }
            Some(ArchiveStrategy::Checkpoint) => {
                let (reference, tree) = checkpoint(project, lane, &path)?;
                record(Some(&reference))?;
                checkpoint_ref = Some(reference);
                checkpoint_tree = Some(tree);
            }
            None => {}
        }
    }
    match (strategy, checkpoint_tree) {
        // Compare contents, not `git status` text: editing a file that is
        // already modified changes nothing in the status lines. Snapshotting
        // again right before removal keeps the unprotected window minimal.
        (Some(ArchiveStrategy::Checkpoint), Some(tree)) if is_dirty => {
            if snapshot_tree(&path, lane)? != tree {
                // The recorded checkpoint no longer holds the lane's work.
                let _ = record(None);
                bail!(
                    "The tree changed while its checkpoint was being taken; the worktree was left in place"
                )
            }
        }
        // A clean tree can also change between the first check and removal.
        _ if !is_dirty => {
            if !worktree_status(&path)?.is_empty() {
                return Ok(ArchiveResult::Dirty);
            }
        }
        _ => {
            if !worktree_status(&path)?.is_empty() {
                bail!("Changes remain after archiving; the worktree was left in place")
            }
        }
    }
    preserve_copy_files(project, lane, &path)?;
    // Only checkpoint removal is intentionally dirty and therefore forced.
    // Every other path lets Git refuse work that appeared after our checks.
    remove_worktree(
        repo,
        &path,
        matches!(strategy, Some(ArchiveStrategy::Checkpoint)),
    )?;
    Ok(ArchiveResult::Archived { checkpoint_ref })
}

pub(super) fn remove_worktree(repo: &Path, path: &Path, force: bool) -> anyhow::Result<()> {
    if force {
        run(
            repo,
            &["worktree", "remove", "--force", &path.to_string_lossy()],
        )?;
    } else {
        run(repo, &["worktree", "remove", &path.to_string_lossy()])?;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}
