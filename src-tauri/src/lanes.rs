//! Lane worktrees on disk. The service owns persistence and calls these
//! functions on a blocking thread; this module owns Git, rollback, and the
//! archive choices that keep uncommitted work recoverable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::store::{Lane, LaneWorktree, NewLane, NewLaneWorktree, Project};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveStrategy {
    Commit,
    Stash,
    Checkpoint,
}

#[derive(Debug)]
pub enum ArchiveResult {
    Dirty,
    Archived { checkpoint_ref: Option<String> },
}

/// Filesystem-safe lane names are also used in branch names and paths.
pub fn slug(task: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in task.chars() {
        if character.is_ascii_alphanumeric() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            dash = false;
        } else {
            dash = true;
        }
    }
    // Truncate first, then trim, so a cut never leaves a trailing dash.
    let slug = slug.chars().take(48).collect::<String>();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "lane".into()
    } else {
        slug.into()
    }
}

fn worktree_path(root: &Path, repo: &Path, name: &str) -> PathBuf {
    root.join(name).join(
        repo.file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("repo")),
    )
}

/// The first of `base`, `base-2`, `base-3`, … that no stored lane of this
/// project uses and, for a fresh worktree, whose folder and branch are free.
/// The folder and branch checks matter because other projects (with the
/// same repository name) and other Panoptes stores share the lanes root and
/// the repository's branches.
fn free_name(
    root: &Path,
    repo: &Path,
    base: &str,
    taken: &[String],
    main_checkout: bool,
) -> anyhow::Result<String> {
    for suffix in 1.. {
        let name = if suffix == 1 {
            base.to_string()
        } else {
            format!("{base}-{suffix}")
        };
        if taken.contains(&name) {
            continue;
        }
        if main_checkout {
            return Ok(name);
        }
        let branch_free = !ref_exists(repo, &format!("refs/heads/panoptes/{name}"))?;
        // `exists` follows links; a dangling one still occupies the name.
        let folder_free = std::fs::symlink_metadata(root.join(&name)).is_err();
        if branch_free && folder_free {
            return Ok(name);
        }
    }
    unreachable!("an unbounded range always yields a name")
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
pub fn archive(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
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
        return Ok(ArchiveResult::Archived {
            checkpoint_ref: None,
        });
    }
    ensure_on_lane_branch(&path, lane)?;
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

/// Brings an archived lane's worktree back on its branch. For a checkpoint,
/// the checkpoint's parent is the branch tip it was taken from, so the
/// branch is fast-forwarded onto it, which recovers both tracked and
/// untracked captured files. If the branch has moved since, nothing is
/// changed: forcing it would drop those newer commits. Any later failure
/// puts the branch back where it was.
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
    if let Some(reference) = worktree.checkpoint_ref.as_deref() {
        let checkpoint = run(
            repo,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )
        .context("The lane's checkpoint is missing from the repository")?;
        let parent = run(repo, &["rev-parse", "--verify", &format!("{checkpoint}^")])?;
        if ref_exists(repo, &full_ref)? {
            let tip = run(repo, &["rev-parse", "--verify", &full_ref])?;
            if tip != parent {
                bail!(
                    "{} has new commits since the lane was archived; restoring the checkpoint would drop them. Its changes are kept at {reference}",
                    lane.branch
                );
            }
            // The old-value argument makes this a compare-and-swap.
            run(repo, &["update-ref", &full_ref, &checkpoint, &parent])?;
            branch_cleanup = BranchCleanup::Reset(parent);
        } else {
            run(repo, &["update-ref", &full_ref, &checkpoint, ""])?;
            branch_cleanup = BranchCleanup::Delete;
        }
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
    restore_copy_files(project, lane, &path)?;
    copy_files(project, &path)?;
    run_setup(project, &path, None)?;
    cleanup.armed = false;
    Ok(())
}

/// Permanently removes a lane's worktree, local branch and recovery refs.
/// The caller has already obtained explicit confirmation. A main-checkout
/// lane owns none of those things, so deletion only removes its store row.
pub fn delete(project: &Project, lane: &Lane) -> anyhow::Result<()> {
    let worktree = sole_worktree(lane)?;
    if worktree.main_checkout {
        return Ok(());
    }
    let path = PathBuf::from(&worktree.path);
    let repo = Path::new(&project.repo);
    if std::fs::symlink_metadata(&path).is_ok() {
        ensure_on_lane_branch(&path, lane)?;
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
    Ok(())
}

fn sole_worktree(lane: &Lane) -> anyhow::Result<&LaneWorktree> {
    if lane.worktrees.len() != 1 {
        bail!("This operation needs exactly one worktree");
    }
    Ok(&lane.worktrees[0])
}

/// Every change in the tree, whatever the user's status settings say: with
/// `status.showUntrackedFiles=no` a plain `git status` hides new files, and
/// removing the worktree would then delete them.
fn worktree_status(path: &Path) -> anyhow::Result<String> {
    run(
        path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )
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

/// The work in an active lane's worktree, or `None` when it can't be read.
pub fn work(project: &Project, lane: &Lane) -> Option<LaneWork> {
    let path = Path::new(&sole_worktree(lane).ok()?.path);
    let status = worktree_status(path).ok()?;
    let changed_files = status.lines().filter(|line| !line.is_empty()).count() as u32;
    let range = format!("{}..HEAD", project.settings.base_branch);
    let commits_ahead = run(path, &["rev-list", "--count", &range])
        .ok()
        .and_then(|count| count.parse().ok());
    Some(LaneWork {
        changed_files,
        commits_ahead,
    })
}

/// The uncommitted files and the commits a lane's branch adds to the base
/// branch, as git lists them, for a handoff to read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneChanges {
    /// `git status --porcelain` lines, such as ` M src/lib.rs`.
    pub files: Vec<String>,
    /// One line per commit, newest first.
    pub commits: Vec<String>,
}

pub fn changes(project: &Project, lane: &Lane) -> LaneChanges {
    let Ok(worktree) = sole_worktree(lane) else {
        return LaneChanges::default();
    };
    let path = Path::new(&worktree.path);
    let lines = |text: String| -> Vec<String> {
        text.lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect()
    };
    let range = format!("{}..HEAD", project.settings.base_branch);
    LaneChanges {
        files: worktree_status(path).map(lines).unwrap_or_default(),
        commits: run(path, &["log", "--format=%h %s", &range])
            .map(lines)
            .unwrap_or_default(),
    }
}

/// Identifies the exact state of a lane's worktree, untracked files
/// included, so a change between two moments can be told apart from none.
/// It writes only loose objects for the snapshot, never the real index.
pub fn fingerprint(lane: &Lane) -> Option<String> {
    let path = Path::new(&sole_worktree(lane).ok()?.path);
    let head = run(path, &["rev-parse", "--verify", "--quiet", "HEAD"]).unwrap_or_default();
    // A snapshot can refuse new nested repositories; their status still
    // tells a change apart.
    let tree = snapshot_tree_as(path, &format!("fingerprint-{}", lane.id))
        .or_else(|_| worktree_status(path))
        .ok()?;
    Some(format!("{head} {tree}"))
}

/// Archiving keeps the lane's branch, so it is only safe when the worktree
/// is on that branch. Commits made on a detached HEAD or another branch would
/// otherwise become unreachable once the worktree is removed, and so would a
/// commit the Commit strategy makes there. A merge, rebase, cherry-pick,
/// revert or bisect in progress is refused for the same reason.
fn ensure_on_lane_branch(path: &Path, lane: &Lane) -> anyhow::Result<()> {
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
        Ok(head) if head == expected => Ok(()),
        Ok(head) => bail!(
            "The lane's worktree is on {} instead of {}; switch back to {} before archiving or deleting the lane so its branch is unambiguous. The worktree was left in place",
            head.trim_start_matches("refs/heads/"),
            lane.branch,
            lane.branch
        ),
        Err(_) => bail!(
            "The lane's worktree has a detached HEAD; switch back to {} before archiving or deleting the lane so its branch is unambiguous. The worktree was left in place",
            lane.branch
        ),
    }
}

fn current_branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
}

