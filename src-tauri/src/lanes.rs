//! Lane worktrees on disk. The service owns persistence and calls these
//! functions on a blocking thread; this module owns Git, rollback, and the
//! archive choices that keep uncommitted work recoverable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;

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
    let slug = slug.trim_matches('-').chars().take(48).collect::<String>();
    if slug.is_empty() { "lane".into() } else { slug }
}

/// Makes a worktree and its branch, copies the project's local files, and
/// runs setup. Any failure removes everything this call created. Missing copy
/// sources (the usual case for `.env`) are simply absent in the new lane.
pub fn create(
    root: &Path,
    project: &Project,
    name: &str,
    task: &str,
    main_checkout: bool,
    cancelled: &AtomicBool,
) -> anyhow::Result<NewLane> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("Lane creation cancelled");
    }
    if main_checkout {
        return Ok(NewLane {
            project_id: project.id,
            name: name.into(),
            task: task.into(),
            branch: current_branch(&PathBuf::from(&project.repo))
                .unwrap_or_else(|| project.settings.base_branch.clone()),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: project.repo.clone(),
                main_checkout: true,
            }],
        });
    }

    let repo = PathBuf::from(&project.repo);
    let branch = format!("panoptes/{name}");
    let path = root.join(name).join(
        repo.file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("repo")),
    );
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let full_ref = format!("refs/heads/{branch}");
    if ref_exists(&repo, &full_ref)? {
        bail!("The branch {branch} already exists");
    }
    std::fs::create_dir_all(path.parent().context("A lane needs a parent folder")?)
        .with_context(|| format!("Could not create the folder for {name}"))?;

    let mut cleanup = CreationCleanup {
        repo: repo.clone(),
        path: path.clone(),
        branch: branch.clone(),
        delete_branch: true,
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
        name: name.into(),
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
    let before = worktree_status(&path)?;
    let is_dirty = !before.is_empty();
    if is_dirty && strategy.is_none() {
        return Ok(ArchiveResult::Dirty);
    }

    let mut checkpoint_ref = None;
    if is_dirty {
        match strategy {
            Some(ArchiveStrategy::Commit) => {
                run(&path, &["add", "-A"])?;
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
                let reference = checkpoint(project, lane, &path)?;
                checkpoint_ref = Some(reference);
            }
            None => {}
        }
        let after = worktree_status(&path)?;
        match strategy {
            Some(ArchiveStrategy::Commit | ArchiveStrategy::Stash) if !after.is_empty() => {
                bail!("Changes remain after archiving; the worktree was left in place")
            }
            Some(ArchiveStrategy::Checkpoint) if after != before => {
                bail!(
                    "The tree changed while its checkpoint was being taken; the worktree was left in place"
                )
            }
            _ => {}
        }
    }
    // A clean tree can also change between the first check and removal.
    if !is_dirty && !worktree_status(&path)?.is_empty() {
        return Ok(ArchiveResult::Dirty);
    }
    remove_worktree(Path::new(&project.repo), &path)?;
    Ok(ArchiveResult::Archived { checkpoint_ref })
}

/// Restores an archived checkpoint onto its original lane branch. The final
/// checkpoint has the old branch tip as its parent, so advancing the branch
/// and checking it out recovers both tracked and untracked captured files.
pub fn restore(project: &Project, lane: &Lane) -> anyhow::Result<()> {
    let worktree = sole_worktree(lane)?;
    if worktree.main_checkout {
        return Ok(());
    }
    let reference = worktree
        .checkpoint_ref
        .as_deref()
        .context("Only a lane archived as a checkpoint can be restored with its changes")?;
    let path = PathBuf::from(&worktree.path);
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let repo = Path::new(&project.repo);
    run(repo, &["branch", "-f", &lane.branch, reference])?;
    let mut cleanup = CreationCleanup {
        repo: repo.to_path_buf(),
        path: path.clone(),
        branch: lane.branch.clone(),
        delete_branch: false,
        armed: true,
    };
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
    copy_files(project, &path)?;
    run_setup(project, &path, None)?;
    cleanup.armed = false;
    Ok(())
}

fn sole_worktree(lane: &Lane) -> anyhow::Result<&LaneWorktree> {
    if lane.worktrees.len() != 1 {
        bail!("This operation needs exactly one worktree");
    }
    Ok(&lane.worktrees[0])
}

fn worktree_status(path: &Path) -> anyhow::Result<String> {
    run(path, &["status", "--porcelain"])
}

fn current_branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()
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
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(setup)
        .current_dir(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            if output.status.success() {
                return Ok(());
            }
            return Err(command_failed("project setup", &output));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
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

fn remove_worktree(repo: &Path, path: &Path) -> anyhow::Result<()> {
    run(
        repo,
        &["worktree", "remove", "--force", &path.to_string_lossy()],
    )?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

struct CreationCleanup {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
    delete_branch: bool,
    armed: bool,
}

impl Drop for CreationCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = remove_worktree(&self.repo, &self.path);
        // `git worktree add` can fail after creating the destination but
        // before registering it. This path was required not to exist before
        // the attempt, so removing whatever the attempt left is safe.
        let _ = std::fs::remove_dir_all(&self.path);
        if self.delete_branch
            && ref_exists(&self.repo, &format!("refs/heads/{}", self.branch)).unwrap_or(false)
        {
            let _ = run(&self.repo, &["branch", "-D", &self.branch]);
        }
    }
}

fn checkpoint(project: &Project, lane: &Lane, path: &Path) -> anyhow::Result<String> {
    let index = git_path(path, "index")?;
    let temporary = index.with_file_name(format!(
        "panoptes-archive-{}-{}",
        std::process::id(),
        lane.id
    ));
    let _temporary = PrivateIndex(temporary.clone());
    match std::fs::copy(&index, &temporary) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            run_index(path, &temporary, &["read-tree", "HEAD"])?;
        }
        Err(error) => return Err(error).context("Could not copy the worktree index"),
    }
    run_index(path, &temporary, &["add", "-A"])?;
    let tree = run_index(path, &temporary, &["write-tree"])?;
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["commit-tree", &tree, "-p", "HEAD", "-m"])
        .arg(format!("Panoptes checkpoint for {}", lane.name))
        .env("GIT_INDEX_FILE", &temporary)
        .env("GIT_AUTHOR_NAME", "Panoptes")
        .env("GIT_AUTHOR_EMAIL", "panoptes@localhost")
        .env("GIT_COMMITTER_NAME", "Panoptes")
        .env("GIT_COMMITTER_EMAIL", "panoptes@localhost")
        .output()
        .context("Could not write the lane checkpoint")?;
    let commit = checked_stdout("git commit-tree", output)?;
    let reference = format!("refs/panoptes/lanes/{}/archive", lane.id);
    run(
        Path::new(&project.repo),
        &["update-ref", &reference, &commit],
    )?;
    Ok(reference)
}

struct PrivateIndex(PathBuf);

impl Drop for PrivateIndex {
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
    use crate::store::{LaneState, ProjectSettings, WriteLock};

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
        std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
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
            write_lock: WriteLock::default(),
        }
    }

    fn create_lane(f: &Fixture) -> NewLane {
        create(
            &f.root,
            &f.project,
            "fix-login",
            "Fix login",
            false,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn names_are_slugs() {
        assert_eq!(slug("  Fix the login, please! "), "fix-the-login-please");
        assert_eq!(slug("..."), "lane");
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

        let reference = checkpoint(&f.project, &lane, &path).unwrap();
        assert_eq!(reference, "refs/panoptes/lanes/42/archive");
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
