//! Read-only worktree usage and idle-lane selection for the cleanup dialog.
//! Archiving stays in `lanes` and repeats its safety checks for every lane.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use serde::Serialize;

use crate::lanes::ArchivePreservation;
use crate::store::{Lane, Project};

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUsage {
    pub worktrees: u64,
    pub bytes: u64,
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
}