fn copy_files_reference(lane: &Lane) -> String {
    format!("refs/panoptes/lanes/{}/copy-files", lane.name)
}

/// Saves the lane's current configured copies outside the branch and index.
/// These files are often ignored, so the tracked checkpoint cannot retain
/// them. A tree ref keeps them without making them tracked on restore.
fn preserve_copy_files(project: &Project, lane: &Lane, path: &Path) -> anyhow::Result<()> {
    let repo = Path::new(&project.repo);
    let reference = copy_files_reference(lane);
    let index = git_path(path, "index")?;
    let temporary = index.with_file_name(format!(
        "panoptes-copy-files-{}-{}",
        std::process::id(),
        lane.id
    ));
    let _temporary = TemporaryFile(temporary.clone());
    run_index(path, &temporary, &["read-tree", "--empty"])?;
    let mut any = false;
    for relative in &project.settings.copy_files {
        let source = path.join(relative);
        match std::fs::metadata(&source) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => bail!("{} is not a file", source.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Could not read {}", source.display()));
            }
        }
        run_index(path, &temporary, &["add", "-f", "--", relative])?;
        any = true;
    }
    if !any {
        if ref_exists(repo, &reference)? {
            run(repo, &["update-ref", "-d", &reference])?;
        }
        return Ok(());
    }
    let tree = run_index(path, &temporary, &["write-tree"])?;
    run(repo, &["update-ref", &reference, &tree])?;
    Ok(())
}

