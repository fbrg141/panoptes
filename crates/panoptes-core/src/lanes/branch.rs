use std::path::Path;

use anyhow::bail;

use super::inspect::worktree_status;
use super::{DetachedHead, UnreachableCommit};
use crate::git::{git_path, ref_exists, run};
use crate::store::Lane;

/// Attaches a detached worktree to its lane branch without archiving or
/// deleting anything. This is also the recovery for unreachable commits.
pub fn create_branch_here(path: &Path, lane: &Lane) -> anyhow::Result<()> {
    match check_lane_branch(path, lane, Removal::Archive)? {
        BranchCheck::Ready => return Ok(()),
        BranchCheck::Detached(_) => {}
    }
    let head = run(path, &["rev-parse", "HEAD"])?;
    let reference = format!("refs/heads/{}", lane.branch);
    if ref_exists(path, &reference)? {
        let branch = run(path, &["rev-parse", "--verify", &reference])?;
        if branch == head {
            run(path, &["switch", &lane.branch])?;
        } else if run(path, &["merge-base", "--is-ancestor", &branch, &head]).is_ok() {
            // The user explicitly chose to recreate the lane branch here.
            // `-C` is safe only when this advances it to a descendant.
            run(path, &["switch", "-C", &lane.branch])?;
        } else {
            bail!(
                "The lane branch {} points to {}, which is not an ancestor of HEAD ({}). It was not moved; choose another branch or save the detached commits on a new branch first",
                lane.branch,
                branch,
                head
            );
        }
    } else {
        run(path, &["switch", "-c", &lane.branch])?;
    }
    Ok(())
}

/// Archiving keeps the lane's branch, so it is only safe when the worktree
/// is on that branch. Commits made on a detached HEAD or another branch would
/// otherwise become unreachable once the worktree is removed, and so would a
/// commit the Commit strategy makes there. A merge, rebase, cherry-pick,
/// revert or bisect in progress is refused for the same reason.
/// What the caller is about to do with the worktree, since deletion also
/// removes the lane branch and archiving keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Removal {
    Archive,
    Delete,
}

pub(super) enum BranchCheck {
    Ready,
    Detached(DetachedHead),
}

pub(super) fn check_lane_branch(
    path: &Path,
    lane: &Lane,
    removal: Removal,
) -> anyhow::Result<BranchCheck> {
    const IN_PROGRESS: [(&str, &str); 6] = [
        ("MERGE_HEAD", "a merge"),
        ("rebase-merge", "a rebase"),
        ("rebase-apply", "a rebase"),
        ("CHERRY_PICK_HEAD", "a cherry-pick"),
        ("REVERT_HEAD", "a revert"),
        ("BISECT_LOG", "a bisect"),
    ];
    for (name, operation) in IN_PROGRESS {
        if git_path(path, name)?.exists() {
            bail!(
                "The lane's worktree is in the middle of {operation}; finish or abort it before archiving or deleting the lane. The worktree was left in place"
            );
        }
    }
    let expected = format!("refs/heads/{}", lane.branch);
    match run(path, &["symbolic-ref", "--quiet", "HEAD"]) {
        Ok(head) if head == expected => Ok(BranchCheck::Ready),
        Ok(head) => bail!(
            "The lane's worktree is on {} instead of {}; switch back to {} before archiving or deleting the lane so its branch is unambiguous. The worktree was left in place",
            head.trim_start_matches("refs/heads/"),
            lane.branch,
            lane.branch
        ),
        Err(_) => Ok(BranchCheck::Detached(detached_head(path, lane, removal)?)),
    }
}

/// Lists every commit that would become unreachable from local and remote
/// branches if this detached worktree were removed. Tags deliberately do not
/// count: the recovery promise is specifically about branch reachability.
pub(super) fn detached_head(
    path: &Path,
    lane: &Lane,
    removal: Removal,
) -> anyhow::Result<DetachedHead> {
    let commit = run(path, &["rev-parse", "HEAD"])?;
    let dirty = !worktree_status(path)?.is_empty();
    // Deleting a lane also deletes its branch, so that branch cannot be what
    // keeps a detached commit reachable. Archiving keeps it, so it counts.
    // `--exclude` before `--branches` matches the short branch name.
    let exclude = format!("--exclude={}", lane.branch);
    let mut args = vec!["log", "--format=%H%x09%s", "HEAD", "--not"];
    if removal == Removal::Delete {
        args.push(&exclude);
    }
    args.extend(["--branches", "--remotes", "--"]);
    let log = run(path, &args)?;
    let unreachable_commits = log
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (hash, subject) = line.split_once('\t').unwrap_or((line, ""));
            UnreachableCommit {
                hash: hash.into(),
                subject: subject.into(),
            }
        })
        .collect();
    Ok(DetachedHead {
        commit,
        dirty,
        unreachable_commits,
    })
}

pub(super) fn current_branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}
