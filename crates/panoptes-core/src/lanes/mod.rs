//! Lane worktrees on disk. The service owns persistence and calls these
//! functions on a blocking thread; this module owns Git, rollback, and the
//! archive choices that keep uncommitted work recoverable.

use serde::{Deserialize, Serialize};

mod archive;
mod branch;
mod checkpoint;
mod copy_files;
mod create;
mod delete;
mod inspect;
mod naming;
mod restore;
mod setup;
pub(crate) mod turn;

pub(crate) use archive::archive_recovering;
pub(crate) use branch::create_branch_here;
pub(crate) use checkpoint::snapshot_tree;
pub(crate) use create::{create, discard_created};
pub(crate) use delete::delete_confirmed;
pub(crate) use inspect::{archive_preservation, work};
pub(crate) use naming::slug;
pub(crate) use restore::restore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveStrategy {
    Commit,
    Stash,
    Checkpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreachableCommit {
    pub hash: String,
    pub subject: String,
}

/// Why a detached worktree cannot simply be removed. An empty commit list and
/// a clean tree mean HEAD is already reachable from a local or remote branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachedHead {
    pub commit: String,
    pub dirty: bool,
    pub unreachable_commits: Vec<UnreachableCommit>,
}

impl DetachedHead {
    pub fn is_safe_to_remove(&self) -> bool {
        !self.dirty && self.unreachable_commits.is_empty()
    }
}

/// The worktree is on a branch other than the lane's. That branch keeps its
/// commits whatever happens to the worktree; only uncommitted changes (when
/// `dirty`) live in the worktree alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherBranch {
    pub branch: String,
    pub dirty: bool,
}

/// The branch fix the user chose in a dialog before archiving again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchRecovery {
    #[default]
    None,
    /// Attach the lane branch to a detached HEAD.
    RecreateBranch,
    /// Commit any changes on the other branch, then switch back to the lane's.
    SwitchBack,
}

#[derive(Debug)]
pub(crate) enum ArchiveResult {
    Dirty,
    DetachedHead(DetachedHead),
    OtherBranch(OtherBranch),
    Archived { checkpoint_ref: Option<String> },
}

#[derive(Debug)]
pub(crate) enum DeleteResult {
    Deleted,
    DetachedHead(DetachedHead),
    OtherBranch(OtherBranch),
}

/// What a lane's worktree holds compared with where it started: the files
/// changed and not yet committed, and the commits on its branch that the
/// project's base branch does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneWork {
    pub changed_files: u32,
    /// `None` when the base branch can't be compared, such as once it is gone.
    pub commits_ahead: Option<u32>,
}

/// What the normal archive path will do with an eligible lane's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchivePreservation {
    Clean,
    Checkpoint,
}

#[cfg(test)]
mod tests;
