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

pub use archive::archive_recovering;
#[cfg(test)]
pub use archive::{archive, archive_recording};
pub use branch::create_branch_here;
pub(crate) use checkpoint::snapshot_tree;
pub use create::{create, discard_created};
#[cfg(test)]
pub use delete::delete;
pub use delete::delete_detached;
pub use inspect::{archive_preservation, work};
pub use naming::slug;
pub use restore::restore;

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

#[derive(Debug)]
pub enum ArchiveResult {
    Dirty,
    DetachedHead(DetachedHead),
    Archived { checkpoint_ref: Option<String> },
}

#[derive(Debug)]
pub enum DeleteResult {
    Deleted,
    DetachedHead(DetachedHead),
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
