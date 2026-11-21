use std::path::Path;

use anyhow::{Context, bail};

use super::TemporaryFile;
use crate::git::{LANE_GIT_TIMEOUT, command_failed, git_path, ref_exists, run, run_index};
use crate::store::{Lane, Project};

pub(super) fn copy_files_reference(lane: &Lane) -> String {
    format!("refs/panoptes/lanes/{}/copy-files", lane.name)
}

/// Saves the lane's current configured copies outside the branch and index.
/// These files are often ignored, so the tracked checkpoint cannot retain
/// them. A tree ref keeps them without making them tracked on restore.
pub(super) fn preserve_copy_files(
    project: &Project,
    lane: &Lane,
    path: &Path,
) -> anyhow::Result<()> {
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
pub(super) fn restore_copy_files(
    project: &Project,
    lane: &Lane,
    path: &Path,
) -> anyhow::Result<()> {
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
        let output = crate::git::output_with_timeout(
            crate::git::command()
                .arg("-C")
                .arg(repo)
                .args(["show", &object]),
            None,
            LANE_GIT_TIMEOUT,
        )?;
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

pub(super) fn copy_files(project: &Project, destination: &Path) -> anyhow::Result<()> {
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