/// Restores configured copies without adding them to the new worktree's
/// index. A tracked file supplied by Git still wins over a preserved copy.
fn restore_copy_files(project: &Project, lane: &Lane, path: &Path) -> anyhow::Result<()> {
    let repo = Path::new(&project.repo);
    let reference = copy_files_reference(lane);
    if !ref_exists(repo, &reference)? {
        return Ok(());
    }
    let entries = run(repo, &["ls-tree", "-r", "-z", "--name-only", &reference])?;
    let entries = entries.split('\0').collect::<Vec<_>>();
    for relative in &project.settings.copy_files {
        if !entries.contains(&relative.as_str()) {
            continue;
        }
        let target = path.join(relative);
        if target.exists() {
            continue;
        }
        let object = format!("{reference}:{relative}");
        let output = git(repo, &["show", &object])?;
        if !output.status.success() {
            return Err(command_failed("git show", &output));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, output.stdout)
            .with_context(|| format!("Could not restore {}", target.display()))?;
    }
    Ok(())
}

fn copy_files(project: &Project, destination: &Path) -> anyhow::Result<()> {
    let repo = Path::new(&project.repo);
    for relative in &project.settings.copy_files {
        let source = repo.join(relative);
        let target = destination.join(relative);
        match std::fs::metadata(&source) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => bail!("{} is not a file", source.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Could not read {}", source.display()));
            }
        }
        // A tracked file already supplied by Git wins over the local copy.
        if target.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target).with_context(|| {
            format!(
                "Could not copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(())
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
    /// Move it back to this commit (the tip before a restore advanced it).
    Reset(String),
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
            BranchCleanup::Reset(commit) => {
                let _ = run(&self.repo, &["update-ref", &full_ref, commit]);
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
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["commit-tree", &tree, "-p", "HEAD", "-m"])
        .arg(format!("Panoptes checkpoint for {}", lane.name))
        .env("GIT_AUTHOR_NAME", "Panoptes")
        .env("GIT_AUTHOR_EMAIL", "panoptes@localhost")
        .env("GIT_COMMITTER_NAME", "Panoptes")
        .env("GIT_COMMITTER_EMAIL", "panoptes@localhost")
        .output()
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
fn snapshot_tree(path: &Path, lane: &Lane) -> anyhow::Result<String> {
    snapshot_tree_as(path, &format!("archive-{}", lane.id))
}

/// [`snapshot_tree`] through an index copy named for its purpose. A counter
/// keeps concurrent snapshots of one worktree from sharing the copy.
fn snapshot_tree_as(path: &Path, purpose: &str) -> anyhow::Result<String> {
    static SNAPSHOTS: AtomicU64 = AtomicU64::new(0);
    let index = git_path(path, "index")?;
    let temporary = index.with_file_name(format!(
        "panoptes-{purpose}-{}-{}",
        std::process::id(),
        SNAPSHOTS.fetch_add(1, Ordering::Relaxed)
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

fn git_path(cwd: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(run(cwd, &["rev-parse", "--git-path", name])?);
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn ref_exists(repo: &Path, reference: &str) -> anyhow::Result<bool> {
    let output = git(repo, &["show-ref", "--verify", "--quiet", reference])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(command_failed("git show-ref", &output)),
    }
}

fn run(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    checked_stdout("git", git(cwd, args)?)
}

fn run_index(cwd: &Path, index: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .output()
        .with_context(|| format!("Could not run git {}", args.join(" ")))?;
    checked_stdout("git", output)
}

fn git(cwd: &Path, args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("Could not run git {}", args.join(" ")))
}

fn checked_stdout(description: &str, output: Output) -> anyhow::Result<String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(command_failed(description, &output))
    }
}

fn command_failed(description: &str, output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    anyhow::anyhow!("{description} failed ({}): {detail}", output.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LaneState, ProjectSettings};

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        repo: PathBuf,
        project: Project,
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn fixture(setup: Option<&str>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("tracked.txt"), "start\n").unwrap();
        std::fs::write(repo.join(".gitignore"), ".env\nscratch.log\n").unwrap();
        std::fs::write(repo.join(".env"), "secret\n").unwrap();
        git(&repo, &["add", "tracked.txt", ".gitignore"]);
        git(&repo, &["commit", "--quiet", "-m", "start"]);
        let project = Project {
            id: 7,
            repo: repo.to_string_lossy().into_owned(),
            settings: ProjectSettings {
                name: "App".into(),
                base_branch: "trunk".into(),
                copy_files: vec![".env".into(), "missing".into()],
                setup: setup.map(Into::into),
            },
        };
        Fixture {
            root: root.join("lanes"),
            _dir: dir,
            repo,
            project,
        }
    }

    fn lane(f: &Fixture, id: i64, new: &NewLane) -> Lane {
        Lane {
            id,
            project_id: f.project.id,
            name: new.name.clone(),
            task: new.task.clone(),
            branch: new.branch.clone(),
            state: LaneState::Active,
            worktrees: new
                .worktrees
                .iter()
                .map(|worktree| LaneWorktree {
                    project_id: worktree.project_id,
                    path: worktree.path.clone(),
                    main_checkout: worktree.main_checkout,
                    checkpoint_ref: None,
                })
                .collect(),
        }
    }

    fn create_lane(f: &Fixture) -> NewLane {
        create(
            &f.root,
            &f.project,
            "fix-login",
            &[],
            "Fix login",
            false,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn nested_repository(path: &Path) {
        std::fs::create_dir(path).unwrap();
        git(path, &["init", "--quiet", "--initial-branch", "trunk"]);
        std::fs::write(path.join("inner.txt"), "inner work\n").unwrap();
        git(path, &["add", "inner.txt"]);
        git(path, &["commit", "--quiet", "-m", "inner work"]);
    }

    #[test]
    fn names_are_slugs() {
        assert_eq!(slug("  Fix the login, please! "), "fix-the-login-please");
        assert_eq!(slug("..."), "lane");
        let long = format!("{} b", "a".repeat(47));
        assert_eq!(slug(&long), "a".repeat(47));
    }

    #[test]
    fn names_skip_folders_and_branches_already_in_use() {
        let f = fixture(None);
        std::fs::create_dir_all(f.root.join("fix-login")).unwrap();
        git(&f.repo, &["branch", "panoptes/fix-login-2"]);
        let new = create(
            &f.root,
            &f.project,
            "fix-login",
            &["fix-login-3".into()],
            "Fix login",
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(new.name, "fix-login-4");
        assert_eq!(new.branch, "panoptes/fix-login-4");
    }

    #[test]
    fn setup_with_lots_of_output_finishes() {
        // Far more than a pipe buffer holds.
        let f = fixture(Some(
            "head -c 1000000 /dev/zero | tr '\\0' x; printf ok > done.txt",
        ));
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        assert!(path.join("done.txt").exists());
    }

    #[test]
    fn a_lane_whose_folder_was_deleted_can_still_be_archived() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::remove_dir_all(&path).unwrap();
        let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
        assert!(matches!(result, ArchiveResult::Archived { .. }));
        assert!(!git(&f.repo, &["worktree", "list"]).contains("fix-login"));
    }

    #[test]
    fn a_clean_archive_can_be_restored_on_its_branch() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 1, &new);
        archive(&f.project, &lane, None).unwrap();
        lane.state = LaneState::Archived;
        restore(&f.project, &lane).unwrap();
        let path = PathBuf::from(&new.worktrees[0].path);
        assert_eq!(
            git(&path, &["branch", "--show-current"]),
            "panoptes/fix-login"
        );
    }

    #[test]
    fn archive_restores_lane_copy_files_for_every_strategy() {
        for strategy in [
            None,
            Some(ArchiveStrategy::Commit),
            Some(ArchiveStrategy::Stash),
            Some(ArchiveStrategy::Checkpoint),
        ] {
            let f = fixture(None);
            let new = create_lane(&f);
            let mut lane = lane(&f, 1, &new);
            let path = PathBuf::from(&new.worktrees[0].path);
            std::fs::write(path.join(".env"), "lane secret\n").unwrap();
            if strategy.is_some() {
                std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
            }

            let ArchiveResult::Archived { checkpoint_ref } =
                archive(&f.project, &lane, strategy).unwrap()
            else {
                panic!("archive should finish")
            };
            lane.worktrees[0].checkpoint_ref = checkpoint_ref;
            lane.state = LaneState::Archived;
            std::fs::write(f.repo.join(".env"), "replacement secret\n").unwrap();
            restore(&f.project, &lane).unwrap();

            assert_eq!(
                std::fs::read_to_string(path.join(".env")).unwrap(),
                "lane secret\n",
                "{strategy:?}"
            );
            assert!(
                ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/copy-files").unwrap(),
                "{strategy:?}"
            );
            assert_eq!(git(&path, &["status", "--porcelain"]), "", "{strategy:?}");
        }
    }

    #[test]
    fn unconfigured_ignored_files_are_not_preserved() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("scratch.log"), "temporary\n").unwrap();
        archive(&f.project, &lane, None).unwrap();
        lane.state = LaneState::Archived;

        restore(&f.project, &lane).unwrap();

        assert!(!path.join("scratch.log").exists());
    }

    #[test]
    fn restore_refuses_a_branch_that_moved_and_leaves_it_alone() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 42, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        let ArchiveResult::Archived { checkpoint_ref } =
            archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
        else {
            panic!("checkpoint archive should finish")
        };
        lane.worktrees[0].checkpoint_ref = checkpoint_ref;
        lane.state = LaneState::Archived;
        let elsewhere = git(
            &f.repo,
            &[
                "commit-tree",
                "-p",
                "panoptes/fix-login",
                "-m",
                "later",
                "HEAD^{tree}",
            ],
        );
        git(
            &f.repo,
            &["update-ref", "refs/heads/panoptes/fix-login", &elsewhere],
        );

        let error = restore(&f.project, &lane).unwrap_err();
        assert!(format!("{error:#}").contains("new commits"), "{error:#}");
        assert_eq!(
            git(&f.repo, &["rev-parse", "panoptes/fix-login"]),
            elsewhere
        );
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_restore_puts_the_branch_back() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 42, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        let tip = git(&f.repo, &["rev-parse", "panoptes/fix-login"]);
        let ArchiveResult::Archived { checkpoint_ref } =
            archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
        else {
            panic!("checkpoint archive should finish")
        };
        lane.worktrees[0].checkpoint_ref = checkpoint_ref;
        lane.state = LaneState::Archived;
        let mut project = f.project.clone();
        project.settings.setup = Some("exit 3".into());

        assert!(restore(&project, &lane).is_err());
        assert_eq!(git(&f.repo, &["rev-parse", "panoptes/fix-login"]), tip);
        assert!(!path.exists());
    }

    #[test]
    fn creation_branches_copies_files_and_runs_setup() {
        let f = fixture(Some("printf ready > setup.txt"));
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        assert_eq!(new.branch, "panoptes/fix-login");
        assert_eq!(path, f.root.join("fix-login/app"));
        assert_eq!(
            std::fs::read_to_string(path.join(".env")).unwrap(),
            "secret\n"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("setup.txt")).unwrap(),
            "ready"
        );
        assert_eq!(git(&path, &["branch", "--show-current"]), new.branch);
        assert_eq!(git(&path, &["status", "--porcelain"]), "?? setup.txt");
    }

    #[test]
    fn failed_setup_removes_the_worktree_and_new_branch() {
        let f = fixture(Some("printf partial > setup.txt; exit 7"));
        let error = create(
            &f.root,
            &f.project,
            "fix-login",
            &[],
            "Fix login",
            false,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("project setup failed"),
            "{error:#}"
        );
        assert!(!f.root.join("fix-login").exists());
        assert!(!ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
    }

    #[test]
    fn main_checkout_is_an_opt_in_without_disk_changes() {
        let f = fixture(Some("exit 99"));
        let new = create(
            &f.root,
            &f.project,
            "quick",
            &[],
            "Quick question",
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(new.worktrees[0].path, f.project.repo);
        assert!(new.worktrees[0].main_checkout);
        assert_eq!(new.branch, "trunk");
        assert!(!f.root.exists());
    }

    #[test]
    fn non_forced_removal_preserves_a_worktree_that_became_dirty() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "new work\n").unwrap();

        remove_worktree(&f.repo, &path, false).unwrap_err();

        assert_eq!(
            std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
            "new work\n"
        );
    }

    #[test]
    fn archiving_a_clean_lane_keeps_its_branch() {
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 1, &new);
        let result = archive(&f.project, &lane, None).unwrap();
        assert!(matches!(
            result,
            ArchiveResult::Archived {
                checkpoint_ref: None
            }
        ));
        assert!(!PathBuf::from(&new.worktrees[0].path).exists());
        assert!(ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
    }

    #[test]
    fn deleting_a_lane_removes_its_dirty_worktree_branch_and_recovery_refs() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "checkpoint work\n").unwrap();
        let ArchiveResult::Archived { checkpoint_ref } =
            archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap()
        else {
            panic!("checkpoint archive should finish")
        };
        lane.state = LaneState::Archived;
        lane.worktrees[0].checkpoint_ref = checkpoint_ref;
        restore(&f.project, &lane).unwrap();
        std::fs::write(path.join("uncommitted.txt"), "delete me\n").unwrap();

        delete(&f.project, &lane).unwrap();

        assert!(!path.exists());
        assert!(!ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
        assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/archive").unwrap());
        assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/copy-files").unwrap());
        assert_eq!(git(&f.repo, &["branch", "--show-current"]), "trunk");
    }

    #[test]
    fn deleting_a_main_checkout_lane_leaves_the_checkout_and_branch_alone() {
        let f = fixture(None);
        let new = create(
            &f.root,
            &f.project,
            "quick",
            &[],
            "Quick question",
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        let lane = lane(&f, 1, &new);
        std::fs::write(f.repo.join("uncommitted.txt"), "keep me").unwrap();

        delete(&f.project, &lane).unwrap();

        assert_eq!(
            std::fs::read_to_string(f.repo.join("uncommitted.txt")).unwrap(),
            "keep me"
        );
        assert!(ref_exists(&f.repo, "refs/heads/trunk").unwrap());
    }

    #[test]
    fn deleting_a_lane_on_another_branch_leaves_everything_in_place() {
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        git(&path, &["checkout", "--quiet", "-b", "somewhere-else"]);

        let error = delete(&f.project, &lane).unwrap_err();

        assert!(format!("{error:#}").contains("somewhere-else"), "{error:#}");
        assert!(path.exists());
        assert!(ref_exists(&f.repo, "refs/heads/panoptes/fix-login").unwrap());
        assert!(ref_exists(&f.repo, "refs/heads/somewhere-else").unwrap());
    }

    #[test]
    fn untracked_files_count_as_changes_whatever_the_status_settings() {
        let f = fixture(None);
        git(&f.repo, &["config", "status.showUntrackedFiles", "no"]);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("agent-wrote.txt"), "work\n").unwrap();
        std::fs::create_dir(path.join("nested")).unwrap();
        std::fs::write(path.join("nested/deep.txt"), "work\n").unwrap();

        let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
        assert!(matches!(result, ArchiveResult::Dirty));
        assert!(path.join("agent-wrote.txt").exists());
        assert!(path.join("nested/deep.txt").exists());
    }

    #[test]
    fn worktree_status_sees_submodule_changes_hidden_by_config() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        let source = f.repo.parent().unwrap().join("submodule-source");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "--quiet", "--initial-branch", "trunk"]);
        std::fs::write(source.join("tracked.txt"), "start\n").unwrap();
        git(&source, &["add", "tracked.txt"]);
        git(&source, &["commit", "--quiet", "-m", "start"]);
        git(
            &path,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                &source.to_string_lossy(),
                "dependency",
            ],
        );
        git(&path, &["commit", "--quiet", "-m", "add dependency"]);
        let dependency = path.join("dependency");
        std::fs::write(dependency.join("new.txt"), "new\n").unwrap();
        git(&dependency, &["add", "new.txt"]);
        git(&dependency, &["commit", "--quiet", "-m", "new commit"]);
        git(&path, &["config", "diff.ignoreSubmodules", "all"]);
        assert_eq!(git(&path, &["status", "--porcelain"]), "");

        assert!(!worktree_status(&path).unwrap().is_empty());
    }

    #[test]
    fn a_detached_head_is_refused_so_its_commits_are_kept() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        git(&path, &["checkout", "--quiet", "--detach"]);
        std::fs::write(path.join("tracked.txt"), "detached work\n").unwrap();
        git(&path, &["commit", "--quiet", "-am", "detached work"]);
        let commit = git(&path, &["rev-parse", "HEAD"]);

        for strategy in [None, Some(ArchiveStrategy::Commit)] {
            let error = archive(&f.project, &lane(&f, 1, &new), strategy).unwrap_err();
            assert!(format!("{error:#}").contains("detached HEAD"), "{error:#}");
        }
        assert!(path.exists());
        assert_eq!(git(&path, &["rev-parse", "HEAD"]), commit);
    }

    #[test]
    fn another_branch_is_refused_so_its_commits_are_kept() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        git(&path, &["checkout", "--quiet", "-b", "side"]);
        std::fs::write(path.join("tracked.txt"), "side work\n").unwrap();
        git(&path, &["commit", "--quiet", "-am", "side work"]);

        let error = archive(&f.project, &lane(&f, 1, &new), None).unwrap_err();
        assert!(format!("{error:#}").contains("is on side instead of panoptes/fix-login"));
        assert!(path.exists());
    }

    #[test]
    fn an_operation_in_progress_is_refused() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        let head = git(&path, &["rev-parse", "HEAD"]);
        std::fs::write(git_path(&path, "MERGE_HEAD").unwrap(), format!("{head}\n")).unwrap();

        let error = archive(&f.project, &lane(&f, 1, &new), None).unwrap_err();
        assert!(format!("{error:#}").contains("in the middle of a merge"));
        assert!(path.exists());
    }

    #[test]
    fn a_dirty_archive_without_a_choice_does_nothing() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        let before = git(&path, &["status", "--porcelain"]);

        let result = archive(&f.project, &lane(&f, 1, &new), None).unwrap();
        assert!(matches!(result, ArchiveResult::Dirty));
        assert!(path.exists());
        assert_eq!(git(&path, &["status", "--porcelain"]), before);
    }

    #[test]
    fn commit_archive_refuses_a_nested_repository() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        nested_repository(&path.join("inner"));

        let error = archive(
            &f.project,
            &lane(&f, 1, &new),
            Some(ArchiveStrategy::Commit),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("inner"));
        assert!(path.join("inner/.git").exists());
        assert_eq!(
            std::fs::read_to_string(path.join("inner/inner.txt")).unwrap(),
            "inner work\n"
        );
        assert_eq!(
            git(&f.repo, &["ls-tree", "panoptes/fix-login", "inner"]),
            ""
        );
    }

    #[test]
    fn checkpoint_archive_refuses_a_nested_repository() {
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        nested_repository(&path.join("inner"));

        let error = archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap_err();

        assert!(format!("{error:#}").contains("inner"));
        assert!(path.join("inner/.git").exists());
        assert_eq!(
            std::fs::read_to_string(path.join("inner/inner.txt")).unwrap(),
            "inner work\n"
        );
        assert!(!ref_exists(&f.repo, "refs/panoptes/lanes/fix-login/archive").unwrap());
    }

    #[test]
    fn snapshots_allow_a_gitlink_already_in_head() {
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        let source = f.repo.parent().unwrap().join("submodule-source");
        nested_repository(&source);
        git(
            &path,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                &source.to_string_lossy(),
                "dependency",
            ],
        );
        git(&path, &["commit", "--quiet", "-m", "add dependency"]);

        let tree = snapshot_tree(&path, &lane).unwrap();

        assert!(git(&path, &["cat-file", "-e", &tree]).is_empty());
        assert!(git(&path, &["ls-tree", &tree, "dependency"]).starts_with("160000 "));
    }

    #[test]
    fn commit_archives_all_changes_on_the_lane_branch() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        std::fs::write(path.join("new.txt"), "new\n").unwrap();

        let result = archive(
            &f.project,
            &lane(&f, 1, &new),
            Some(ArchiveStrategy::Commit),
        )
        .unwrap();
        assert!(matches!(result, ArchiveResult::Archived { .. }));
        assert!(!path.exists());
        assert_eq!(
            git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
            "work"
        );
        assert_eq!(git(&f.repo, &["show", "panoptes/fix-login:new.txt"]), "new");
    }

    #[test]
    fn new_changes_from_a_commit_hook_keep_the_worktree_in_place() {
        use std::os::unix::fs::PermissionsExt;

        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        git(&path, &["config", "status.showUntrackedFiles", "no"]);
        let hook = f.repo.join(".git/hooks/post-commit");
        std::fs::write(&hook, "#!/bin/sh\ntouch from-hook.txt\n").unwrap();
        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).unwrap();

        let error = archive(
            &f.project,
            &lane(&f, 1, &new),
            Some(ArchiveStrategy::Commit),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("worktree was left in place"));
        assert!(path.join("from-hook.txt").exists());
    }

    #[test]
    fn stash_archives_changes_without_committing_them() {
        let f = fixture(None);
        let new = create_lane(&f);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        std::fs::write(path.join("new.txt"), "new\n").unwrap();

        let result = archive(&f.project, &lane(&f, 1, &new), Some(ArchiveStrategy::Stash)).unwrap();
        assert!(matches!(result, ArchiveResult::Archived { .. }));
        assert!(!path.exists());
        assert_eq!(
            git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
            "start"
        );
        assert!(git(&f.repo, &["stash", "list"]).contains("Panoptes lane fix-login"));
        let stash = git(
            &f.repo,
            &["stash", "show", "--include-untracked", "--name-only"],
        );
        assert!(stash.contains("new.txt"), "{stash}");
    }

    #[test]
    fn a_checkpoint_preserves_the_real_index_head_and_branch() {
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 42, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "staged\n").unwrap();
        git(&path, &["add", "tracked.txt"]);
        std::fs::write(path.join("new.txt"), "untracked\n").unwrap();
        let status = git(&path, &["status", "--porcelain"]);
        let head = git(&path, &["rev-parse", "HEAD"]);

        let (reference, _) = checkpoint(&f.project, &lane, &path).unwrap();
        assert_eq!(reference, "refs/panoptes/lanes/fix-login/archive");
        assert_eq!(git(&path, &["status", "--porcelain"]), status);
        assert_eq!(git(&path, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            git(&f.repo, &["show", "panoptes/fix-login:tracked.txt"]),
            "start"
        );
        assert_eq!(
            git(&f.repo, &["show", &format!("{reference}:tracked.txt")]),
            "staged"
        );
        assert_eq!(
            git(&f.repo, &["show", &format!("{reference}:new.txt")]),
            "untracked"
        );
    }

    #[test]
    fn snapshots_see_a_second_edit_to_an_already_modified_file() {
        // `git status` reads ` M tracked.txt` both times; the tree does not.
        let f = fixture(None);
        let new = create_lane(&f);
        let lane = lane(&f, 1, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "first\n").unwrap();
        let before = snapshot_tree(&path, &lane).unwrap();
        let status = git(&path, &["status", "--porcelain"]);
        std::fs::write(path.join("tracked.txt"), "second\n").unwrap();
        assert_eq!(git(&path, &["status", "--porcelain"]), status);
        assert_ne!(snapshot_tree(&path, &lane).unwrap(), before);
    }

    #[test]
    fn a_checkpoint_archive_can_be_restored_onto_its_branch() {
        let f = fixture(None);
        let new = create_lane(&f);
        let mut lane = lane(&f, 42, &new);
        let path = PathBuf::from(&new.worktrees[0].path);
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        std::fs::write(path.join("new.txt"), "new\n").unwrap();

        let result = archive(&f.project, &lane, Some(ArchiveStrategy::Checkpoint)).unwrap();
        let ArchiveResult::Archived { checkpoint_ref } = result else {
            panic!("checkpoint archive should finish")
        };
        lane.worktrees[0].checkpoint_ref = checkpoint_ref;
        lane.state = LaneState::Archived;
        assert!(!path.exists());

        restore(&f.project, &lane).unwrap();
        assert_eq!(
            std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
            "work\n"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("new.txt")).unwrap(),
            "new\n"
        );
        assert_eq!(
            git(&path, &["branch", "--show-current"]),
            "panoptes/fix-login"
        );
        assert_eq!(git(&path, &["status", "--porcelain"]), "");
    }
}
