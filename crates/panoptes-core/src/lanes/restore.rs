use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::copy_files::{copy_files, restore_copy_files};
use super::create::{BranchCleanup, CreationCleanup};
use super::inspect::sole_worktree;
use super::setup::run_setup;
use crate::git::{ref_exists, run};
use crate::store::{Lane, Project};

/// Brings an archived lane's worktree back on its branch. A checkpoint is
/// put back as it was taken: as uncommitted changes in the worktree. The
/// branch is not moved, so the work never turns into a commit the user did
/// not make. If the branch has moved since the checkpoint was taken, nothing
/// is changed: the checkpoint describes an older tree, and laying it over
/// the newer one would undo those commits in the worktree. Any later failure
/// removes the worktree again; the checkpoint ref stays either way.
pub fn restore(project: &Project, lane: &Lane) -> anyhow::Result<()> {
    let worktree = sole_worktree(lane)?;
    if worktree.main_checkout {
        return Ok(());
    }
    let path = PathBuf::from(&worktree.path);
    if std::fs::symlink_metadata(&path).is_ok() {
        bail!("{} already exists", path.display());
    }
    let repo = Path::new(&project.repo);
    let full_ref = format!("refs/heads/{}", lane.branch);
    let mut branch_cleanup = BranchCleanup::Keep;
    let mut checkpoint = None;
    if let Some(reference) = worktree.checkpoint_ref.as_deref() {
        let commit = run(
            repo,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )
        .context("The lane's checkpoint is missing from the repository")?;
        let parent = run(repo, &["rev-parse", "--verify", &format!("{commit}^")])?;
        if ref_exists(repo, &full_ref)? {
            let tip = run(repo, &["rev-parse", "--verify", &full_ref])?;
            if tip != parent {
                bail!(
                    "{} has new commits since the lane was archived, so its checkpoint no longer fits on top. Its changes are kept at {reference}",
                    lane.branch
                );
            }
        } else {
            // The branch was deleted meanwhile: bring it back where the
            // checkpoint was taken. The empty old value refuses to replace
            // a branch that appeared in between.
            run(repo, &["update-ref", &full_ref, &parent, ""])?;
            branch_cleanup = BranchCleanup::Delete;
        }
        checkpoint = Some(commit);
    } else if !ref_exists(repo, &full_ref)? {
        bail!("The branch {} no longer exists", lane.branch);
    }
    let mut cleanup = CreationCleanup {
        repo: repo.to_path_buf(),
        path: path.clone(),
        branch: lane.branch.clone(),
        branch_cleanup,
        armed: true,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    run(
        repo,
        &[
            "worktree",
            "add",
            "--quiet",
            &path.to_string_lossy(),
            &lane.branch,
        ],
    )?;
    if let Some(commit) = checkpoint {
        // Only the working files change: modified files come back modified,
        // new files untracked and deleted files deleted, with nothing staged.
        run(
            &path,
            &["restore", "--source", &commit, "--worktree", "--", "."],
        )?;
    }
    restore_copy_files(project, lane, &path)?;
    copy_files(project, &path)?;
    run_setup(project, &path, None)?;
    cleanup.armed = false;
    Ok(())
}
