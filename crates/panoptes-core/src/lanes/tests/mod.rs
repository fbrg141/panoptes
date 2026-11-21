use super::archive::remove_worktree;
use super::checkpoint::checkpoint;
use super::inspect::worktree_status;
use super::*;
use crate::git::{git_path, ref_exists};
use crate::store::{Lane, LaneState, LaneWorktree, NewLane, Project, ProjectSettings};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

mod archive;
mod create;
mod delete;
mod detached;
mod restore;
mod work;

// The service always archives with recovery and deletes with an explicit
// detached-HEAD choice; these are the plain calls the tests make.

/// Archives only after inspecting the current tree. A dirty tree with no
/// strategy is left completely untouched so the UI can offer the three
/// recovery choices. The branch itself is never deleted.
pub(super) fn archive(
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
pub(super) fn archive_recording(
    project: &Project,
    lane: &Lane,
    strategy: Option<ArchiveStrategy>,
    record: &dyn Fn(Option<&str>) -> anyhow::Result<()>,
) -> anyhow::Result<ArchiveResult> {
    archive_recovering(project, lane, strategy, BranchRecovery::None, record)
}

/// Permanently removes a lane's worktree, local branch and recovery refs.
/// The caller has already obtained explicit confirmation. A main-checkout
/// lane owns none of those things, so deletion only removes its store row.
pub(super) fn delete(project: &Project, lane: &Lane) -> anyhow::Result<DeleteResult> {
    delete_confirmed(project, lane, false)
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    project: Project,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = crate::git::command()
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
