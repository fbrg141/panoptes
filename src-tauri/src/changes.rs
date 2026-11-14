//! Read-only diffs between two Git trees. Lane snapshots include untracked
//! files through the existing private index; no command here writes the user's
//! worktree, index, branches or configuration.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::lanes;
use crate::store::{Lane, LaneState, Project};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    pub max_file_bytes: u64,
    pub max_changed_lines: u64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            max_file_bytes: 1_000_000,
            max_changed_lines: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub from: String,
    pub to: String,
    pub files: Vec<FileChange>,
    pub totals: ChangeTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeTotals {
    pub files: usize,
    pub added: u64,
    pub removed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobIds {
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub added: u64,
    pub removed: u64,
    pub binary: bool,
    pub too_large: bool,
    pub blob: BlobIds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffLineKind {
    Context,
    Add,
    Remove,
}

/// The range a lane should be reviewed over. Active lanes snapshot their disk
/// state; archived lanes use their checkpoint, or their clean branch when no
/// checkpoint was needed.
pub fn lane_range(project: &Project, lane: &Lane) -> anyhow::Result<TreeRange> {
    let repo = Path::new(&project.repo);
    let base = resolve(
        repo,
        &format!("refs/heads/{}", project.settings.base_branch),
    )?;
    match lane.state {
        LaneState::Active => {
            let worktree = lane
                .worktrees
                .iter()
                .find(|worktree| worktree.project_id == project.id)
                .context("The lane has no worktree for this project")?;
            let path = Path::new(&worktree.path);
            let to = lanes::snapshot_tree(path, lane)?;
            let from = merge_base(path, &base, "HEAD")?;
            Ok(TreeRange { from, to })
        }
        LaneState::Archived => {
            let worktree = lane
                .worktrees
                .iter()
                .find(|worktree| worktree.project_id == project.id)
                .context("The lane has no worktree for this project")?;
            let target = worktree.checkpoint_ref.as_deref().unwrap_or(&lane.branch);
            let from = merge_base(repo, &base, target)?;
            let to = resolve(repo, &format!("{target}^{{tree}}"))?;
            Ok(TreeRange { from, to })
        }
    }
}

/// File metadata and totals for a tree range. Patch text is deliberately not
/// included: callers fetch one file's hunks only when they open it.
pub fn diff(repo: &Path, range: &TreeRange, opts: DiffOptions) -> anyhow::Result<Changes> {
    validate_range(range)?;
    let raw = run(
        repo,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--raw",
            "--no-abbrev",
            "--no-commit-id",
            "--find-renames",
            "--end-of-options",
            &range.from,
            &range.to,
        ],
    )?;
    let numstat = run(
        repo,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--numstat",
            "--no-commit-id",
            "--find-renames",
            "--end-of-options",
            &range.from,
            &range.to,
        ],
    )?;
    let counts = parse_numstat(&numstat)?;
    let mut files = parse_raw(&raw)?;
    let sizes = blob_sizes(repo, &files)?;
    let mut totals = ChangeTotals {
        files: files.len(),
        added: 0,
        removed: 0,
    };
    for file in &mut files {
        let key = (file.old_path.clone(), file.path.clone());
        let count = counts
            .get(&key)
            .copied()
            .context("Git omitted a file from its numstat diff")?;
        file.added = count.added.unwrap_or(0);
        file.removed = count.removed.unwrap_or(0);
        file.binary = count.added.is_none() && count.removed.is_none();
        let before = file
            .blob
            .before
            .as_deref()
            .and_then(|id| sizes.get(id))
            .copied()
            .unwrap_or(0);
        let after = file
            .blob
            .after
            .as_deref()
            .and_then(|id| sizes.get(id))
            .copied()
            .unwrap_or(0);
        file.too_large = !file.binary
            && (before.max(after) > opts.max_file_bytes
                || file.added + file.removed > opts.max_changed_lines);
        totals.added += file.added;
        totals.removed += file.removed;
    }
    Ok(Changes {
        from: range.from.clone(),
        to: range.to.clone(),
        files,
        totals,
    })
}

