//! Pure state for lane review: stable worktree/file identity, selection and
//! generation-checked asynchronous responses. Views only render this state.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use panoptes_core::changes::{FileChange, FileStatus, Hunk, LaneChanges, MergePreview, TreeRange};

use super::format::plural;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseNoticeKind {
    Clean,
    Conflict,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseNotice {
    pub kind: BaseNoticeKind,
    pub message: String,
    pub detail: Option<String>,
    pub paths: Vec<String>,
}

/// Base readiness is interpreted once here, so every future worktree section
/// gets the same wording and an unavailable preview can never look clean.
pub fn base_notices(review: &LaneChanges) -> Vec<BaseNotice> {
    review
        .worktrees
        .iter()
        .filter_map(|worktree| {
            let base = &worktree.base;
            if base.new_commits == Some(0) && base.preview == MergePreview::Clean {
                return None;
            }
            let freshness = match base.new_commits {
                Some(0) => format!("{} is unchanged", worktree.base_branch),
                Some(count) => format!(
                    "{} has {}",
                    worktree.base_branch,
                    plural(count as usize, "new commit", "new commits")
                ),
                None => format!("Could not count new commits on {}", worktree.base_branch),
            };
            let (kind, message, detail, paths) = match &base.preview {
                MergePreview::Clean => (
                    BaseNoticeKind::Clean,
                    format!("{freshness} · Merges cleanly"),
                    None,
                    Vec::new(),
                ),
                MergePreview::Conflicts(files) => (
                    BaseNoticeKind::Conflict,
                    format!(
                        "{freshness} · Would conflict in {}",
                        plural(files.len(), "file", "files")
                    ),
                    None,
                    files.clone(),
                ),
                MergePreview::Unavailable(reason) => (
                    BaseNoticeKind::Unavailable,
                    format!("{freshness} · Conflict preview unavailable"),
                    Some(reason.clone()),
                    Vec::new(),
                ),
            };
            let message = if review.worktrees.len() > 1 {
                format!("{} · {message}", worktree.project_name)
            } else {
                message
            };
            Some(BaseNotice {
                kind,
                message,
                detail,
                paths,
            })
        })
        .collect()
}

/// A path is unique only inside one lane worktree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReviewFileId {
    pub lane_id: i64,
    pub project_id: i64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRevision {
    before: Option<String>,
    after: Option<String>,
    old_path: Option<String>,
    status: FileStatus,
}

impl From<&FileChange> for FileRevision {
    fn from(file: &FileChange) -> Self {
        Self {
            before: file.blob.before.clone(),
            after: file.blob.after.clone(),
            old_path: file.old_path.clone(),
            status: file.status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hunks {
    Loading,
    Ready(Arc<Vec<Hunk>>),
    Failed(String),
}

#[derive(Debug, Clone)]
struct HunkEntry {
    request: u64,
    revision: FileRevision,
    pub state: Hunks,
}

/// What the store needs to fetch one file. The revision and request number
/// remain private so a response cannot be applied to different file contents.
#[derive(Debug, Clone)]
pub struct HunkRequest {
    pub file: ReviewFileId,
    pub range: TreeRange,
    request: u64,
    revision: FileRevision,
}

#[derive(Debug)]
pub struct ReviewState {
    pub lane_id: i64,
    pub visible: bool,
    pub data: Option<Arc<LaneChanges>>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<ReviewFileId>,
    pub expanded: HashSet<ReviewFileId>,
    pub hunk: usize,
    generation: u64,
    next_hunk_request: u64,
    hunks: HashMap<ReviewFileId, HunkEntry>,
}

impl ReviewState {
    pub fn new(lane_id: i64) -> Self {
        Self {
            lane_id,
            visible: true,
            data: None,
            loading: false,
            error: None,
            selected: None,
            expanded: HashSet::new(),
            hunk: 0,
            generation: 0,
            next_hunk_request: 0,
            hunks: HashMap::new(),
        }
    }

    pub fn begin_refresh(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.error = None;
        self.generation
    }

    /// Applies a listing only if it answers the latest request. Existing file
    /// selections and cached hunks survive when their blobs did not change.
    pub fn apply_refresh(&mut self, generation: u64, data: LaneChanges) -> bool {
        if generation != self.generation || data.lane_id != self.lane_id {
            return false;
        }
        let first_load = self.data.is_none();
        let first = data.worktrees.iter().find_map(|worktree| {
            worktree.changes.files.first().map(|file| ReviewFileId {
                lane_id: data.lane_id,
                project_id: worktree.project_id,
                path: file.path.clone(),
            })
        });
        let mut revisions = HashMap::new();
        for worktree in &data.worktrees {
            for file in &worktree.changes.files {
                let id = ReviewFileId {
                    lane_id: data.lane_id,
                    project_id: worktree.project_id,
                    path: file.path.clone(),
                };
                revisions.insert(id, FileRevision::from(file));
            }
        }
        self.hunks.retain(|id, entry| {
            revisions
                .get(id)
                .is_some_and(|revision| &entry.revision == revision)
        });
        self.expanded.retain(|id| revisions.contains_key(id));
        if self
            .selected
            .as_ref()
            .is_some_and(|id| revisions.contains_key(id))
        {
            // Keep the reviewer's place.
        } else {
            self.selected = first;
            self.hunk = 0;
        }
        if first_load && let Some(id) = self.selected.clone() {
            self.expanded.insert(id);
        }
        self.data = Some(Arc::new(data));
        self.loading = false;
        self.error = None;
        true
    }

    pub fn fail_refresh(&mut self, generation: u64, error: String) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        self.error = Some(error);
        true
    }

    pub fn file(&self, id: &ReviewFileId) -> Option<&FileChange> {
        if id.lane_id != self.lane_id {
            return None;
        }
        self.data
            .as_ref()?
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == id.project_id)?
            .changes
            .files
            .iter()
            .find(|file| file.path == id.path)
    }

    pub fn hunks(&self, id: &ReviewFileId) -> Option<&Hunks> {
        self.hunks.get(id).map(|entry| &entry.state)
    }

    /// Starts loading a file unless its current revision is already loading
    /// or loaded. An error can be retried by expanding it again.
    pub fn begin_hunks(&mut self, id: &ReviewFileId) -> Option<HunkRequest> {
        let file = self.file(id)?;
        let revision = FileRevision::from(file);
        let range = self
            .data
            .as_ref()?
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == id.project_id)
            .map(|worktree| TreeRange {
                from: worktree.changes.from.clone(),
                to: worktree.changes.to.clone(),
            })?;
        if let Some(entry) = self.hunks.get(id)
            && entry.revision == revision
            && !matches!(entry.state, Hunks::Failed(_))
        {
            return None;
        }
        self.next_hunk_request += 1;
        let request = self.next_hunk_request;
        self.hunks.insert(
            id.clone(),
            HunkEntry {
                request,
                revision: revision.clone(),
                state: Hunks::Loading,
            },
        );
        Some(HunkRequest {
            file: id.clone(),
            range,
            request,
            revision,
        })
    }

    pub fn apply_hunks(&mut self, request: HunkRequest, hunks: Vec<Hunk>) -> bool {
        self.finish_hunks(request, Hunks::Ready(Arc::new(hunks)))
    }

    pub fn fail_hunks(&mut self, request: HunkRequest, error: String) -> bool {
        self.finish_hunks(request, Hunks::Failed(error))
    }

    fn finish_hunks(&mut self, request: HunkRequest, state: Hunks) -> bool {
        let Some(current) = self.file(&request.file) else {
            return false;
        };
        if FileRevision::from(current) != request.revision {
            return false;
        }
        let Some(entry) = self.hunks.get_mut(&request.file) else {
            return false;
        };
        if entry.request != request.request || entry.revision != request.revision {
            return false;
        }
        entry.state = state;
        if self.selected.as_ref() == Some(&request.file)
            && let Hunks::Ready(hunks) = &entry.state
        {
            self.hunk = if hunks.is_empty() {
                0
            } else {
                self.hunk.min(hunks.len() - 1)
            };
        }
        true
    }

    /// Selects and expands a file, returning its fetch when needed.
    pub fn select_file(&mut self, id: ReviewFileId) -> Option<HunkRequest> {
        self.file(&id)?;
        self.selected = Some(id.clone());
        self.expanded.insert(id.clone());
        self.hunk = 0;
        self.begin_hunks(&id)
    }

    /// A file header collapses an expanded selection and expands any other.
    pub fn toggle_file(&mut self, id: ReviewFileId) -> Option<HunkRequest> {
        if self.selected.as_ref() == Some(&id) && self.expanded.contains(&id) {
            self.expanded.remove(&id);
            return None;
        }
        self.select_file(id)
    }

    pub fn move_file(&mut self, by: isize) -> Option<HunkRequest> {
        let files = self.files();
        if files.is_empty() {
            return None;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|id| files.iter().position(|file| file == id))
            .unwrap_or(0);
        let next = (current as isize + by).rem_euclid(files.len() as isize) as usize;
        self.select_file(files[next].clone())
    }

    pub fn move_hunk(&mut self, by: isize) -> bool {
        let Some(id) = self.selected.clone() else {
            return false;
        };
        if !self.expanded.contains(&id) {
            return false;
        }
        let Some(Hunks::Ready(hunks)) = self.hunks(&id) else {
            return false;
        };
        if hunks.is_empty() {
            return false;
        }
        self.hunk = (self.hunk as isize + by).rem_euclid(hunks.len() as isize) as usize;
        true
    }

    /// Stable lane/worktree/path order, independent of response timing.
    pub fn files(&self) -> Vec<ReviewFileId> {
        self.data
            .iter()
            .flat_map(|data| &data.worktrees)
            .flat_map(|worktree| {
                worktree.changes.files.iter().map(|file| ReviewFileId {
                    lane_id: self.lane_id,
                    project_id: worktree.project_id,
                    path: file.path.clone(),
                })
            })
            .collect()
    }

    pub fn expanded_requests(&mut self) -> Vec<HunkRequest> {
        let mut ids: Vec<_> = self.expanded.iter().cloned().collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| self.begin_hunks(&id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_core::changes::{
        BaseReview, BlobIds, ChangeTotals, Changes, DiffLine, DiffLineKind, MergePreview,
        WorktreeChanges,
    };

    fn file(path: &str, after: &str) -> FileChange {
        FileChange {
            path: path.into(),
            old_path: None,
            status: FileStatus::Modified,
            added: 1,
            removed: 0,
            binary: false,
            too_large: false,
            blob: BlobIds {
                before: Some("before".into()),
                after: Some(after.into()),
            },
        }
    }

    fn data(to: &str, paths: &[&str], after: &str) -> LaneChanges {
        LaneChanges {
            lane_id: 7,
            worktrees: vec![WorktreeChanges {
                project_id: 3,
                project_name: "web".into(),
                base_branch: "main".into(),
                base: BaseReview {
                    new_commits: Some(0),
                    preview: MergePreview::Clean,
                },
                changes: Changes {
                    from: "base".into(),
                    to: to.into(),
                    files: paths.iter().map(|path| file(path, after)).collect(),
                    totals: ChangeTotals::default(),
                },
            }],
            totals: ChangeTotals::default(),
        }
    }

    fn id(path: &str) -> ReviewFileId {
        ReviewFileId {
            lane_id: 7,
            project_id: 3,
            path: path.into(),
        }
    }

    fn hunk(line: &str) -> Hunk {
        Hunk {
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            header: String::new(),
            lines: vec![DiffLine {
                kind: DiffLineKind::Add,
                text: line.into(),
                old_line: None,
                new_line: Some(1),
            }],
        }
    }

    #[test]
    fn stale_lane_responses_never_replace_a_newer_refresh() {
        let mut state = ReviewState::new(7);
        let old = state.begin_refresh();
        let new = state.begin_refresh();
        assert!(!state.apply_refresh(old, data("old", &["old.rs"], "a")));
        assert!(state.apply_refresh(new, data("new", &["new.rs"], "b")));
        assert_eq!(state.selected, Some(id("new.rs")));
    }

    #[test]
    fn a_response_for_another_lane_or_worktree_is_not_accepted() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        let mut wrong_lane = data("one", &["a.rs"], "same");
        wrong_lane.lane_id = 8;
        assert!(!state.apply_refresh(generation, wrong_lane));
        assert!(state.apply_refresh(generation, data("one", &["a.rs"], "same")));
        assert!(
            state
                .begin_hunks(&ReviewFileId {
                    lane_id: 8,
                    ..id("a.rs")
                })
                .is_none()
        );
    }

    #[test]
    fn base_notices_distinguish_clean_conflicting_and_unavailable() {
        let mut review = data("one", &["a.rs"], "same");
        assert!(base_notices(&review).is_empty());
        review.worktrees[0].base.new_commits = Some(2);
        review.worktrees[0].base.preview = MergePreview::Clean;
        let notices = base_notices(&review);
        assert_eq!(notices[0].kind, BaseNoticeKind::Clean);
        assert_eq!(
            notices[0].message,
            "main has 2 new commits · Merges cleanly"
        );

        review.worktrees[0].base.preview = MergePreview::Conflicts(vec!["new file-✓.txt".into()]);
        let notices = base_notices(&review);
        assert_eq!(notices[0].kind, BaseNoticeKind::Conflict);
        assert_eq!(notices[0].paths, ["new file-✓.txt"]);

        review.worktrees[0].base.preview = MergePreview::Unavailable("old git".into());
        let notices = base_notices(&review);
        assert_eq!(notices[0].kind, BaseNoticeKind::Unavailable);
        assert_eq!(notices[0].detail.as_deref(), Some("old git"));
    }

    #[test]
    fn unchanged_files_keep_their_cached_hunks_across_a_refresh() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("one", &["a.rs", "b.rs"], "same"));
        let request = state.begin_hunks(&id("a.rs")).unwrap();
        state.apply_hunks(request, vec![hunk("cached")]);
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("two", &["a.rs", "b.rs"], "same"));
        assert!(matches!(state.hunks(&id("a.rs")), Some(Hunks::Ready(_))));
        assert!(state.begin_hunks(&id("a.rs")).is_none());
    }

    #[test]
    fn changed_or_removed_files_drop_stale_hunks_and_responses() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("one", &["a.rs"], "old"));
        let request = state.begin_hunks(&id("a.rs")).unwrap();
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("two", &["a.rs"], "new"));
        assert!(!state.apply_hunks(request, vec![hunk("stale")]));
        assert!(state.hunks(&id("a.rs")).is_none());
    }

    #[test]
    fn navigation_is_stable_and_expands_the_selected_file() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("one", &["a.rs", "b.rs"], "same"));
        assert_eq!(state.selected, Some(id("a.rs")));
        let _ = state.move_file(1);
        assert_eq!(state.selected, Some(id("b.rs")));
        assert!(state.expanded.contains(&id("b.rs")));
        let _ = state.move_file(1);
        assert_eq!(state.selected, Some(id("a.rs")));
        assert!(!state.move_hunk(1));
    }

    #[test]
    fn equal_paths_in_different_worktrees_have_independent_hunks() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        let mut changes = data("one", &["src/lib.rs"], "web");
        let mut api = changes.worktrees[0].clone();
        api.project_id = 4;
        api.project_name = "api".into();
        api.changes.files[0].blob.after = Some("api".into());
        changes.worktrees.push(api);
        state.apply_refresh(generation, changes);
        let web = id("src/lib.rs");
        let api = ReviewFileId {
            project_id: 4,
            ..web.clone()
        };
        let web_request = state.begin_hunks(&web).unwrap();
        let api_request = state.begin_hunks(&api).unwrap();
        assert!(state.apply_hunks(api_request, vec![hunk("api")]));
        assert!(state.apply_hunks(web_request, vec![hunk("web")]));
        assert!(matches!(state.hunks(&web), Some(Hunks::Ready(_))));
        assert!(matches!(state.hunks(&api), Some(Hunks::Ready(_))));
        assert_eq!(state.files(), vec![web, api]);
    }

    #[test]
    fn hunk_navigation_wraps_only_when_hunks_exist() {
        let mut state = ReviewState::new(7);
        let generation = state.begin_refresh();
        state.apply_refresh(generation, data("one", &["a.rs"], "same"));
        let request = state.begin_hunks(&id("a.rs")).unwrap();
        state.apply_hunks(request, vec![hunk("one"), hunk("two")]);
        assert!(state.move_hunk(1));
        assert_eq!(state.hunk, 1);
        assert!(state.move_hunk(1));
        assert_eq!(state.hunk, 0);
    }
}
