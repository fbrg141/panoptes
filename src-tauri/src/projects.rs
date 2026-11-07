//! Projects on disk: which git repository a folder belongs to, the settings
//! a newly registered repository starts with, and the rules settings must
//! follow. The service stores what this decides.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::store::ProjectSettings;

/// Untracked files copied into new worktrees unless the project says
/// otherwise.
pub const DEFAULT_COPY_FILES: &[&str] = &[".env"];

/// Where a working directory sits in git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The enclosing repository or worktree root, else the folder itself.
    pub root: String,
    /// The main checkout of that repository, shared by all its worktrees.
    /// `None` outside git.
    pub checkout: Option<PathBuf>,
}

/// Finds the repository `cwd` is in without running git: the nearest `.git`
/// above it, followed through a linked worktree's `.git` file to the main
/// checkout. Paths are made canonical where they exist, so they compare
/// equal to a registered project's.
pub fn locate(cwd: &Path) -> Location {
    let Some(root) = cwd.ancestors().find(|dir| dir.join(".git").exists()) else {
        return Location {
            root: cwd.to_string_lossy().into_owned(),
            checkout: None,
        };
    };
    Location {
        root: root.to_string_lossy().into_owned(),
        checkout: Some(canonical(&main_checkout(root))),
    }
}

/// `root` holds a `.git`. A folder means it is the main checkout. A file
/// points at `<main>/.git/worktrees/<name>` for a linked worktree, whose
/// `commondir` leads back to `<main>/.git`; anything else (a submodule) is
/// its own checkout.
fn main_checkout(root: &Path) -> PathBuf {
    let dot_git = root.join(".git");
    let Ok(text) = std::fs::read_to_string(&dot_git) else {
        return root.to_path_buf();
    };
    let Some(git_dir) = text.trim().strip_prefix("gitdir:") else {
        return root.to_path_buf();
    };
    let git_dir = root.join(git_dir.trim());
    let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) else {
        return root.to_path_buf();
    };
    let common = canonical(&git_dir.join(common.trim()));
    match (common.file_name(), common.parent()) {
        (Some(name), Some(parent)) if name == ".git" => parent.to_path_buf(),
        _ => root.to_path_buf(),
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The main checkout to register for a folder the user picked, which may be
/// anywhere inside the repository or one of its worktrees.
pub fn repository_of(folder: &Path) -> anyhow::Result<PathBuf> {
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("Could not open {}", folder.display()))?;
    match locate(&folder).checkout {
        Some(checkout) => Ok(checkout),
        None => bail!("{} is not in a git repository", folder.display()),
    }
}

/// What a newly registered repository starts with: its folder's name, its
/// default branch, and the default files to copy.
pub fn default_settings(repo: &Path) -> ProjectSettings {
    ProjectSettings {
        name: repo
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo.to_string_lossy().into_owned()),
        base_branch: default_branch(repo),
        copy_files: DEFAULT_COPY_FILES.iter().map(|f| f.to_string()).collect(),
        setup: None,
    }
}

/// The branch `origin`'s HEAD points at, else the branch checked out, else
/// `main`.
pub fn default_branch(repo: &Path) -> String {
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        (output.status.success() && !text.is_empty()).then(|| text.to_string())
    };
    if let Some(remote) = git(&[
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ]) && let Some(branch) = remote.strip_prefix("origin/")
    {
        return branch.to_string();
    }
    git(&["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_else(|| "main".to_string())
}

/// `settings` tidied (trimmed, empty setup dropped, duplicate files
/// removed), or why they can't be saved.
pub fn validate(settings: ProjectSettings) -> anyhow::Result<ProjectSettings> {
    let name = settings.name.trim().to_string();
    if name.is_empty() {
        bail!("A project needs a name");
    }
    let base_branch = settings.base_branch.trim().to_string();
    if base_branch.is_empty() {
        bail!("A project needs a base branch");
    }
    let mut copy_files: Vec<String> = Vec::new();
    for file in settings.copy_files {
        let file = file.trim();
        if file.is_empty() {
            continue;
        }
        let inside = Path::new(file)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
        if !inside {
            bail!("{file} is not a path inside the repository");
        }
        if !copy_files.iter().any(|f| f == file) {
            copy_files.push(file.to_string());
        }
    }
    let setup = settings
        .setup
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(ProjectSettings {
        name,
        base_branch,
        copy_files,
        setup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with one commit on `trunk`.
    fn repo(dir: &Path) -> PathBuf {
        let repo = dir.join("app");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        canonical(&repo)
    }

    #[test]
    fn a_folder_inside_a_repository_belongs_to_its_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        let location = locate(&repo.join("src"));
        assert_eq!(location.root, repo.to_string_lossy());
        assert_eq!(location.checkout, Some(repo.clone()));
        assert_eq!(repository_of(&repo.join("src")).unwrap(), repo);
    }

    #[test]
    fn a_linked_worktree_belongs_to_the_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        let worktree = dir.path().join("workspaces/fix/app");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "fix",
                worktree.to_str().unwrap(),
            ],
        );
        let location = locate(&worktree);
        assert_eq!(location.root, worktree.to_string_lossy());
        assert_eq!(location.checkout, Some(repo.clone()));
        assert_eq!(repository_of(&worktree).unwrap(), repo);
    }

    #[test]
    fn a_folder_outside_git_has_no_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let location = locate(dir.path());
        assert_eq!(location.root, dir.path().to_string_lossy());
        assert_eq!(location.checkout, None);
        let error = repository_of(dir.path()).unwrap_err().to_string();
        assert!(error.contains("is not in a git repository"), "{error}");
    }

    #[test]
    fn a_new_project_takes_the_folder_name_and_checked_out_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        let settings = default_settings(&repo);
        assert_eq!(settings.name, "app");
        assert_eq!(settings.base_branch, "trunk");
        assert_eq!(settings.copy_files, [".env"]);
        assert_eq!(settings.setup, None);
    }

    #[test]
    fn the_default_branch_follows_origin_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        git(&repo, &["branch", "release"]);
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/release", "HEAD"],
        );
        git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/release",
            ],
        );
        assert_eq!(default_branch(&repo), "release");
    }

    fn settings(copy_files: &[&str], setup: Option<&str>) -> ProjectSettings {
        ProjectSettings {
            name: " App ".into(),
            base_branch: " main ".into(),
            copy_files: copy_files.iter().map(|f| f.to_string()).collect(),
            setup: setup.map(Into::into),
        }
    }

    #[test]
    fn settings_are_tidied() {
        let tidy = validate(settings(
            &[".env", " ", ".env", "config/local.toml"],
            Some("  "),
        ))
        .unwrap();
        assert_eq!(tidy.name, "App");
        assert_eq!(tidy.base_branch, "main");
        assert_eq!(tidy.copy_files, [".env", "config/local.toml"]);
        assert_eq!(tidy.setup, None);
    }

    #[test]
    fn files_outside_the_repository_are_refused() {
        for file in ["../secrets", "/etc/passwd", "a/../../b"] {
            let error = validate(settings(&[file], None)).unwrap_err().to_string();
            assert!(error.contains("not a path inside"), "{file}: {error}");
        }
        let mut empty = settings(&[], None);
        empty.name = " ".into();
        assert!(validate(empty).is_err());
    }
}