/// Hunks for one file in an already listed range. Binary and oversized files
/// return no hunks; their `FileChange` carries the reason and the counts.
pub fn hunks(
    repo: &Path,
    range: &TreeRange,
    path: &str,
    opts: DiffOptions,
) -> anyhow::Result<Vec<Hunk>> {
    let changes = diff(repo, range, opts)?;
    let file = changes
        .files
        .iter()
        .find(|file| file.path == path)
        .context("That file is not in this change range")?;
    if file.binary || file.too_large {
        return Ok(Vec::new());
    }
    let old_path = file.old_path.clone();
    let mut args = vec![
        "diff-tree",
        "-r",
        "-p",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
        "--no-commit-id",
        "-U3",
        "--end-of-options",
        &range.from,
        &range.to,
        "--",
        path,
    ];
    if let Some(old_path) = old_path.as_deref() {
        args.push(old_path);
    }
    parse_hunks(&run(repo, &args)?)
}

fn validate_range(range: &TreeRange) -> anyhow::Result<()> {
    for id in [&range.from, &range.to] {
        if !matches!(id.len(), 40 | 64) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("A change range needs full Git object ids");
        }
    }
    Ok(())
}

fn resolve(repo: &Path, reference: &str) -> anyhow::Result<String> {
    run(
        repo,
        &["rev-parse", "--verify", "--end-of-options", reference],
    )
}

fn merge_base(repo: &Path, base: &str, target: &str) -> anyhow::Result<String> {
    run(repo, &["merge-base", "--end-of-options", base, target])
}

#[derive(Debug, Clone, Copy)]
struct Counts {
    added: Option<u64>,
    removed: Option<u64>,
}

fn parse_numstat(output: &str) -> anyhow::Result<HashMap<(Option<String>, String), Counts>> {
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    let mut counts = HashMap::new();
    while let Some(record) = records.next() {
        let fields = record.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            bail!("Could not read git numstat record");
        }
        let number = |value: &str| -> anyhow::Result<Option<u64>> {
            if value == "-" {
                Ok(None)
            } else {
                value
                    .parse::<u64>()
                    .map(Some)
                    .context("Could not read git line counts")
            }
        };
        let (old_path, path) = if fields[2].is_empty() {
            (
                Some(records.next().context("Missing rename's old path")?.into()),
                records.next().context("Missing rename's new path")?.into(),
            )
        } else {
            (None, fields[2].into())
        };
        counts.insert(
            (old_path, path),
            Counts {
                added: number(fields[0])?,
                removed: number(fields[1])?,
            },
        );
    }
    Ok(counts)
}

fn parse_raw(output: &str) -> anyhow::Result<Vec<FileChange>> {
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    let mut files = Vec::new();
    while let Some(metadata) = records.next() {
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 5 || !fields[0].starts_with(':') {
            bail!("Could not read git raw diff record");
        }
        let status = fields[4].chars().next().context("Missing diff status")?;
        let (status, old_path) = match status {
            'A' => (FileStatus::Added, None),
            'M' => (FileStatus::Modified, None),
            'D' => (FileStatus::Deleted, None),
            'T' => (FileStatus::TypeChanged, None),
            'R' | 'C' => (
                FileStatus::Renamed,
                Some(records.next().context("Missing rename's old path")?.into()),
            ),
            other => bail!("Unsupported git diff status {other}"),
        };
        let path = records.next().context("Missing diff path")?.into();
        let blob = |id: &str| {
            if id.bytes().all(|byte| byte == b'0') {
                None
            } else {
                Some(id.to_string())
            }
        };
        files.push(FileChange {
            path,
            old_path,
            status,
            added: 0,
            removed: 0,
            binary: false,
            too_large: false,
            blob: BlobIds {
                before: blob(fields[2]),
                after: blob(fields[3]),
            },
        });
    }
    Ok(files)
}

