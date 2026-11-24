//! Read-only worktree usage and idle-lane selection for the cleanup dialog.
//! Archiving stays in `lanes` and repeats its safety checks for every lane.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::Serialize;

use crate::lanes::ArchivePreservation;
use crate::store::{Lane, LaneState, Project};

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUsage {
    pub worktrees: u64,
    pub bytes: u64,
}

/// Everything Panoptes knows about one Git worktree registration. Unmanaged
/// worktrees are informational: only lanes Panoptes owns may be archived or
/// deleted through their own safety-checked flows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub project_id: i64,
    pub project: String,
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
    pub bare: bool,
    pub exists: bool,
    /// Tracked and untracked changes, when Git could report them.
    pub dirty: Option<bool>,
    /// Allocated disk blocks, including ignored files; `None` when unreadable.
    pub bytes: Option<u64>,
    pub lane_id: Option<i64>,
    pub lane: Option<String>,
    pub lane_state: Option<LaneState>,
    pub managed: bool,
    pub main_checkout: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInventory {
    pub worktrees: Vec<WorktreeInfo>,
    pub total: WorktreeUsage,
    /// A repository that could not be listed, or a worktree whose size or
    /// status could not be read. The rest of the inventory remains useful.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct LaneScan {
    /// `None` when at least one of the lane's worktrees could not be measured.
    pub bytes: Option<u64>,
    pub preservation: Option<ArchivePreservation>,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub project: WorktreeUsage,
    pub total: WorktreeUsage,
    pub by_lane: HashMap<i64, LaneScan>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleLane {
    pub lane: Lane,
    pub last_activity: i64,
    /// Null when disk usage could not be read completely.
    pub bytes: Option<u64>,
    pub preservation: ArchivePreservation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneCleanup {
    pub project: WorktreeUsage,
    pub total: WorktreeUsage,
    pub idle_days: u32,
    pub candidates: Vec<IdleLane>,
    /// Measurement or session-activity errors. Measurement errors make the
    /// totals partial; activity errors suppress all suggestions.
    pub errors: Vec<String>,
    /// Every worktree Git knows about for registered projects, not just the
    /// lane-owned worktrees the existing cleanup totals can reclaim.
    pub worktree_inventory: WorktreeInventory,
}

/// Measures all active, lane-owned worktrees. Main checkouts and archived
/// lanes are not extra worktrees Panoptes can reclaim.
pub fn scan(project_id: i64, lanes: &[(Project, Lane)]) -> Scan {
    let mut scan = Scan::default();
    for (project, lane) in lanes {
        let mut bytes = Some(0);
        for worktree in lane
            .worktrees
            .iter()
            .filter(|worktree| !worktree.main_checkout)
        {
            match allocated_bytes(Path::new(&worktree.path)) {
                Ok(Some(size)) => {
                    scan.total.worktrees += 1;
                    scan.total.bytes += size;
                    if worktree.project_id == project_id {
                        scan.project.worktrees += 1;
                        scan.project.bytes += size;
                    }
                    if let Some(total) = bytes.as_mut() {
                        *total += size;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    bytes = None;
                    scan.errors
                        .push(format!("Could not measure {}: {error:#}", worktree.path));
                }
            }
        }
        scan.by_lane.insert(
            lane.id,
            LaneScan {
                bytes,
                preservation: crate::lanes::archive_preservation(project, lane),
            },
        );
    }
    scan
}

/// Lists every worktree registered by Git for each project. Git worktrees are
/// per repository, so there is no machine-wide list to scan.
pub fn inventory(projects: &[Project], lanes: &[Lane]) -> WorktreeInventory {
    let mut result = WorktreeInventory::default();
    for project in projects {
        let repo = Path::new(&project.repo);
        let records = match worktree_list(repo) {
            Ok(records) => records,
            Err(error) => {
                result.errors.push(format!(
                    "Could not list worktrees for {}: {error:#}",
                    project.settings.name
                ));
                continue;
            }
        };
        for record in records {
            let path = PathBuf::from(&record.path);
            let exists = std::fs::symlink_metadata(&path).is_ok();
            let bytes = if exists {
                match allocated_bytes(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        result
                            .errors
                            .push(format!("Could not measure {}: {error:#}", path.display()));
                        None
                    }
                }
            } else {
                None
            };
            let dirty = if exists {
                match worktree_dirty(&path) {
                    Ok(dirty) => Some(dirty),
                    Err(error) => {
                        result.errors.push(format!(
                            "Could not read status of {}: {error:#}",
                            path.display()
                        ));
                        None
                    }
                }
            } else {
                None
            };
            let canonical_path = canonical(&path);
            let owned = lanes.iter().find_map(|lane| {
                lane.worktrees
                    .iter()
                    .find(|worktree| canonical(Path::new(&worktree.path)) == canonical_path)
                    .map(|worktree| (lane, worktree))
            });
            let main_checkout = owned
                .map(|(_, worktree)| worktree.main_checkout)
                .unwrap_or_else(|| canonical(repo) == canonical_path);
            let mut info = WorktreeInfo {
                project_id: project.id,
                project: project.settings.name.clone(),
                path: record.path,
                branch: record.branch,
                head: record.head,
                detached: record.detached,
                locked: record.locked,
                prunable: record.prunable,
                bare: record.bare,
                exists,
                dirty,
                bytes,
                managed: owned.is_some(),
                main_checkout,
                ..Default::default()
            };
            if let Some((lane, _)) = owned {
                info.lane_id = Some(lane.id);
                info.lane = Some(lane.task.clone());
                info.lane_state = Some(lane.state);
            }
            result.total.worktrees += 1;
            result.total.bytes += info.bytes.unwrap_or(0);
            result.worktrees.push(info);
        }
    }
    result
        .worktrees
        .sort_by(|a, b| a.project.cmp(&b.project).then(a.path.cmp(&b.path)));
    result
}

/// Removes only stale Git administrative records whose directories are gone.
/// It never removes a live worktree or any of its files.
pub fn prune(projects: &[Project]) -> anyhow::Result<usize> {
    for project in projects {
        crate::git::run(Path::new(&project.repo), &["worktree", "prune"])
            .with_context(|| format!("Could not prune worktrees for {}", project.settings.name))?;
    }
    Ok(projects.len())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedWorktree {
    path: String,
    head: Option<String>,
    branch: Option<String>,
    detached: bool,
    locked: bool,
    prunable: bool,
    bare: bool,
}

fn worktree_list(repo: &Path) -> anyhow::Result<Vec<ParsedWorktree>> {
    let mut command = crate::git::command();
    command
        .arg("-C")
        .arg(repo)
        .args(["worktree", "list", "--porcelain", "-z"]);
    let output = crate::git::output(&mut command, None)?;
    if !output.status.success() {
        return Err(crate::git::command_failed("git worktree list", &output));
    }
    Ok(parse_worktrees(&output.stdout))
}

/// Git's `-z` porcelain format separates fields with NUL and records with an
/// empty field. Paths are shown lossily when the filesystem uses bytes that
/// are not UTF-8; Git operations still use the original path.
fn parse_worktrees(output: &[u8]) -> Vec<ParsedWorktree> {
    let mut rows = Vec::new();
    let mut current = ParsedWorktree::default();
    let mut started = false;
    for field in output.split(|byte| *byte == 0) {
        if field.is_empty() {
            if started {
                rows.push(std::mem::take(&mut current));
                started = false;
            }
            continue;
        }
        let text = String::from_utf8_lossy(field);
        if let Some(path) = text.strip_prefix("worktree ") {
            if started {
                rows.push(std::mem::take(&mut current));
            }
            current.path = path.to_string();
            started = true;
        } else if let Some(head) = text.strip_prefix("HEAD ") {
            current.head = Some(head.to_string());
        } else if let Some(reference) = text.strip_prefix("branch ") {
            current.branch = Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_string(),
            );
        } else if text == "detached" {
            current.detached = true;
        } else if text == "bare" {
            current.bare = true;
        } else if text == "locked" || text.starts_with("locked ") {
            current.locked = true;
        } else if text == "prunable" || text.starts_with("prunable ") {
            current.prunable = true;
        }
    }
    if started {
        rows.push(current);
    }
    rows
}

fn worktree_dirty(path: &Path) -> anyhow::Result<bool> {
    let mut command = crate::git::command();
    command
        .arg("-C")
        .arg(path)
        .args(["status", "--porcelain", "-z"]);
    let output = crate::git::output(&mut command, None)?;
    if !output.status.success() {
        return Err(crate::git::command_failed("git status", &output));
    }
    Ok(!output.stdout.is_empty())
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A lane is idle only once the full threshold has elapsed. Activity in the
/// future is not idle, and subtraction cannot overflow on a corrupt timestamp.
pub fn is_idle(last_activity: i64, now: i64, idle_days: u32) -> bool {
    now.saturating_sub(last_activity) >= i64::from(idle_days) * DAY_MS
}

/// BSD `du -sk` reports allocated 1024-byte blocks, including ignored files
/// such as dependencies and build output, and does not follow symlinks.
fn allocated_bytes(path: &Path) -> anyhow::Result<Option<u64>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let output = crate::git::output(Command::new("du").arg("-sk").arg(path), None)
        .with_context(|| format!("Could not run du for {}", path.display()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        bail!("du failed ({}): {}", output.status, detail.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let kibibytes = stdout
        .split_whitespace()
        .next()
        .context("du returned no size")?
        .parse::<u64>()
        .context("du returned an invalid size")?;
    kibibytes
        .checked_mul(1_024)
        .map(Some)
        .context("Worktree size is too large to report")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LaneWorktree, ProjectSettings};

    fn git(dir: &Path, args: &[&str]) {
        let status = crate::git::command()
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

    fn repo(dir: &Path) -> PathBuf {
        let repo = dir.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        canonical(&repo)
    }

    fn project(repo: &Path) -> Project {
        Project {
            id: 1,
            repo: repo.to_string_lossy().into_owned(),
            settings: ProjectSettings {
                name: "App".into(),
                base_branch: "trunk".into(),
                copy_files: Vec::new(),
                setup: None,
            },
        }
    }

    #[test]
    fn idle_threshold_includes_the_boundary_but_not_future_activity() {
        let now = 10 * DAY_MS;
        assert!(!is_idle(now - 4 * DAY_MS + 1, now, 4));
        assert!(is_idle(now - 4 * DAY_MS, now, 4));
        assert!(is_idle(now - 5 * DAY_MS, now, 4));
        assert!(!is_idle(now + 1, now, 4));
    }

    #[test]
    fn disk_usage_is_allocated_blocks_and_missing_folders_cost_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/file"), vec![0; 2_048]).unwrap();
        let bytes = allocated_bytes(dir.path()).unwrap().unwrap();
        assert_eq!(bytes % 1_024, 0);
        assert!(bytes >= 2_048, "{bytes}");
        assert_eq!(allocated_bytes(&dir.path().join("missing")).unwrap(), None);
    }

    #[test]
    fn parses_nul_separated_worktree_records() {
        let output = b"worktree /tmp/one\0HEAD abcdef123\0branch refs/heads/main\0\0\
worktree /tmp/two\0HEAD 123456789\0detached\0locked reason\0prunable gone\0\0\
worktree /tmp/bare\0bare\0\0";
        let rows = parse_worktrees(output);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].path, "/tmp/one");
        assert_eq!(rows[0].head.as_deref(), Some("abcdef123"));
        assert_eq!(rows[0].branch.as_deref(), Some("main"));
        assert_eq!(rows[1].path, "/tmp/two");
        assert!(rows[1].detached);
        assert!(rows[1].locked);
        assert!(rows[1].prunable);
        assert!(rows[2].bare);
    }

    #[test]
    fn inventory_matches_lanes_and_reports_size_branch_status_and_owner() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        let linked = dir.path().join("worktrees/fix/app");
        std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "fix",
                linked.to_str().unwrap(),
            ],
        );
        std::fs::write(linked.join("untracked.txt"), "changed").unwrap();
        let lane = Lane {
            id: 7,
            project_id: 1,
            name: "fix".into(),
            task: "Fix it".into(),
            branch: "fix".into(),
            state: LaneState::Active,
            worktrees: vec![LaneWorktree {
                project_id: 1,
                path: linked.to_string_lossy().into_owned(),
                main_checkout: false,
                checkpoint_ref: None,
            }],
        };
        let inventory = inventory(&[project(&repo)], std::slice::from_ref(&lane));
        assert!(inventory.errors.is_empty(), "{:?}", inventory.errors);
        assert_eq!(inventory.total.worktrees, 2);
        assert!(inventory.total.bytes > 0);
        let main = inventory
            .worktrees
            .iter()
            .find(|worktree| worktree.main_checkout)
            .unwrap();
        assert_eq!(main.branch.as_deref(), Some("trunk"));
        assert!(main.exists);
        assert_eq!(main.dirty, Some(false));
        assert!(main.bytes.unwrap() > 0);
        let owned = inventory
            .worktrees
            .iter()
            .find(|worktree| worktree.lane_id == Some(7))
            .unwrap();
        assert_eq!(owned.branch.as_deref(), Some("fix"));
        assert_eq!(owned.lane.as_deref(), Some("Fix it"));
        assert_eq!(owned.lane_state, Some(LaneState::Active));
        assert!(owned.managed);
        assert_eq!(owned.dirty, Some(true));
        assert!(owned.bytes.unwrap() > 0);
    }

    #[test]
    fn pruning_removes_only_stale_git_registrations() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        let stale = dir.path().join("worktrees/stale/app");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "stale",
                stale.to_str().unwrap(),
            ],
        );
        std::fs::remove_dir_all(&stale).unwrap();
        assert_eq!(prune(&[project(&repo)]).unwrap(), 1);
        let remaining = worktree_list(&repo).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, repo.to_string_lossy());
    }
}
