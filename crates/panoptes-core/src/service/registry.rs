use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use panoptes_harness::{AgentSession, SessionStatus};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::error::{ServiceResult, not_open};
use super::status::Tracker;
use super::writers::LaneWriters;

pub(super) type EarlyExit = Arc<std::sync::Mutex<Option<String>>>;

pub(super) struct OpenSession {
    /// Distinguishes this process from earlier ones for the same session, so
    /// a late exit from a replaced process cannot close its successor.
    pub(super) generation: u64,
    pub(super) session: Arc<dyn AgentSession>,
    pub(super) status: Tracker,
    /// Lane membership, used to keep an open lane from being archived.
    pub(super) lane: Option<i64>,
    /// Read-only history alongside the lane's live session.
    pub(super) preview: bool,
}

/// What a session's process is started with.
pub(super) struct Launch {
    pub(super) harness: String,
    pub(super) cwd: PathBuf,
    pub(super) resume: Option<String>,
    /// Lane membership, not a lock.
    pub(super) lane: Option<i64>,
    /// Read-only for archived lanes or a missing-folder resume.
    pub(super) read_only: bool,
    /// History alongside the lane's live session; a prompt replaces both.
    pub(super) preview: bool,
    /// Extra system-prompt instructions stored by a newly created session.
    pub(super) instructions: Option<String>,
}

/// Lists a session as starting for as long as it lives.
pub(super) struct Starting {
    starting: Arc<std::sync::Mutex<HashMap<String, Option<i64>>>>,
    key: String,
}

impl Starting {
    fn new(
        starting: &Arc<std::sync::Mutex<HashMap<String, Option<i64>>>>,
        key: &str,
        lane: Option<i64>,
    ) -> Self {
        starting.lock().unwrap().insert(key.to_string(), lane);
        Self {
            starting: starting.clone(),
            key: key.to_string(),
        }
    }
}

impl Drop for Starting {
    fn drop(&mut self) {
        self.starting.lock().unwrap().remove(&self.key);
    }
}

/// Per-key async locks. Entries are never removed, so two callers can never
/// receive different locks for one key.
struct KeyedLocks<K>(std::sync::Mutex<HashMap<K, Arc<Mutex<()>>>>);

impl<K> Default for KeyedLocks<K> {
    fn default() -> Self {
        Self(std::sync::Mutex::default())
    }
}

impl<K: Eq + Hash + Clone> KeyedLocks<K> {
    async fn lock(&self, key: K) -> OwnedMutexGuard<()> {
        let lock = self.0.lock().unwrap().entry(key).or_default().clone();
        lock.lock_owned().await
    }
}

/// Process registry and lifecycle state.
///
/// Lock order is session lifecycle, then lane lifecycle, then the open
/// mutex, then standard mutexes. Keep it in this order at every call site.
pub(super) struct Registry {
    open: Arc<Mutex<HashMap<String, OpenSession>>>,
    lifecycle: KeyedLocks<String>,
    lane_lifecycle: KeyedLocks<i64>,
    starting: Arc<std::sync::Mutex<HashMap<String, Option<i64>>>>,
    generations: AtomicU64,
    lane_writers: LaneWriters,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self {
            open: Arc::default(),
            lifecycle: KeyedLocks::default(),
            lane_lifecycle: KeyedLocks::default(),
            starting: Arc::default(),
            generations: AtomicU64::new(1),
            lane_writers: LaneWriters::default(),
        }
    }

    pub(super) async fn open(&self) -> tokio::sync::MutexGuard<'_, HashMap<String, OpenSession>> {
        self.open.lock().await
    }

    pub(super) fn open_handle(&self) -> Arc<Mutex<HashMap<String, OpenSession>>> {
        self.open.clone()
    }

    pub(super) async fn lifecycle(&self, key: &str) -> OwnedMutexGuard<()> {
        self.lifecycle.lock(key.to_string()).await
    }

    pub(super) async fn lane_lifecycle(&self, id: i64) -> OwnedMutexGuard<()> {
        self.lane_lifecycle.lock(id).await
    }

    pub(super) fn starting(&self, key: &str, lane: Option<i64>) -> Starting {
        Starting::new(&self.starting, key, lane)
    }

    pub(super) fn is_starting_in_lane(&self, lane_id: i64) -> bool {
        self.starting
            .lock()
            .unwrap()
            .values()
            .any(|lane| *lane == Some(lane_id))
    }

    pub(super) async fn live_in_lane(&self, lane_id: i64) -> Option<String> {
        self.open()
            .await
            .iter()
            .find(|(_, session)| session.lane == Some(lane_id) && !session.preview)
            .map(|(key, _)| key.clone())
    }

    /// Whether a session in the lane is starting or open, previews included.
    pub(super) async fn lane_in_use(&self, lane_id: i64) -> bool {
        self.is_starting_in_lane(lane_id)
            || self
                .open()
                .await
                .values()
                .any(|session| session.lane == Some(lane_id))
    }

    pub(super) async fn lanes_in_use(&self) -> HashSet<i64> {
        let mut lanes: HashSet<i64> = self
            .starting
            .lock()
            .unwrap()
            .values()
            .filter_map(|lane| *lane)
            .collect();
        lanes.extend(
            self.open()
                .await
                .values()
                .filter_map(|session| session.lane),
        );
        lanes
    }

    pub(super) fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn lane_writers(&self) -> LaneWriters {
        self.lane_writers.clone()
    }

    pub(super) async fn session(&self, key: &str) -> ServiceResult<Arc<dyn AgentSession>> {
        self.open()
            .await
            .get(key)
            .map(|open| open.session.clone())
            .ok_or_else(not_open)
    }
}

/// The key of a session other than `lane_id`'s own that has edited the lane
/// and is Running or Waiting, if there is one.
pub(super) fn busy_writer(
    open: &HashMap<String, OpenSession>,
    writers: &LaneWriters,
    lane_id: i64,
) -> Option<String> {
    writers.busy_writer(lane_id, |key| {
        open.get(key).is_some_and(|session| {
            session.lane != Some(lane_id)
                && matches!(
                    session.status.lock().unwrap().status(),
                    SessionStatus::Running | SessionStatus::Waiting
                )
        })
    })
}