fn blob_sizes(repo: &Path, files: &[FileChange]) -> anyhow::Result<HashMap<String, u64>> {
    let mut ids = files
        .iter()
        .flat_map(|file| [file.blob.before.as_ref(), file.blob.after.as_ref()])
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let input = format!("{}\n", ids.join("\n"));
    let output = run_input(
        repo,
        &[
            "cat-file",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ],
        input.as_bytes(),
    )?;
    let mut sizes = HashMap::new();
    for line in output.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() == 3 && fields[1] == "blob" {
            let size = fields[2]
                .parse::<u64>()
                .context("Could not read a Git blob's size")?;
            sizes.insert(fields[0].to_string(), size);
        }
    }
    Ok(sizes)
}

fn parse_hunks(patch: &str) -> anyhow::Result<Vec<Hunk>> {
    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_line = 0;
    let mut new_line = 0;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let end = rest.find("@@").context("Malformed hunk header")?;
            let ranges = rest[..end].trim();
            let mut ranges = ranges.split_whitespace();
            let (old_start, old_lines) =
                parse_range(ranges.next().context("Missing old range")?, '-')?;
            let (new_start, new_lines) =
                parse_range(ranges.next().context("Missing new range")?, '+')?;
            old_line = old_start;
            new_line = new_start;
            current = Some(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                header: rest[end + 2..].trim().to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        if line.starts_with("diff --git") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            continue;
        }
        let (kind, text) = match line.split_at_checked(1) {
            Some((" ", text)) => (DiffLineKind::Context, text),
            Some(("+", text)) => (DiffLineKind::Add, text),
            Some(("-", text)) => (DiffLineKind::Remove, text),
            _ => continue,
        };
        let (old, new) = match kind {
            DiffLineKind::Context => {
                let numbers = (Some(old_line), Some(new_line));
                old_line += 1;
                new_line += 1;
                numbers
            }
            DiffLineKind::Add => {
                let number = new_line;
                new_line += 1;
                (None, Some(number))
            }
            DiffLineKind::Remove => {
                let number = old_line;
                old_line += 1;
                (Some(number), None)
            }
        };
        hunk.lines.push(DiffLine {
            kind,
            text: text.to_string(),
            old_line: old,
            new_line: new,
        });
    }
    if let Some(hunk) = current {
        hunks.push(hunk);
    }
    Ok(hunks)
}

fn parse_range(range: &str, prefix: char) -> anyhow::Result<(u32, u32)> {
    let range = range.strip_prefix(prefix).context("Malformed hunk range")?;
    let mut parts = range.split(',');
    let start = parts.next().context("Missing hunk start")?.parse()?;
    let lines = parts.next().map(str::parse).transpose()?.unwrap_or(1);
    Ok((start, lines))
}

fn run(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    run_input(repo, args, &[])
}

