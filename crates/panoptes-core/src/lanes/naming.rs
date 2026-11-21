use std::path::{Path, PathBuf};

use crate::git::ref_exists;

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

pub(super) fn worktree_path(root: &Path, repo: &Path, name: &str) -> PathBuf {
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
pub(super) fn free_name(
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
