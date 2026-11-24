//! Durable before/after commits for one lane turn. The refs sit beside the
//! lane branch and are removed with the lane's generated ref prefix.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, bail};

use super::checkpoint::snapshot_tree;
use crate::git::{LANE_GIT_TIMEOUT, checked_stdout, ref_exists, run};
use crate::store::Lane;

/// Reads the tree of an existing ref, or `None` when it has not been
/// written. Used to adopt an interrupted snapshot without recreating it.
pub(crate) fn turn_tree(repo: &Path, reference: &str) -> anyhow::Result<Option<String>> {
    if !ref_exists(repo, reference)? {
        return Ok(None);
    }
    Ok(Some(run(
        repo,
        &["rev-parse", &format!("{reference}^{{tree}}")],
    )?))
}

/// Writes a snapshot of the worktree and points one deterministic ref at it.
/// `parent_ref` is a previous generated commit ref when a chain exists;
/// otherwise HEAD is the parent. This never moves HEAD or the branch.
pub(crate) fn write_turn_snapshot(
    worktree: &Path,
    repo: &Path,
    lane: &Lane,
    reference: &str,
    parent_ref: Option<&str>,
) -> anyhow::Result<String> {
    let tree = snapshot_tree(worktree, lane)?;
    write_tree_snapshot(repo, lane, reference, &tree, parent_ref)?;
    Ok(tree)
}

/// Writes a commit for a tree that was read from an archive checkpoint or
/// recovered ref. The tree itself is not a ref, so this keeps it reachable.
pub(crate) fn write_tree_snapshot(
    repo: &Path,
    _lane: &Lane,
    reference: &str,
    tree: &str,
    parent_ref: Option<&str>,
) -> anyhow::Result<()> {
    let parent = match parent_ref {
        Some(reference) if ref_exists(repo, reference)? => {
            run(repo, &["rev-parse", "--verify", reference])?
        }
        _ => run(repo, &["rev-parse", "HEAD"])?,
    };
    let message = "Panoptes turn snapshot";
    let mut command = crate::git::command();
    command
        .arg("-C")
        .arg(repo)
        .args(["-c", "commit.gpgsign=false", "commit-tree", tree, "-p"])
        .arg(&parent)
        .args(["-m", message])
        .env("GIT_AUTHOR_NAME", "Panoptes")
        .env("GIT_AUTHOR_EMAIL", "panoptes@localhost")
        .env("GIT_COMMITTER_NAME", "Panoptes")
        .env("GIT_COMMITTER_EMAIL", "panoptes@localhost");
    let output = crate::git::output_with_timeout(&mut command, None, LANE_GIT_TIMEOUT)
        .context("Could not write a turn snapshot")?;
    let commit = checked_stdout("git commit-tree", output)?;
    run(repo, &["update-ref", reference, &commit])?;
    Ok(())
}

pub(crate) fn remove_turn_ref(repo: &Path, reference: &str) -> anyhow::Result<()> {
    if !reference.starts_with("refs/panoptes/lanes/") {
        bail!("Refusing to remove a ref Panoptes did not generate: {reference}");
    }
    if ref_exists(repo, reference)? {
        run(repo, &["update-ref", "-d", reference])?;
    }
    Ok(())
}

/// Removes generated turn refs that no stored turn row owns. User refs and
/// refs outside this lane's generated turn prefix are never touched.
pub(crate) fn remove_orphan_turn_refs(
    repo: &Path,
    lane: &Lane,
    owned: &HashSet<String>,
) -> anyhow::Result<()> {
    let prefix = format!("refs/panoptes/lanes/{}/turns/", lane.name);
    let found = run(repo, &["for-each-ref", "--format=%(refname)", &prefix])?;
    for reference in found.lines().filter(|line| !line.is_empty()) {
        if !owned.contains(reference) {
            remove_turn_ref(repo, reference)?;
        }
    }
    Ok(())
}