fn run_input(repo: &Path, args: &[&str], input: &[u8]) -> anyhow::Result<String> {
    let output = git_output(repo, args, input)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args[0], stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_output(repo: &Path, args: &[&str], input: &[u8]) -> anyhow::Result<Output> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    crate::git::output(&mut command, Some(input))
        .with_context(|| format!("Could not run git {}", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn commit(repo: &Path, message: &str) {
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "--quiet", "-m", message]);
    }

    fn fixture() -> (tempfile::TempDir, Project, Lane) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch=trunk"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("old.txt"), "rename me\n").unwrap();
        std::fs::write(repo.join("delete.txt"), "delete me\n").unwrap();
        std::fs::write(repo.join("type.txt"), "change my type\n").unwrap();
        commit(&repo, "base");
        git(&repo, &["checkout", "--quiet", "-b", "panoptes/review"]);
        let project = Project {
            id: 1,
            repo: repo.to_string_lossy().into_owned(),
            settings: crate::store::ProjectSettings {
                name: "repo".into(),
                base_branch: "trunk".into(),
                copy_files: Vec::new(),
                setup: None,
            },
        };
        let lane = Lane {
            id: 1,
            project_id: 1,
            name: "review".into(),
            task: "Review".into(),
            branch: "panoptes/review".into(),
            state: LaneState::Active,
            worktrees: vec![crate::store::LaneWorktree {
                project_id: 1,
                path: repo.to_string_lossy().into_owned(),
                main_checkout: true,
                checkpoint_ref: None,
            }],
        };
        (dir, project, lane)
    }

    #[test]
    fn lane_diff_sees_untracked_files_and_does_not_touch_the_index() {
        let (_dir, project, lane) = fixture();
        let repo = Path::new(&project.repo);
        let index = repo.join(".git/index");
        let before = std::fs::read(&index).unwrap();
        std::fs::write(repo.join("tracked.txt"), "one\nchanged\n").unwrap();
        std::fs::write(repo.join("new file-✓.txt"), "new\n").unwrap();
        std::fs::write(repo.join(".env"), "ignored\n").unwrap();
        let range = lane_range(&project, &lane).unwrap();
        let changes = diff(repo, &range, DiffOptions::default()).unwrap();

        assert_eq!(changes.totals.files, 2);
        assert_eq!(changes.totals.added, 2);
        assert_eq!(changes.totals.removed, 1);
        assert!(
            changes
                .files
                .iter()
                .any(|file| file.path == "new file-✓.txt")
        );
        assert!(!changes.files.iter().any(|file| file.path == ".env"));
        assert_eq!(std::fs::read(&index).unwrap(), before);
        let hunks = hunks(repo, &range, "tracked.txt", DiffOptions::default()).unwrap();
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].lines.iter().any(|line| {
            line.kind == DiffLineKind::Add && line.text == "changed" && line.new_line == Some(2)
        }));
        assert_eq!(std::fs::read(index).unwrap(), before);
    }

    #[test]
    fn renames_deletes_type_changes_and_leading_dashes_are_exact() {
        let (_dir, project, lane) = fixture();
        let repo = Path::new(&project.repo);
        git(repo, &["mv", "old.txt", "renamed.txt"]);
        std::fs::write(repo.join("renamed.txt"), "rename me\nchanged\n").unwrap();
        std::fs::remove_file(repo.join("delete.txt")).unwrap();
        #[cfg(unix)]
        {
            std::fs::remove_file(repo.join("type.txt")).unwrap();
            std::os::unix::fs::symlink("tracked.txt", repo.join("type.txt")).unwrap();
        }
        std::fs::write(repo.join("-leading.txt"), "safe\n").unwrap();
        let range = lane_range(&project, &lane).unwrap();
        let changes = diff(repo, &range, DiffOptions::default()).unwrap();
        let file = |path: &str| changes.files.iter().find(|file| file.path == path).unwrap();

        assert_eq!(file("renamed.txt").status, FileStatus::Renamed);
        assert_eq!(file("renamed.txt").old_path.as_deref(), Some("old.txt"));
        assert_eq!(file("delete.txt").status, FileStatus::Deleted);
        assert_eq!(file("-leading.txt").status, FileStatus::Added);
        #[cfg(unix)]
        assert_eq!(file("type.txt").status, FileStatus::TypeChanged);
        assert_eq!(
            hunks(repo, &range, "renamed.txt", DiffOptions::default())
                .unwrap()
                .len(),
            1
        );
        assert!(hunks(repo, &range, "-leading.txt", DiffOptions::default()).is_ok());
    }

    #[test]
    fn user_diff_and_status_configuration_cannot_hide_changes() {
        let (_dir, project, lane) = fixture();
        let repo = Path::new(&project.repo);
        git(repo, &["config", "status.showUntrackedFiles", "no"]);
        git(repo, &["config", "diff.external", "/bin/false"]);
        std::fs::write(repo.join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "present\n").unwrap();
        let range = lane_range(&project, &lane).unwrap();
        let changes = diff(repo, &range, DiffOptions::default()).unwrap();
        assert_eq!(changes.files.len(), 2);
        assert_eq!(
            hunks(repo, &range, "tracked.txt", DiffOptions::default())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn an_archived_checkpoint_is_reviewed_without_its_worktree() {
        let (_dir, project, mut lane) = fixture();
        let repo = Path::new(&project.repo);
        std::fs::write(repo.join("archived.txt"), "kept\n").unwrap();
        commit(repo, "checkpoint contents");
        let reference = "refs/panoptes/lanes/review/archive";
        git(repo, &["update-ref", reference, "HEAD"]);
        lane.state = LaneState::Archived;
        lane.worktrees[0].path = repo.join("removed").to_string_lossy().into_owned();
        lane.worktrees[0].checkpoint_ref = Some(reference.into());

        let changes = diff(
            repo,
            &lane_range(&project, &lane).unwrap(),
            DiffOptions::default(),
        )
        .unwrap();
        assert_eq!(changes.files.len(), 1);
        assert_eq!(changes.files[0].path, "archived.txt");
    }

    #[test]
    fn an_advanced_base_does_not_appear_as_lane_work() {
        let (_dir, project, lane) = fixture();
        let repo = Path::new(&project.repo);
        std::fs::write(repo.join("lane.txt"), "lane\n").unwrap();
        commit(repo, "lane");
        git(repo, &["checkout", "--quiet", "trunk"]);
        std::fs::write(repo.join("other.txt"), "other\n").unwrap();
        commit(repo, "other");
        git(repo, &["checkout", "--quiet", "panoptes/review"]);

        let changes = diff(
            repo,
            &lane_range(&project, &lane).unwrap(),
            DiffOptions::default(),
        )
        .unwrap();
        assert_eq!(changes.files.len(), 1);
        assert_eq!(changes.files[0].path, "lane.txt");
    }

    #[test]
    fn binary_and_large_files_keep_counts_without_hunks() {
        let (_dir, project, lane) = fixture();
        let repo = Path::new(&project.repo);
        std::fs::write(repo.join("image.bin"), [0, 1, 2, 3]).unwrap();
        std::fs::write(repo.join("large.txt"), "many\nlines\n").unwrap();
        let range = lane_range(&project, &lane).unwrap();
        let options = DiffOptions {
            max_file_bytes: 1,
            max_changed_lines: 5_000,
        };
        let changes = diff(repo, &range, options).unwrap();
        let binary = changes
            .files
            .iter()
            .find(|file| file.path == "image.bin")
            .unwrap();
        assert!(binary.binary);
        assert!(
            hunks(repo, &range, "image.bin", options)
                .unwrap()
                .is_empty()
        );
        let large = changes
            .files
            .iter()
            .find(|file| file.path == "large.txt")
            .unwrap();
        assert!(large.too_large);
        assert_eq!((large.added, large.removed), (2, 0));
        assert!(
            hunks(repo, &range, "large.txt", options)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn wire_format_uses_stable_camel_case_names() {
        let changes = Changes {
            from: "a".repeat(40),
            to: "b".repeat(40),
            files: vec![FileChange {
                path: "new.txt".into(),
                old_path: Some("old.txt".into()),
                status: FileStatus::Renamed,
                added: 2,
                removed: 1,
                binary: false,
                too_large: false,
                blob: BlobIds {
                    before: Some("c".repeat(40)),
                    after: Some("d".repeat(40)),
                },
            }],
            totals: ChangeTotals {
                files: 1,
                added: 2,
                removed: 1,
            },
        };
        assert_eq!(
            serde_json::to_value(changes).unwrap(),
            serde_json::json!({
                "from": "a".repeat(40),
                "to": "b".repeat(40),
                "files": [{
                    "path": "new.txt",
                    "oldPath": "old.txt",
                    "status": "renamed",
                    "added": 2,
                    "removed": 1,
                    "binary": false,
                    "tooLarge": false,
                    "blob": { "before": "c".repeat(40), "after": "d".repeat(40) }
                }],
                "totals": { "files": 1, "added": 2, "removed": 1 }
            })
        );
    }
}
