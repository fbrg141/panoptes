//! Lane worktrees on disk. The service owns persistence and calls these
//! functions on a blocking thread; this module owns Git, rollback, and the
//! archive choices that keep uncommitted work recoverable.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::git::{LANE_GIT_TIMEOUT, checked_stdout, git_path, ref_exists, run, run_index};
use crate::store::{Lane, NewLane, NewLaneWorktree, Project};

mod branch;
mod copy_files;
mod inspect;
mod naming;

pub use branch::create_branch_here;
use branch::{BranchCheck, Removal, check_lane_branch, current_branch, switch_back};
use copy_files::{copy_files, preserve_copy_files, restore_copy_files};
pub use inspect::{archive_preservation, work};
use inspect::{sole_worktree, worktree_status};
pub use naming::slug;
use naming::{free_name, worktree_path};

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
pub enum ArchiveResult {
    Dirty,
    DetachedHead(DetachedHead),
    OtherBranch(OtherBranch),
    Archived { checkpoint_ref: Option<String> },
}

#[derive(Debug)]
pub enum DeleteResult {
    Deleted,
    DetachedHead(DetachedHead),
    OtherBranch(OtherBranch),
}

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

/// Archives only after inspecting the current tree. A dirty tree with no
/// strategy is left completely untouched so the UI can offer the three
/// recovery choices. The branch itself is never deleted.
#[cfg(test)]
pub fn archive(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
) -> anyhow::Result<ArchiveResult> {
    archive_recording(project, lane, strategy, &|_| Ok(()))
}

/// Archives a lane, telling `record` which checkpoint ref holds its work
/// before the worktree is removed (and `None` if that checkpoint turned out
/// stale). The caller stores it, so an archive interrupted between removing
/// the folder and marking the lane archived can still find the checkpoint.
#[cfg(test)]
pub fn archive_recording(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
    record: &dyn Fn(Option<&str>) -> anyhow::Result<()>,
) -> anyhow::Result<ArchiveResult> {
    archive_recovering(project, lane, strategy, BranchRecovery::None, record)
}

/// `recovery` is the user's explicit choice in the detached-HEAD or
/// other-branch dialog. Recreating attaches the lane branch to HEAD without
/// ever resetting a branch that is ahead or has diverged; switching back
/// first commits any changes on the branch the worktree is on. Either runs
/// before the normal archive checks.
pub fn archive_recovering(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
    recovery: BranchRecovery,
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
        BranchCheck::Detached(_) if recovery == BranchRecovery::RecreateBranch => {
            create_branch_here(&path, lane)?
        }
        BranchCheck::Detached(detached) => return Ok(ArchiveResult::DetachedHead(detached)),
        BranchCheck::OtherBranch(other) if recovery == BranchRecovery::SwitchBack => {
            switch_back(&path, lane, &other)?
        }
        BranchCheck::OtherBranch(other) => return Ok(ArchiveResult::OtherBranch(other)),
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

/// Permanently removes a lane's worktree, local branch and recovery refs.
/// The caller has already obtained explicit confirmation. A main-checkout
/// lane owns none of those things, so deletion only removes its store row.
#[cfg(test)]
pub fn delete(project: &Project, lane: &Lane) -> anyhow::Result<DeleteResult> {
    delete_confirmed(project, lane, false)
}

/// Deletion goes ahead off the lane branch only after the user has confirmed
/// the detached-HEAD or other-branch dialog (`confirmed`). For a detached
/// HEAD the clean-tree and reachability checks are repeated here so a change
/// made while that dialog was open cannot be deleted. Another branch is
/// never deleted: it keeps its commits, and only the lane branch goes.
pub fn delete_confirmed(
    project: &Project,
    lane: &Lane,
    confirmed: bool,
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
            BranchCheck::Detached(detached) if confirmed && detached.is_safe_to_remove() => {}
            BranchCheck::Detached(detached) => {
                return Ok(DeleteResult::DetachedHead(detached));
            }
            BranchCheck::OtherBranch(_) if confirmed => {}
            BranchCheck::OtherBranch(other) => return Ok(DeleteResult::OtherBranch(other)),
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

fn run_setup(project: &Project, path: &Path, cancelled: Option<&AtomicBool>) -> anyhow::Result<()> {
    let Some(setup) = project.settings.setup.as_deref() else {
        return Ok(());
    };
    // Output goes to a file, not a pipe: a pipe nobody reads fills up after
    // about 64 KB and blocks the setup forever, and a background process
    // that inherits a pipe would keep it open after the shell exits.
    let log_path = std::env::temp_dir().join(format!(
        "panoptes-setup-{}-{}.log",
        std::process::id(),
        SETUP_LOGS.fetch_add(1, Ordering::Relaxed)
    ));
    let log = std::fs::File::create(&log_path).context("Could not create the setup log")?;
    let _log = TemporaryFile(log_path.clone());
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(setup)
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A dedicated group lets cancellation stop package managers and any
        // children they spawned, not just the shell.
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Could not run setup for {}", path.display()))?;
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            stop_setup(&mut child);
            bail!("Lane creation cancelled");
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let log = std::fs::read(&log_path).unwrap_or_default();
            let log = String::from_utf8_lossy(&log);
            bail!(
                "project setup failed ({status}): {}",
                tail(log.trim(), 4000)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

static SETUP_LOGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The last `max` bytes of `text`, on a character boundary.
fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn stop_setup(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: the child was started in its own process group. A negative
        // pid signals that group; it cannot name Panoptes' own group.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn remove_worktree(repo: &Path, path: &Path, force: bool) -> anyhow::Result<()> {
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

/// What a failed create or restore does with the lane branch.
enum BranchCleanup {
    Keep,
    Delete,
}

struct CreationCleanup {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
    branch_cleanup: BranchCleanup,
    armed: bool,
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

/// Saves every tracked and untracked change as a commit on top of HEAD,
/// without touching the real index, HEAD or branch. Returns its ref and the
/// tree it captured. The ref is keyed by the lane's name, not its store id:
/// debug and release builds keep separate stores that can hand out the same
/// id, while a name is unique in the repository because its `panoptes/<name>`
/// branch outlives archiving and new names skip existing branches.
fn checkpoint(project: &Project, lane: &Lane, path: &Path) -> anyhow::Result<(String, String)> {
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
fn reject_new_gitlinks(path: &Path, index: Option<&Path>) -> anyhow::Result<()> {
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

fn gitlink_path(entry: &str) -> Option<&str> {
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

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests;
