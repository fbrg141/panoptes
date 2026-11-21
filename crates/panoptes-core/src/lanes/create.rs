use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};

use super::archive::remove_worktree;
use super::branch::current_branch;
use super::copy_files::copy_files;
use super::naming::{free_name, worktree_path};
use super::setup::run_setup;
use crate::git::{ref_exists, run};
use crate::store::{NewLane, NewLaneWorktree, Project};

/// Makes a worktree and its branch, copies the project's local files, and
/// runs setup. Any failure removes everything this call created. Missing copy
/// sources (the usual case for `.env`) are simply absent in the new lane.
/// The lane is named after `base`, with a numeric suffix if `taken` or the
/// disk already uses that name.
pub fn create(
    root: &Path,
    project: &Project,
    base: &str,
    taken: &[String],
    task: &str,
    main_checkout: bool,
    cancelled: &AtomicBool,
) -> anyhow::Result<NewLane> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("Lane creation cancelled");
    }
    let repo = PathBuf::from(&project.repo);
    let name = free_name(root, &repo, base, taken, main_checkout)?;
    if main_checkout {
        return Ok(NewLane {
            project_id: project.id,
            name,
            task: task.into(),
            branch: current_branch(&repo).unwrap_or_else(|| project.settings.base_branch.clone()),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: project.repo.clone(),
                main_checkout: true,
            }],
        });
    }

    let branch = format!("panoptes/{name}");
    let path = worktree_path(root, &repo, &name);
    std::fs::create_dir_all(path.parent().context("A lane needs a parent folder")?)
        .with_context(|| format!("Could not create the folder for {name}"))?;

    let mut cleanup = CreationCleanup {
        repo: repo.clone(),
        path: path.clone(),
        branch: branch.clone(),
        branch_cleanup: BranchCleanup::Delete,
        armed: true,
    };
    run(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            &branch,
            &path.to_string_lossy(),
            &project.settings.base_branch,
        ],
    )?;
    copy_files(project, &path)?;
    if cancelled.load(Ordering::Relaxed) {
        bail!("Lane creation cancelled");
    }
    run_setup(project, &path, Some(cancelled))?;
    cleanup.armed = false;

    Ok(NewLane {
        project_id: project.id,
        name,
        task: task.into(),
        branch,
        worktrees: vec![NewLaneWorktree {
            project_id: project.id,
            path: path.to_string_lossy().into_owned(),
            main_checkout: false,
        }],
    })
}

/// A small explicit rollback helper used by the service when SQLite refuses
/// the new record. It takes the project's repository separately because a
/// `NewLane` intentionally contains only what gets persisted.
pub fn discard_created(project: &Project, new: &NewLane) {
    if new.worktrees.iter().any(|worktree| worktree.main_checkout) {
        return;
    }
    for worktree in &new.worktrees {
        let path = PathBuf::from(&worktree.path);
        let _ = run(
            Path::new(&project.repo),
            &["worktree", "remove", "--force", &path.to_string_lossy()],
        );
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap_or_else(|| Path::new("")));
    }
    let _ = run(Path::new(&project.repo), &["branch", "-D", &new.branch]);
}

/// What a failed create or restore does with the lane branch.
pub(super) enum BranchCleanup {
    Keep,
    Delete,
}

pub(super) struct CreationCleanup {
    pub(super) repo: PathBuf,
    pub(super) path: PathBuf,
    pub(super) branch: String,
    pub(super) branch_cleanup: BranchCleanup,
    pub(super) armed: bool,
}

impl Drop for CreationCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = remove_worktree(&self.repo, &self.path, true);
        // `git worktree add` can fail after creating the destination but
        // before registering it. This path was required not to exist before
        // the attempt, so removing whatever the attempt left is safe.
        let _ = std::fs::remove_dir_all(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        let full_ref = format!("refs/heads/{}", self.branch);
        match &self.branch_cleanup {
            BranchCleanup::Keep => {}
            BranchCleanup::Delete => {
                if ref_exists(&self.repo, &full_ref).unwrap_or(false) {
                    let _ = run(&self.repo, &["branch", "-D", &self.branch]);
                }
            }
        }
    }
}
