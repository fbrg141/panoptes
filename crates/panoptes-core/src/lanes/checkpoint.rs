use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::git::{LANE_GIT_TIMEOUT, checked_stdout, git_path, run, run_index};
use crate::store::{Lane, Project};

/// Saves every tracked and untracked change as a commit on top of HEAD,
/// without touching the real index, HEAD or branch. Returns its ref and the
/// tree it captured. The ref is keyed by the lane's name, not its store id:
/// debug and release builds keep separate stores that can hand out the same
/// id, while a name is unique in the repository because its `panoptes/<name>`
/// branch outlives archiving and new names skip existing branches.
pub(super) fn checkpoint(
    project: &Project,
    lane: &Lane,
    path: &Path,
) -> anyhow::Result<(String, String)> {
    let tree = snapshot_tree(path, lane)?;
    let output = crate::git::output_with_timeout(
        crate::git::command()
            .arg("-C")
            .arg(path)
            .args(["commit-tree", &tree, "-p", "HEAD", "-m"])
            .arg(format!("Panoptes checkpoint for {}", lane.name))
            .env("GIT_AUTHOR_NAME", "Panoptes")
            .env("GIT_AUTHOR_EMAIL", "panoptes@localhost")
            .env("GIT_COMMITTER_NAME", "Panoptes")
            .env("GIT_COMMITTER_EMAIL", "panoptes@localhost"),
        None,
        LANE_GIT_TIMEOUT,
    )
    .context("Could not write the lane checkpoint")?;
    let commit = checked_stdout("git commit-tree", output)?;
    let reference = format!("refs/panoptes/lanes/{}/archive", lane.name);
    run(
        Path::new(&project.repo),
        &["update-ref", &reference, &commit],
    )?;
    Ok((reference, tree))
}

/// An untracked nested repository becomes a gitlink with no commits in the
/// outer repository. Removing the worktree would then delete its contents.
/// Gitlinks already present in HEAD are real submodules and are allowed.
pub(super) fn reject_new_gitlinks(path: &Path, index: Option<&Path>) -> anyhow::Result<()> {
    let staged = match index {
        Some(index) => run_index(path, index, &["ls-files", "--stage", "-z"])?,
        None => run(path, &["ls-files", "--stage", "-z"])?,
    };
    let head = run(path, &["ls-tree", "-r", "-z", "HEAD"])?;
    let existing = head
        .split('\0')
        .filter_map(gitlink_path)
        .collect::<Vec<_>>();
    for entry in staged.split('\0') {
        if let Some(repository) = gitlink_path(entry)
            && !existing.contains(&repository)
        {
            bail!(
                "Nested repository at {repository} is not a gitlink in HEAD; archiving was stopped to keep its contents"
            );
        }
    }
    Ok(())
}

pub(super) fn gitlink_path(entry: &str) -> Option<&str> {
    entry
        .strip_prefix("160000 ")?
        .split_once('\t')
        .map(|(_, path)| path)
}

/// The tree of the worktree as it is on disk, tracked and untracked files
/// alike, written through a private copy of the index.
pub(crate) fn snapshot_tree(path: &Path, lane: &Lane) -> anyhow::Result<String> {
    let index = git_path(path, "index")?;
    let temporary = index.with_file_name(format!(
        "panoptes-archive-{}-{}",
        std::process::id(),
        lane.id
    ));
    let _temporary = TemporaryFile(temporary.clone());
    match std::fs::copy(&index, &temporary) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            run_index(path, &temporary, &["read-tree", "HEAD"])?;
        }
        Err(error) => return Err(error).context("Could not copy the worktree index"),
    }
    run_index(path, &temporary, &["add", "-A"])?;
    reject_new_gitlinks(path, Some(&temporary))?;
    run_index(path, &temporary, &["write-tree"])
}

pub(super) struct TemporaryFile(pub(super) PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
