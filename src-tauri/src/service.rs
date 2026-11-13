//! The app's session registry: which harnesses are loaded, which sessions
//! are open, and Panoptes' own store. Independent of Tauri so it can be
//! tested with a fake harness; `lib.rs` exposes it as commands.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::Context;
use panoptes_harness::{
    AgentSession, Answer, DiscoveredSession, Harness, HarnessError, HarnessErrorKind, HarnessEvent,
    HarnessRef, ImageAttachment, Message, ModelInfo, PromptMode, PromptOutcome, Role,
    SessionSettings, SessionStatus, StartSession,
};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::lanes::{self, ArchiveResult, ArchiveStrategy, LaneWork};
use crate::projects::{self, Location};
use crate::store::{Lane, LaneState, Project, ProjectSettings, Store};
use crate::writes;

pub type ServiceResult<T> = Result<T, ServiceError>;

/// An error as the UI receives it: a kind to choose the recovery from, and a
/// message to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The session has no process; open it again.
    NotOpen,
    /// The harness it needs is not installed or failed to load.
    Unavailable,
    /// The harness refused the command; its message says why.
    Rejected,
    /// The harness process ended; reopen the session to restart it.
    Exited,
    /// The harness did not answer in time.
    TimedOut,
    /// A lane creation was cancelled; its partial worktree was rolled back.
    Cancelled,
    /// Anything else (I/O, a harness error with no kind).
    Other,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<anyhow::Error> for ServiceError {
    fn from(error: anyhow::Error) -> Self {
        let kind = match HarnessError::kind_of(&error) {
            Some(HarnessErrorKind::Rejected) => ErrorKind::Rejected,
            Some(HarnessErrorKind::Exited) => ErrorKind::Exited,
            Some(HarnessErrorKind::TimedOut) => ErrorKind::TimedOut,
            None => ErrorKind::Other,
        };
        Self::new(kind, format!("{error:#}"))
    }
}

fn err(error: anyhow::Error) -> ServiceError {
    error.into()
}

fn not_open() -> ServiceError {
    ServiceError::new(ErrorKind::NotOpen, "This session is not open")
}

fn only_in_a_lane(what: &str) -> ServiceError {
    ServiceError::new(
        ErrorKind::Rejected,
        format!("Only a session in a lane can be {what}"),
    )
}

const PANOPTES_PREFIX: &str = "panoptes:";

/// The key of a Panoptes session. It is known before the harness starts, so
/// every event the session sends carries it.
pub fn panoptes_key(id: i64) -> String {
    format!("{PANOPTES_PREFIX}{id}")
}

/// The key of a harness conversation that no lane has adopted.
pub fn harness_key(harness: &str, id: &str) -> String {
    format!("{harness}:{id}")
}

fn panoptes_id(key: &str) -> Option<i64> {
    key.strip_prefix(PANOPTES_PREFIX)?.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub key: String,
    pub event: SessionUpdate,
}

/// What the UI hears about an open session: the harness's own events, plus
/// the status the service works out from them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SessionUpdate {
    Harness(HarnessEvent),
    Status(StatusChanged),
    LaneWrites(LaneWritesChanged),
}

/// Sent when an open session is first seen editing another lane's worktree,
/// so the sidebar can hold back that lane's own session straight away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "laneWritesChanged")]
pub struct LaneWritesChanged {
    pub lane_id: i64,
}

/// Sent when an open session's status changes, and once when it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "statusChanged")]
pub struct StatusChanged {
    pub status: SessionStatus,
}

/// Works out a session's status from its events. Waiting wins over
/// Running, so a session that needs the user always says so.
#[derive(Debug)]
struct StatusTracker {
    status: SessionStatus,
    running: bool,
    /// Open questions and approvals, with when the harness stops waiting
    /// (Unix ms).
    questions: HashMap<String, Option<i64>>,
    /// The latest assistant message in this run reported an error.
    errored: bool,
    /// How the last run ended: Idle before any, then Done or Failed.
    settled: SessionStatus,
}

impl Default for StatusTracker {
    fn default() -> Self {
        Self {
            status: SessionStatus::Idle,
            running: false,
            questions: HashMap::new(),
            errored: false,
            settled: SessionStatus::Idle,
        }
    }
}

impl StatusTracker {
    /// Folds in one event; returns the new status if it changed.
    fn apply(&mut self, event: &HarnessEvent, now: i64) -> Option<SessionStatus> {
        match event {
            HarnessEvent::RunStarted => {
                self.running = true;
                self.errored = false;
            }
            HarnessEvent::MessageCompleted { message } if message.role == Role::Assistant => {
                // A retried provider error is cleared by the reply that follows.
                self.errored = message.error.is_some();
            }
            HarnessEvent::Question { question } => {
                self.questions
                    .insert(question.id.clone(), question.expires_at);
            }
            HarnessEvent::Approval { approval } => {
                self.questions
                    .insert(approval.id.clone(), approval.expires_at);
            }
            HarnessEvent::RunSettled { .. } => {
                self.running = false;
                self.questions.clear();
                self.settled = if self.errored {
                    SessionStatus::Failed
                } else {
                    SessionStatus::Done
                };
            }
            HarnessEvent::Exited { .. } => {
                self.running = false;
                self.questions.clear();
                self.settled = SessionStatus::Failed;
            }
            _ => {}
        }
        self.update(now)
    }

    fn answered(&mut self, question_id: &str, now: i64) -> Option<SessionStatus> {
        self.questions.remove(question_id);
        self.update(now)
    }

    fn update(&mut self, now: i64) -> Option<SessionStatus> {
        // The harness picks a default once a question expires.
        self.questions
            .retain(|_, expires| expires.is_none_or(|at| at > now));
        let status = if !self.questions.is_empty() {
            SessionStatus::Waiting
        } else if self.running {
            SessionStatus::Running
        } else {
            self.settled
        };
        (status != self.status).then(|| {
            self.status = status;
            status
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub type Emit = Arc<dyn Fn(SessionEvent) + Send + Sync>;

/// The one session type the UI sees: a Panoptes session in a lane, or a
/// harness conversation found on disk that no lane has adopted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// `panoptes:<id>` or `<harness>:<harness id>`; see [`panoptes_key`].
    pub key: String,
    /// Panoptes' own id; `None` for a conversation no lane has adopted.
    pub id: Option<i64>,
    pub lane_id: Option<i64>,
    pub harness: String,
    /// The harness conversation, once the harness has reported it.
    pub harness_ref: Option<HarnessRef>,
    /// Where it runs: its lane's worktree, or the harness's folder.
    pub cwd: String,
    /// Panoptes' title, else the harness's, else "New session".
    pub title: String,
    pub tags: Vec<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: u32,
    pub parent_locator: Option<String>,
    pub project_root: String,
    /// The main checkout of the git repository the session runs in, shared
    /// by all its worktrees. `None` outside git.
    pub checkout: Option<String>,
    /// For a lane session, its lane's project; otherwise the registered
    /// project whose repository the folder is in.
    pub project_id: Option<i64>,
    pub pinned: bool,
    pub archived: bool,
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneNavigation {
    pub lane_id: i64,
    /// Non-archived sessions, pinned first and then most recent.
    pub sessions: Vec<Session>,
    /// Archived sessions stay attached to the lane for its archive view.
    pub archived_sessions: Vec<Session>,
    /// What an active lane's worktree holds, whether or not it has a
    /// session; `None` for archived lanes or a worktree that can't be read.
    pub work: Option<LaneWork>,
    /// Another session that has edited this lane's worktree and is Running
    /// or Waiting now. While there is one, the lane gets no session of its
    /// own, so there are never two writers.
    pub busy_writer: Option<BusyWriter>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BusyWriter {
    pub session_key: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// Panoptes sessions, archived ones included, then every harness
    /// conversation no lane has adopted.
    pub sessions: Vec<Session>,
    /// Every registered project, by name, with or without sessions.
    pub projects: Vec<Project>,
    /// Every lane, including archived ones; navigation decides which to show.
    pub lanes: Vec<Lane>,
    /// Authoritative session membership for each lane. The UI renders this
    /// hierarchy rather than inferring it from folders.
    pub navigation: Vec<LaneNavigation>,
    /// One line per harness whose sessions could not be listed, and one
    /// each if the pins, projects, lanes or sessions could not be read.
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AdoptionOutcome {
    Adopted {
        key: String,
    },
    /// No unique matching worktree; the user must pick an active lane.
    ChooseLane,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ArchiveLaneOutcome {
    Archived,
    Dirty { lane: Lane },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSession {
    /// Opening an adopted conversation by its harness key answers with its
    /// Panoptes key.
    pub key: String,
    pub history: Vec<Message>,
}

type Tracker = Arc<std::sync::Mutex<StatusTracker>>;
type EarlyExit = Arc<std::sync::Mutex<Option<String>>>;

struct OpenSession {
    /// Distinguishes this process from earlier ones for the same session, so
    /// a late exit from a replaced process cannot close its successor.
    generation: u64,
    session: Arc<dyn AgentSession>,
    status: Tracker,
    /// Lane membership, used to keep an open lane from being archived.
    lane: Option<i64>,
}

/// What a session's process is started with.
struct Launch {
    harness: String,
    cwd: PathBuf,
    resume: Option<String>,
    /// Lane membership, not a lock.
    lane: Option<i64>,
    /// Read-only for archived lanes or a missing-folder resume.
    read_only: bool,
}

/// Forwards one process's events, tagged with its session's key, and works
/// out the session's status.
struct Forwarder {
    key: String,
    generation: u64,
    open: Arc<Mutex<HashMap<String, OpenSession>>>,
    status: Tracker,
    emit: Emit,
    writes: WriteWatch,
    /// Set if the process ended before `start` had listed the session as
    /// open, with what it last wrote to stderr.
    early_exit: EarlyExit,
}

/// Lanes other than its own that each open session has edited, by lane.
/// Kept in memory for as long as the writing session runs: it only has to
/// hold back a second writer, not record history.
type LaneWriters = Arc<std::sync::Mutex<HashMap<i64, HashSet<String>>>>;

/// What a forwarder needs to notice its session editing other lanes.
struct WriteWatch {
    writers: LaneWriters,
    store: Arc<std::sync::Mutex<Store>>,
    /// The session's own lane, never counted as another lane.
    own: Option<i64>,
    cwd: PathBuf,
}

impl Forwarder {
    /// Notes the other lanes a finished edit touched, and forgets the
    /// session's edits once its process ends.
    async fn watch(&self, event: &HarnessEvent) {
        match event {
            HarnessEvent::ToolCallUpdated { call } => {
                let paths = writes::edited_paths(call);
                if paths.is_empty() {
                    return;
                }
                let store = self.writes.store.clone();
                let (own, cwd) = (self.writes.own, self.writes.cwd.clone());
                let touched = tokio::task::spawn_blocking(move || {
                    let lanes = store.lock().unwrap().lanes()?;
                    anyhow::Ok(writes::lanes_touched(&paths, &cwd, own, &lanes))
                })
                .await;
                let Ok(Ok(touched)) = touched else {
                    return;
                };
                let new: Vec<i64> = {
                    let mut writers = self.writes.writers.lock().unwrap();
                    touched
                        .into_iter()
                        .filter(|lane| writers.entry(*lane).or_default().insert(self.key.clone()))
                        .collect()
                };
                for lane_id in new {
                    (self.emit)(SessionEvent {
                        key: self.key.clone(),
                        event: SessionUpdate::LaneWrites(LaneWritesChanged { lane_id }),
                    });
                }
            }
            HarnessEvent::Exited { .. } => {
                let mut writers = self.writes.writers.lock().unwrap();
                for keys in writers.values_mut() {
                    keys.remove(&self.key);
                }
                writers.retain(|_, keys| !keys.is_empty());
            }
            _ => {}
        }
    }

    async fn run(self, mut events: tokio::sync::mpsc::UnboundedReceiver<HarnessEvent>) {
        while let Some(event) = events.recv().await {
            self.watch(&event).await;
            if let HarnessEvent::Exited { stderr_tail, .. } = &event {
                let mut open = self.open.lock().await;
                let current = open
                    .get(&self.key)
                    .is_some_and(|o| o.generation == self.generation);
                if !current {
                    // Closed on purpose or replaced: not news. If `start`
                    // has not listed the session yet, it reads this under
                    // the same lock and refuses to list a dead process.
                    *self.early_exit.lock().unwrap() = Some(stderr_tail.clone());
                    continue;
                }
                open.remove(&self.key);
            }
            {
                // Holding the tracker while emitting keeps statuses in
                // order with the ones `answer` sends.
                let mut tracker = self.status.lock().unwrap();
                let changed = tracker.apply(&event, now_ms());
                (self.emit)(SessionEvent {
                    key: self.key.clone(),
                    event: SessionUpdate::Harness(event),
                });
                if let Some(status) = changed {
                    (self.emit)(SessionEvent {
                        key: self.key.clone(),
                        event: SessionUpdate::Status(StatusChanged { status }),
                    });
                }
            }
        }
    }
}

pub struct Service {
    harnesses: HashMap<String, Arc<dyn Harness>>,
    open: Arc<Mutex<HashMap<String, OpenSession>>>,
    /// One lock per session key while it is being opened, so concurrent
    /// opens of one session start one process, never two writers on one
    /// session file.
    opening: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    generations: AtomicU64,
    /// Shared with the blocking threads that run every store call.
    store: Arc<std::sync::Mutex<Store>>,
    /// Central folder for lane worktrees, normally `~/Panoptes/lanes`.
    lanes_root: PathBuf,
    /// Cancellation flags for setup commands, keyed by their project.
    lane_cancellations: Arc<std::sync::Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    /// Lanes with an archive, restore or delete in progress.
    busy_lanes: Arc<std::sync::Mutex<HashSet<i64>>>,
    /// Open sessions seen editing a lane that is not their own.
    lane_writers: LaneWriters,
    /// Where each working directory sits in git, so each is looked up once.
    locations: std::sync::Mutex<HashMap<String, Location>>,
    emit: Emit,
}

impl Service {
    pub fn new(
        harnesses: Vec<Arc<dyn Harness>>,
        store: Store,
        lanes_root: PathBuf,
        emit: Emit,
    ) -> Self {
        Self {
            harnesses: harnesses
                .into_iter()
                .map(|harness| (harness.id().to_string(), harness))
                .collect(),
            open: Arc::default(),
            opening: Mutex::default(),
            generations: AtomicU64::new(1),
            store: Arc::new(std::sync::Mutex::new(store)),
            lanes_root,
            lane_cancellations: Arc::default(),
            busy_lanes: Arc::default(),
            lane_writers: Arc::default(),
            locations: std::sync::Mutex::default(),
            emit,
        }
    }

    fn harness(&self, id: &str) -> ServiceResult<Arc<dyn Harness>> {
        self.harnesses.get(id).cloned().ok_or_else(|| {
            ServiceError::new(
                ErrorKind::Unavailable,
                format!("The {id} harness is not available"),
            )
        })
    }

    async fn session(&self, key: &str) -> ServiceResult<Arc<dyn AgentSession>> {
        self.open
            .lock()
            .await
            .get(key)
            .map(|open| open.session.clone())
            .ok_or_else(not_open)
    }

    pub async fn list(&self) -> Listing {
        let mut discovered = Vec::new();
        let mut errors = Vec::new();
        for harness in self.harnesses.values() {
            match harness.list_sessions().await {
                Ok(found) => discovered.extend(found),
                Err(error) => errors.push(format!(
                    "Could not list {} sessions: {}",
                    harness.id(),
                    err(error)
                )),
            }
        }
        let pinned = self
            .with_store(|store| store.pinned())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read pins: {}", err(error)));
                Default::default()
            });
        let projects = self
            .with_store(|store| store.projects())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read projects: {}", err(error)));
                Vec::new()
            });
        let lanes = self
            .with_store(|store| store.lanes())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read lanes: {}", err(error)));
                Vec::new()
            });
        let records = self
            .with_store(|store| store.sessions())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read sessions: {}", err(error)));
                Vec::new()
            });
        let work: HashMap<i64, LaneWork> = {
            let active: Vec<(Project, Lane)> = lanes
                .iter()
                .filter(|lane| lane.state == LaneState::Active)
                .filter_map(|lane| {
                    let project = projects.iter().find(|p| p.id == lane.project_id)?;
                    Some((project.clone(), lane.clone()))
                })
                .collect();
            tokio::task::spawn_blocking(move || {
                active
                    .iter()
                    .filter_map(|(project, lane)| Some((lane.id, lanes::work(project, lane)?)))
                    .collect()
            })
            .await
            .unwrap_or_default()
        };
        let lane_by_id: HashMap<i64, &Lane> = lanes.iter().map(|lane| (lane.id, lane)).collect();
        let by_checkout: HashMap<&Path, i64> = projects
            .iter()
            .map(|project| (Path::new(&project.repo), project.id))
            .collect();
        // Discovered conversations by harness and id, in the harnesses' order.
        let index: HashMap<(&str, &str), usize> = discovered
            .iter()
            .enumerate()
            .map(|(i, session)| {
                let reference = &session.harness_ref;
                ((reference.harness.as_str(), reference.id.as_str()), i)
            })
            .collect();
        let mut adopted = vec![false; discovered.len()];
        let open = self.open.lock().await;
        let busy_writers: HashMap<i64, String> = lanes
            .iter()
            .filter_map(|lane| Some((lane.id, busy_writer(&open, &self.lane_writers, lane.id)?)))
            .collect();
        let mut locations = self.locations.lock().unwrap();
        let mut sessions = Vec::new();
        for record in records {
            let lane = lane_by_id.get(&record.lane_id).copied();
            let cwd = lane.map(lane_cwd).unwrap_or_default();
            let discovered: Option<&DiscoveredSession> = record
                .harness_id
                .as_deref()
                .and_then(|id| index.get(&(record.harness.as_str(), id)))
                .map(|&i| {
                    adopted[i] = true;
                    &discovered[i]
                });
            let harness_ref = match &discovered {
                Some(session) => Some(session.harness_ref.clone()),
                None => record.harness_id.clone().map(|id| HarnessRef {
                    harness: record.harness.clone(),
                    id,
                    locator: record.locator.clone().unwrap_or_default(),
                    cwd: cwd.clone(),
                }),
            };
            let location = cached_location(&mut locations, &cwd);
            let key = panoptes_key(record.id);
            sessions.push(Session {
                open: open.contains_key(&key),
                key,
                id: Some(record.id),
                lane_id: Some(record.lane_id),
                harness: record.harness,
                harness_ref,
                title: record
                    .title
                    .or_else(|| discovered.as_ref().map(|d| d.title.clone()))
                    .unwrap_or_else(|| "New session".into()),
                tags: record.tags,
                created_at: discovered
                    .as_ref()
                    .map_or(record.created_at, |d| d.created_at),
                updated_at: discovered
                    .as_ref()
                    .map_or(record.created_at, |d| d.updated_at),
                message_count: discovered.as_ref().map_or(0, |d| d.message_count),
                parent_locator: discovered.and_then(|d| d.parent_locator.clone()),
                cwd,
                project_root: location.root,
                checkout: location
                    .checkout
                    .as_deref()
                    .map(|c| c.to_string_lossy().into_owned()),
                project_id: lane.map(|lane| lane.project_id),
                pinned: record.pinned,
                archived: record.archived,
            });
        }
        drop(index);
        let unadopted = discovered
            .into_iter()
            .zip(adopted)
            .filter_map(|(session, adopted)| (!adopted).then_some(session));
        for session in unadopted {
            let reference = session.harness_ref;
            let key = harness_key(&reference.harness, &reference.id);
            let location = cached_location(&mut locations, &reference.cwd);
            sessions.push(Session {
                project_id: location
                    .checkout
                    .as_deref()
                    .and_then(|checkout| by_checkout.get(checkout).copied()),
                checkout: location
                    .checkout
                    .as_deref()
                    .map(|c| c.to_string_lossy().into_owned()),
                project_root: location.root,
                pinned: pinned.contains(&key),
                open: open.contains_key(&key),
                key,
                id: None,
                lane_id: None,
                harness: reference.harness.clone(),
                cwd: reference.cwd.clone(),
                harness_ref: Some(reference),
                title: session.title,
                tags: Vec::new(),
                created_at: session.created_at,
                updated_at: session.updated_at,
                message_count: session.message_count,
                parent_locator: session.parent_locator,
                archived: false,
            });
        }
        drop(locations);
        drop(open);
        let all = &sessions;
        let mut navigation: Vec<LaneNavigation> = lanes
            .iter()
            .map(|lane| {
                let mut sessions: Vec<Session> = sessions
                    .iter()
                    .filter(|session| session.lane_id == Some(lane.id))
                    .cloned()
                    .collect();
                sessions.sort_by(|a, b| {
                    b.pinned
                        .cmp(&a.pinned)
                        .then_with(|| b.updated_at.cmp(&a.updated_at))
                });
                let archived_sessions = sessions
                    .iter()
                    .filter(|session| session.archived)
                    .cloned()
                    .collect();
                sessions.retain(|session| !session.archived);
                let busy_writer = busy_writers.get(&lane.id).map(|key| BusyWriter {
                    session_key: key.clone(),
                    title: all
                        .iter()
                        .find(|session| &session.key == key)
                        .map_or_else(|| "Another session".into(), |s| s.title.clone()),
                });
                LaneNavigation {
                    lane_id: lane.id,
                    sessions,
                    archived_sessions,
                    work: work.get(&lane.id).copied(),
                    busy_writer,
                }
            })
            .collect();
        navigation.sort_by_key(|entry| entry.lane_id);
        // All sessions is one reverse-chronological list. The lane lists
        // above apply their own pinned-first ordering.
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Listing {
            sessions,
            projects,
            lanes,
            navigation,
            errors,
        }
    }

    /// Opens a session, or returns it if it is already open. A harness
    /// conversation that a lane has adopted opens as that Panoptes session,
    /// so one conversation never gets two processes. `harness`, `locator`
    /// and `cwd` are only read for a conversation no lane has adopted.
    pub async fn open(
        &self,
        harness: &str,
        key: &str,
        locator: String,
        cwd: String,
    ) -> ServiceResult<OpenedSession> {
        let key = match panoptes_id(key) {
            Some(_) => key.to_string(),
            None => {
                let harness_id = key
                    .strip_prefix(&format!("{harness}:"))
                    .unwrap_or_default()
                    .to_string();
                let name = harness.to_string();
                let adopted = self
                    .with_store(move |store| store.session_for(&name, &harness_id))
                    .await
                    .map_err(err)?;
                adopted.map_or_else(|| key.to_string(), panoptes_key)
            }
        };
        let lock = self
            .opening
            .lock()
            .await
            .entry(key.clone())
            .or_default()
            .clone();
        let _guard = lock.lock().await;

        let existing = self.open.lock().await.get(&key).map(|o| o.session.clone());
        let result = match existing {
            Some(session) => session
                .history()
                .await
                .map(|history| OpenedSession {
                    key: key.clone(),
                    history,
                })
                .map_err(err),
            None => match panoptes_id(&key) {
                Some(id) => match self.recorded_launch(id).await {
                    Ok(launch) => self.start(&key, launch, None).await,
                    Err(error) => Err(error),
                },
                None => {
                    let launch = Launch {
                        harness: harness.to_string(),
                        cwd: PathBuf::from(cwd),
                        resume: Some(locator),
                        lane: None,
                        read_only: false,
                    };
                    self.start(&key, launch, None).await
                }
            },
        };
        drop(_guard);
        self.opening.lock().await.remove(&key);
        result
    }

    /// How a recorded session starts: in its lane's worktree, resuming its
    /// conversation if the harness ever reported one.
    async fn recorded_launch(&self, id: i64) -> ServiceResult<Launch> {
        let (record, lane) = self
            .with_store(move |store| {
                let record = store.session(id)?;
                let lane = store.lane(record.lane_id)?;
                Ok((record, lane))
            })
            .await
            .map_err(err)?;
        let active = lane.state == LaneState::Active;
        Ok(Launch {
            harness: record.harness,
            cwd: PathBuf::from(lane_cwd(&lane)),
            resume: record.locator,
            lane: Some(lane.id),
            // An archived lane has no worktree to edit; its session stays
            // readable without editing tools.
            read_only: !active,
        })
    }

    /// Starts a new Panoptes session in a lane's worktree. Its id, and so
    /// its key, exist before the harness starts.
    /// A lane that another Running or Waiting session has edited gets no
    /// session of its own until that one is Done: two writers otherwise.
    pub async fn create(&self, lane_id: i64, harness: &str) -> ServiceResult<OpenedSession> {
        self.harness(harness)?;
        let busy = {
            let open = self.open.lock().await;
            busy_writer(&open, &self.lane_writers, lane_id)
        };
        if busy.is_some() {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Another session is still changing this lane. Start one here once it is Done",
            ));
        }
        let name = harness.to_string();
        let (record, lane) = self
            .with_store(move |store| {
                let record = store.add_session(lane_id, &name)?;
                Ok((record, store.lane(lane_id)?))
            })
            .await
            .map_err(err)?;
        self.start_recorded(record.id, &lane, harness).await
    }

    /// Adopts a discovered harness conversation. With no explicit lane, a
    /// unique active worktree containing its cwd is selected; otherwise the
    /// UI asks for a lane. An open raw conversation must be closed first so
    /// adoption cannot leave a second, lock-free process behind.
    pub async fn adopt(
        &self,
        harness: &str,
        key: &str,
        locator: String,
        cwd: String,
        lane_id: Option<i64>,
    ) -> ServiceResult<AdoptionOutcome> {
        if panoptes_id(key).is_some() {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "This session already belongs to a lane",
            ));
        }
        let harness_id = key
            .strip_prefix(&format!("{harness}:"))
            .filter(|id| !id.is_empty())
            .ok_or_else(|| ServiceError::new(ErrorKind::Rejected, "Invalid harness session key"))?
            .to_string();
        let name = harness.to_string();
        let id_for_lookup = harness_id.clone();
        if let Some(id) = self
            .with_store(move |store| store.session_for(&name, &id_for_lookup))
            .await
            .map_err(err)?
        {
            return Ok(AdoptionOutcome::Adopted {
                key: panoptes_key(id),
            });
        }
        if self.open.lock().await.contains_key(key) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Stop this session before adopting it into a lane",
            ));
        }

        let lane_id = match lane_id {
            Some(id) => id,
            None => {
                let cwd = PathBuf::from(cwd);
                let occupied = self
                    .with_store(|store| store.sessions())
                    .await
                    .map_err(err)?
                    .into_iter()
                    .map(|record| record.lane_id)
                    .collect::<HashSet<_>>();
                let candidates = self
                    .with_store(|store| store.lanes())
                    .await
                    .map_err(err)?
                    .into_iter()
                    .filter(|lane| lane.state == LaneState::Active)
                    .filter(|lane| !occupied.contains(&lane.id))
                    .filter(|lane| {
                        lane.worktrees
                            .iter()
                            .any(|worktree| same_or_inside(&cwd, Path::new(&worktree.path)))
                    })
                    .map(|lane| lane.id)
                    .collect::<Vec<_>>();
                match candidates.as_slice() {
                    [id] => *id,
                    _ => return Ok(AdoptionOutcome::ChooseLane),
                }
            }
        };
        let harness = harness.to_string();
        let record = self
            .with_store(move |store| store.adopt_session(lane_id, &harness, &harness_id, &locator))
            .await
            .map_err(err)?;
        Ok(AdoptionOutcome::Adopted {
            key: panoptes_key(record.id),
        })
    }

    async fn start_recorded(
        &self,
        id: i64,
        lane: &Lane,
        harness: &str,
    ) -> ServiceResult<OpenedSession> {
        let launch = Launch {
            harness: harness.to_string(),
            cwd: PathBuf::from(lane_cwd(lane)),
            resume: None,
            lane: Some(lane.id),
            read_only: false,
        };
        let result = self.start(&panoptes_key(id), launch, None).await;
        if result.is_err() {
            let _ = self
                .with_store(move |store| store.discard_session(id))
                .await;
        }
        result
    }

    /// Starts a session's process under `key` and forwards its events.
    /// `status` carries a session's status when it is reopened.
    async fn start(
        &self,
        key: &str,
        launch: Launch,
        status: Option<Tracker>,
    ) -> ServiceResult<OpenedSession> {
        let harness = self.harness(&launch.harness)?;
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<HarnessEvent>();
        let fresh = status.is_none();
        let status = status.unwrap_or_default();
        let early_exit = EarlyExit::default();
        if fresh {
            (self.emit)(SessionEvent {
                key: key.to_string(),
                event: SessionUpdate::Status(StatusChanged {
                    status: SessionStatus::Idle,
                }),
            });
        }
        tokio::spawn(
            Forwarder {
                key: key.to_string(),
                generation,
                open: self.open.clone(),
                status: status.clone(),
                emit: self.emit.clone(),
                writes: WriteWatch {
                    writers: self.lane_writers.clone(),
                    store: self.store.clone(),
                    own: launch.lane,
                    cwd: launch.cwd.clone(),
                },
                early_exit: early_exit.clone(),
            }
            .run(rx),
        );

        let request = StartSession {
            cwd: launch.cwd,
            resume: launch.resume,
            read_only: launch.read_only,
        };
        let started = async {
            let session: Arc<dyn AgentSession> =
                Arc::from(harness.start_session(request, tx).await.map_err(err)?);
            let ready = async {
                let history = session.history().await.map_err(err)?;
                if let Some(id) = panoptes_id(key) {
                    let reference = session.harness_ref();
                    self.with_store(move |store| {
                        store.set_harness_ref(id, &reference.id, &reference.locator)
                    })
                    .await
                    .map_err(err)?;
                }
                Ok::<_, ServiceError>(history)
            };
            match ready.await {
                Ok(history) => Ok((session, history)),
                Err(error) => {
                    session.shutdown().await;
                    Err(error)
                }
            }
        };
        let (session, history) = match started.await {
            Ok(started) => started,
            Err(error) => {
                return Err(error);
            }
        };
        {
            let mut open = self.open.lock().await;
            let died = early_exit.lock().unwrap().take();
            if let Some(stderr_tail) = died {
                drop(open);
                session.shutdown().await;
                let detail = stderr_tail.trim();
                return Err(ServiceError::new(
                    ErrorKind::Exited,
                    if detail.is_empty() {
                        "The agent process ended while it was starting".to_string()
                    } else {
                        format!("The agent process ended while it was starting: {detail}")
                    },
                ));
            }
            open.insert(
                key.to_string(),
                OpenSession {
                    generation,
                    session,
                    status,
                    lane: launch.lane,
                },
            );
        }
        Ok(OpenedSession {
            key: key.to_string(),
            history,
        })
    }

    pub async fn history(&self, key: &str) -> ServiceResult<Vec<Message>> {
        self.session(key).await?.history().await.map_err(err)
    }

    /// Sends a prompt to the lane's one session.
    pub async fn prompt(
        &self,
        key: &str,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> ServiceResult<PromptOutcome> {
        self.session(key)
            .await?
            .prompt(text, images, mode)
            .await
            .map_err(err)
    }

    pub async fn settings(&self, key: &str) -> ServiceResult<SessionSettings> {
        self.session(key).await?.settings().await.map_err(err)
    }

    pub async fn models(&self, key: &str) -> ServiceResult<Vec<ModelInfo>> {
        self.session(key).await?.models().await.map_err(err)
    }

    /// Switches the session's model and returns its settings after the
    /// switch, since the thinking levels on offer depend on the model.
    pub async fn set_model(&self, key: &str, model_id: String) -> ServiceResult<SessionSettings> {
        let session = self.session(key).await?;
        session.set_model(model_id).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn set_thinking_level(
        &self,
        key: &str,
        level: String,
    ) -> ServiceResult<SessionSettings> {
        let session = self.session(key).await?;
        session.set_thinking_level(level).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn abort(&self, key: &str) -> ServiceResult<()> {
        self.session(key).await?.abort().await.map_err(err)
    }

    pub async fn answer(
        &self,
        key: &str,
        question_id: String,
        answer: Answer,
    ) -> ServiceResult<()> {
        let (session, status) = self
            .open
            .lock()
            .await
            .get(key)
            .map(|open| (open.session.clone(), open.status.clone()))
            .ok_or_else(not_open)?;
        session
            .answer(question_id.clone(), answer)
            .await
            .map_err(err)?;
        let mut tracker = status.lock().unwrap();
        if let Some(status) = tracker.answered(&question_id, now_ms()) {
            (self.emit)(SessionEvent {
                key: key.to_string(),
                event: SessionUpdate::Status(StatusChanged { status }),
            });
        }
        Ok(())
    }

    pub async fn close(&self, key: &str) {
        let open = self.open.lock().await.remove(key);
        if let Some(open) = open {
            open.session.shutdown().await;
        }
    }

    /// Ends every open session's process, all at once.
    pub async fn close_all(&self) {
        let sessions: Vec<_> = self
            .open
            .lock()
            .await
            .drain()
            .map(|(_, o)| o.session)
            .collect();
        let mut shutdowns = tokio::task::JoinSet::new();
        for session in sessions {
            shutdowns.spawn(async move { session.shutdown().await });
        }
        shutdowns.join_all().await;
    }

    /// Pins a Panoptes session in its record, and any other conversation by
    /// its harness key.
    pub async fn set_pinned(&self, key: String, pinned: bool) -> ServiceResult<()> {
        self.with_store(move |store| match panoptes_id(&key) {
            Some(id) => store.set_session_pinned(id, pinned),
            None => store.set_pinned(&key, pinned),
        })
        .await
        .map_err(err)
    }

    /// Archives a session, closing it first, or brings it back. Its lane is
    /// never touched.
    pub async fn set_archived(&self, key: &str, archived: bool) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("archived"))?;
        if archived {
            self.close(key).await;
        }
        self.with_store(move |store| store.set_session_archived(id, archived))
            .await
            .map_err(err)
    }

    pub async fn rename(&self, key: &str, title: Option<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("renamed"))?;
        self.with_store(move |store| store.set_session_title(id, title.as_deref()))
            .await
            .map_err(err)
    }

    pub async fn set_tags(&self, key: &str, tags: Vec<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("tagged"))?;
        self.with_store(move |store| store.set_session_tags(id, &tags))
            .await
            .map_err(err)
    }

    /// Registers the git repository `folder` is in, by its main checkout,
    /// with the default settings. Registering a repository twice fails.
    pub async fn add_project(&self, folder: String) -> ServiceResult<Project> {
        self.with_store(move |store| {
            let repo = projects::repository_of(Path::new(&folder))?;
            let settings = projects::validate(projects::default_settings(&repo))?;
            store.add_project(&repo.to_string_lossy(), &settings)
        })
        .await
        .map_err(err)
    }

    pub async fn update_project(
        &self,
        id: i64,
        settings: ProjectSettings,
    ) -> ServiceResult<Project> {
        self.with_store(move |store| store.update_project(id, &projects::validate(settings)?))
            .await
            .map_err(err)
    }

    /// Forgets a project. Nothing on disk changes, and its sessions are
    /// listed by folder again.
    pub async fn remove_project(&self, id: i64) -> ServiceResult<()> {
        self.with_store(move |store| store.remove_project(id))
            .await
            .map_err(err)
    }

    /// Creates a durable lane before any session exists. One creation per
    /// project at a time keeps name allocation and the
    /// one-active-main-checkout rule consistent; the store re-checks paths.
    pub async fn create_lane(
        &self,
        project_id: i64,
        task: String,
        main_checkout: bool,
    ) -> ServiceResult<Lane> {
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.lane_cancellations.lock().unwrap();
            if active.contains_key(&project_id) {
                return Err(ServiceError::new(
                    ErrorKind::Rejected,
                    "A lane is already being created for this project",
                ));
            }
            active.insert(project_id, cancelled.clone());
        }
        let root = self.lanes_root.clone();
        let in_setup = cancelled.clone();
        let store = self.store.clone();
        // The store lock is taken only for the reads and the final insert.
        // Git and the setup command can take minutes and must not block
        // every other store call. Creations for one project are serialised
        // by the cancellation map above.
        let result = tokio::task::spawn_blocking(move || {
            let task = task.trim().to_string();
            if task.is_empty() {
                anyhow::bail!("A lane needs a task");
            }
            let (project, taken) = {
                let store = store.lock().unwrap();
                let project = store.project(project_id)?;
                let lanes = store.lanes()?;
                if main_checkout && has_active_main_checkout(&lanes, project_id, None) {
                    anyhow::bail!("This project already has an active main-checkout lane");
                }
                let taken: Vec<String> = lanes
                    .into_iter()
                    .filter(|lane| lane.project_id == project_id)
                    .map(|lane| lane.name)
                    .collect();
                (project, taken)
            };
            let new = lanes::create(
                &root,
                &project,
                &lanes::slug(&task),
                &taken,
                &task,
                main_checkout,
                &in_setup,
            )?;
            let added = store.lock().unwrap().add_lane(&new);
            added.inspect_err(|_| lanes::discard_created(&project, &new))
        })
        .await
        .context("The lane creation stopped")
        .and_then(|result| result);
        self.lane_cancellations.lock().unwrap().remove(&project_id);
        match result {
            Ok(lane) => Ok(lane),
            Err(_) if cancelled.load(Ordering::Relaxed) => Err(ServiceError::new(
                ErrorKind::Cancelled,
                "Lane creation cancelled",
            )),
            Err(error) => Err(err(error)),
        }
    }

    pub fn cancel_lane_creation(&self, project_id: i64) {
        if let Some(cancelled) = self.lane_cancellations.lock().unwrap().get(&project_id) {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Archives a clean lane immediately. A dirty one is returned unchanged
    /// until the caller picks how its work should be preserved.
    pub async fn archive_lane(
        &self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
    ) -> ServiceResult<ArchiveLaneOutcome> {
        let _busy = self.lane_busy(id)?;
        if self
            .open
            .lock()
            .await
            .values()
            .any(|open| open.lane == Some(id))
        {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's sessions before archiving it",
            ));
        }
        let store = self.store.clone();
        // As in `create_lane`, Git (and its commit hooks) runs without the
        // store lock; `lane_busy` keeps two changes to one lane apart.
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock().unwrap();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Active {
                    anyhow::bail!("That lane is already archived");
                }
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            // The checkpoint ref is stored before the folder goes, so an
            // archive that stops in between can be finished later.
            let record =
                |reference: Option<&str>| store.lock().unwrap().set_lane_checkpoint(id, reference);
            match lanes::archive_recording(&project, &lane, strategy, &record)? {
                ArchiveResult::Dirty => Ok(ArchiveLaneOutcome::Dirty { lane }),
                ArchiveResult::Archived { checkpoint_ref } => {
                    store
                        .lock()
                        .unwrap()
                        .archive_lane(id, checkpoint_ref.as_deref())?;
                    Ok(ArchiveLaneOutcome::Archived)
                }
            }
        })
        .await
        .context("The lane archive stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    pub async fn restore_lane(&self, id: i64) -> ServiceResult<Lane> {
        let _busy = self.lane_busy(id)?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock().unwrap();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Archived {
                    anyhow::bail!("That lane is not archived");
                }
                let lanes = store.lanes()?;
                let main = lane.worktrees.iter().any(|tree| tree.main_checkout);
                if main && has_active_main_checkout(&lanes, lane.project_id, Some(id)) {
                    anyhow::bail!("This project already has an active main-checkout lane");
                }
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            lanes::restore(&project, &lane)?;
            store.lock().unwrap().restore_lane(id)
        })
        .await
        .context("The lane restore stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Permanently deletes a lane after the UI's explicit confirmation.
    /// Open sessions are refused rather than killed underneath their user.
    pub async fn delete_lane(&self, id: i64) -> ServiceResult<()> {
        let _busy = self.lane_busy(id)?;
        if self
            .open
            .lock()
            .await
            .values()
            .any(|open| open.lane == Some(id))
        {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's session before deleting it",
            ));
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock().unwrap();
                let lane = store.lane(id)?;
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            lanes::delete(&project, &lane)?;
            store.lock().unwrap().delete_lane(id)
        })
        .await
        .context("The lane deletion stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Marks a lane as being archived, restored or deleted until the guard drops.
    fn lane_busy(&self, id: i64) -> ServiceResult<LaneBusy> {
        if !self.busy_lanes.lock().unwrap().insert(id) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "That lane is already being archived, restored or deleted",
            ));
        }
        Ok(LaneBusy {
            lanes: self.busy_lanes.clone(),
            id,
        })
    }

    /// Runs `call` on a blocking thread: SQLite calls block, for up to the
    /// busy timeout when another process holds the database, and must not
    /// stall the async runtime or the main thread.
    async fn with_store<T: Send + 'static>(
        &self,
        call: impl FnOnce(&Store) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || call(&store.lock().unwrap()))
            .await
            .context("The store call stopped")?
    }
}

struct LaneBusy {
    lanes: Arc<std::sync::Mutex<HashSet<i64>>>,
    id: i64,
}

impl Drop for LaneBusy {
    fn drop(&mut self) {
        self.lanes.lock().unwrap().remove(&self.id);
    }
}

/// The key of a session other than `lane_id`'s own that has edited the lane
/// and is Running or Waiting, if there is one.
fn busy_writer(
    open: &HashMap<String, OpenSession>,
    writers: &LaneWriters,
    lane_id: i64,
) -> Option<String> {
    let writers = writers.lock().unwrap();
    let mut keys: Vec<&String> = writers.get(&lane_id)?.iter().collect();
    keys.sort();
    keys.into_iter()
        .find(|key| {
            open.get(*key).is_some_and(|session| {
                session.lane != Some(lane_id)
                    && matches!(
                        session.status.lock().unwrap().status,
                        SessionStatus::Running | SessionStatus::Waiting
                    )
            })
        })
        .cloned()
}

/// Whether another active lane of `project_id` already uses the main checkout.
fn has_active_main_checkout(lanes: &[Lane], project_id: i64, except: Option<i64>) -> bool {
    lanes.iter().any(|lane| {
        lane.project_id == project_id
            && Some(lane.id) != except
            && lane.state == LaneState::Active
            && lane.worktrees.iter().any(|tree| tree.main_checkout)
    })
}

/// Where a lane's sessions run: its (for now only) worktree.
fn lane_cwd(lane: &Lane) -> String {
    lane.worktrees
        .first()
        .map(|worktree| worktree.path.clone())
        .unwrap_or_default()
}

fn same_or_inside(path: &Path, root: &Path) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.starts_with(root)
}

fn cached_location(cache: &mut HashMap<String, Location>, cwd: &str) -> Location {
    cache
        .entry(cwd.to_string())
        .or_insert_with(|| projects::locate(Path::new(cwd)))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use panoptes_harness::EventSink;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// A harness whose sessions are in-memory; it counts how many processes
    /// it was asked to start.
    #[derive(Default)]
    struct FakeHarness {
        starts: AtomicUsize,
        fail_listing: bool,
        /// Where its sessions run, one session per folder; `/tmp` when empty.
        cwds: Vec<String>,
        sinks: std::sync::Mutex<Vec<EventSink>>,
        /// Every start it was asked for, in order.
        requests: std::sync::Mutex<Vec<StartSession>>,
        /// Its process ends right after it reports its session.
        dies_on_start: bool,
    }

    impl FakeHarness {
        /// Which starts were read-only, in order.
        fn read_only(&self) -> Vec<bool> {
            let requests = self.requests.lock().unwrap();
            requests.iter().map(|request| request.read_only).collect()
        }
    }

    struct FakeSession {
        id: String,
        cwd: String,
        events: EventSink,
    }

    #[async_trait]
    impl Harness for FakeHarness {
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn list_sessions(&self) -> anyhow::Result<Vec<DiscoveredSession>> {
            if self.fail_listing {
                anyhow::bail!("disk on fire");
            }
            let cwds = if self.cwds.is_empty() {
                vec!["/tmp".to_string()]
            } else {
                self.cwds.clone()
            };
            Ok(cwds
                .into_iter()
                .enumerate()
                .map(|(i, cwd)| DiscoveredSession {
                    harness_ref: HarnessRef {
                        harness: "fake".into(),
                        id: format!("s{}", i + 1),
                        locator: format!("s{}", i + 1),
                        cwd,
                    },
                    title: "One".into(),
                    created_at: 0,
                    updated_at: 0,
                    message_count: 0,
                    parent_locator: None,
                })
                .collect())
        }

        async fn start_session(
            &self,
            request: StartSession,
            events: EventSink,
        ) -> anyhow::Result<Box<dyn AgentSession>> {
            let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
            // Slow enough that concurrent opens overlap.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let id = request
                .resume
                .clone()
                .unwrap_or_else(|| format!("new-{start}"));
            let cwd = request.cwd.to_string_lossy().into_owned();
            self.requests.lock().unwrap().push(request);
            let _ = events.send(HarnessEvent::Raw {
                payload: serde_json::json!("early"),
            });
            self.sinks.lock().unwrap().push(events.clone());
            if self.dies_on_start {
                let _ = events.send(HarnessEvent::Exited {
                    code: Some(1),
                    stderr_tail: "no model configured\n".into(),
                });
                // Long enough for the exit to be seen before the session
                // would be listed as open.
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok(Box::new(FakeSession { id, cwd, events }))
        }
    }

    #[async_trait]
    impl AgentSession for FakeSession {
        fn harness_ref(&self) -> HarnessRef {
            HarnessRef {
                harness: "fake".into(),
                id: self.id.clone(),
                locator: self.id.clone(),
                cwd: self.cwd.clone(),
            }
        }
        async fn history(&self) -> anyhow::Result<Vec<Message>> {
            Ok(Vec::new())
        }
        async fn prompt(
            &self,
            _: String,
            _: Vec<ImageAttachment>,
            _: PromptMode,
        ) -> anyhow::Result<PromptOutcome> {
            Ok(PromptOutcome::Started)
        }
        async fn abort(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn answer(&self, _: String, _: Answer) -> anyhow::Result<()> {
            Ok(())
        }
        async fn shutdown(&self) {
            let _ = self.events.send(HarnessEvent::Exited {
                code: Some(0),
                stderr_tail: String::new(),
            });
        }
    }

    struct Fixture {
        service: Arc<Service>,
        harness: Arc<FakeHarness>,
        events: Arc<std::sync::Mutex<Vec<SessionEvent>>>,
        _dir: tempfile::TempDir,
    }

    fn fixture(harness: FakeHarness) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let harness = Arc::new(harness);
        let events: Arc<std::sync::Mutex<Vec<SessionEvent>>> = Arc::default();
        let sink = events.clone();
        let service = Service::new(
            vec![harness.clone()],
            Store::open(dir.path()).unwrap(),
            dir.path().join("lanes"),
            Arc::new(move |event| sink.lock().unwrap().push(event)),
        );
        Fixture {
            service: Arc::new(service),
            harness,
            events,
            _dir: dir,
        }
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    fn exits(fixture: &Fixture) -> usize {
        let events = fixture.events.lock().unwrap();
        events
            .iter()
            .filter(|e| matches!(e.event, SessionUpdate::Harness(HarnessEvent::Exited { .. })))
            .count()
    }

    fn statuses(fixture: &Fixture) -> Vec<SessionStatus> {
        let events = fixture.events.lock().unwrap();
        events
            .iter()
            .filter_map(|e| match e.event {
                SessionUpdate::Status(StatusChanged { status }) => Some(status),
                _ => None,
            })
            .collect()
    }

    async fn open_s1(fixture: &Fixture) -> EventSink {
        fixture
            .service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap();
        fixture.harness.sinks.lock().unwrap()[0].clone()
    }

    fn assistant(error: Option<&str>) -> HarnessEvent {
        HarnessEvent::MessageCompleted {
            message: Message {
                id: "m1".into(),
                role: Role::Assistant,
                blocks: Vec::new(),
                timestamp: None,
                error: error.map(Into::into),
                stopped: false,
            },
        }
    }

    fn question(id: &str, expires_at: Option<i64>) -> HarnessEvent {
        HarnessEvent::Question {
            question: panoptes_harness::Question {
                id: id.into(),
                title: "Proceed?".into(),
                message: None,
                kind: panoptes_harness::QuestionKind::Confirm,
                expires_at,
            },
        }
    }

    #[tokio::test]
    async fn concurrent_opens_start_one_process() {
        let f = fixture(FakeHarness::default());
        let open = |service: Arc<Service>| async move {
            service
                .open("fake", "fake:s1", "s1".into(), "/tmp".into())
                .await
        };
        let (a, b) = tokio::join!(open(f.service.clone()), open(f.service.clone()));
        assert_eq!(a.unwrap().key, "fake:s1");
        assert_eq!(b.unwrap().key, "fake:s1");
        assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
    }

    /// A registered repository with one main-checkout lane, which needs no
    /// worktree on disk.
    async fn lane_of(f: &Fixture) -> (tempfile::TempDir, Lane) {
        let code = tempfile::tempdir().unwrap();
        let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let lane = f
            .service
            .create_lane(project.id, "Fix login".into(), true)
            .await
            .unwrap();
        (code, lane)
    }

    #[tokio::test]
    async fn a_lane_session_has_its_key_before_the_harness_starts() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        let opened = f.service.create(lane.id, "fake").await.unwrap();
        assert_eq!(opened.key, "panoptes:1");
        settle().await;
        {
            // Nothing is held back: even what the harness says while it
            // starts carries the Panoptes key.
            let events = f.events.lock().unwrap();
            assert!(events.iter().all(|e| e.key == opened.key), "{events:?}");
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e.event, SessionUpdate::Harness(HarnessEvent::Raw { .. })))
            );
        }
        let listing = f.service.list().await;
        let session = listing
            .sessions
            .iter()
            .find(|s| s.key == opened.key)
            .unwrap();
        assert_eq!(session.id, Some(1));
        assert_eq!(session.lane_id, Some(lane.id));
        assert_eq!(session.project_id, Some(lane.project_id));
        assert_eq!(session.cwd, lane.worktrees[0].path);
        assert_eq!(session.title, "New session");
        assert_eq!(session.harness_ref.as_ref().unwrap().id, "new-1");
        assert!(session.open);

        // Its conversation, opened by the harness key, is the same session.
        let again = f
            .service
            .open("fake", "fake:new-1", "new-1".into(), "/tmp".into())
            .await
            .unwrap();
        assert_eq!(again.key, opened.key);
        assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_adopted_conversation_is_listed_once_as_its_lane_session() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        let id = {
            let store = f.service.store.lock().unwrap();
            let record = store.add_session(lane.id, "fake").unwrap();
            store.set_harness_ref(record.id, "s1", "s1").unwrap();
            record.id
        };
        let key = panoptes_key(id);
        let listing = f.service.list().await;
        assert_eq!(listing.sessions.len(), 1, "{:?}", listing.sessions);
        assert_eq!(listing.sessions[0].key, key);
        assert_eq!(listing.sessions[0].title, "One");
        assert_eq!(listing.navigation.len(), 1);
        assert_eq!(listing.navigation[0].lane_id, lane.id);
        assert_eq!(listing.navigation[0].sessions[0].key, key);

        f.service.set_pinned(key.clone(), true).await.unwrap();
        f.service.rename(&key, Some("Login".into())).await.unwrap();
        f.service.set_tags(&key, vec!["auth".into()]).await.unwrap();
        let session = &f.service.list().await.sessions[0];
        assert!(session.pinned);
        assert_eq!(session.title, "Login");
        assert_eq!(session.tags, ["auth"]);
        let pins = Store::open(f._dir.path()).unwrap().pinned().unwrap();
        assert!(pins.is_empty(), "{pins:?}");
        let error = f.service.set_archived("fake:s1", true).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Rejected);

        f.service.set_archived(&key, true).await.unwrap();
        let listing = f.service.list().await;
        assert!(listing.sessions[0].archived, "All sessions keeps it");
        assert!(listing.navigation[0].sessions.is_empty());
        assert_eq!(listing.navigation[0].archived_sessions[0].key, key);
    }

    #[tokio::test]
    async fn adopting_uses_a_unique_worktree_or_asks_for_a_lane() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        f.service.set_pinned("fake:s1".into(), true).await.unwrap();

        let outcome = f
            .service
            .adopt(
                "fake",
                "fake:s1",
                "s1".into(),
                lane.worktrees[0].path.clone(),
                None,
            )
            .await
            .unwrap();
        let AdoptionOutcome::Adopted { key } = outcome else {
            panic!("a unique worktree should adopt directly")
        };
        let listing = f.service.list().await;
        assert_eq!(listing.sessions.len(), 1);
        assert_eq!(listing.sessions[0].key, key);
        assert!(listing.sessions[0].pinned);
        assert!(
            Store::open(f._dir.path())
                .unwrap()
                .pinned()
                .unwrap()
                .is_empty()
        );
        assert_eq!(listing.navigation[0].sessions[0].key, key);

        let outcome = f
            .service
            .adopt(
                "fake",
                "fake:s2",
                "s2".into(),
                lane.worktrees[0].path.clone(),
                None,
            )
            .await
            .unwrap();
        assert!(matches!(outcome, AdoptionOutcome::ChooseLane));
        let outcome = f
            .service
            .adopt(
                "fake",
                "fake:s2",
                "s2".into(),
                "/somewhere/else".into(),
                None,
            )
            .await
            .unwrap();
        assert!(matches!(outcome, AdoptionOutcome::ChooseLane));
        let outcome = f
            .service
            .adopt(
                "fake",
                "fake:s2",
                "s2".into(),
                "/somewhere/else".into(),
                Some(lane.id),
            )
            .await
            .unwrap_err();
        assert!(outcome.message.contains("already has a session"));
        let listing = f.service.list().await;
        assert_eq!(listing.sessions.len(), 1);
        assert_eq!(listing.navigation[0].sessions.len(), 1);
    }

    #[tokio::test]
    async fn an_open_raw_session_must_stop_before_adoption() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        f.service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap();
        let error = f
            .service
            .adopt("fake", "fake:s1", "s1".into(), "/tmp".into(), Some(lane.id))
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Rejected);
        assert!(error.message.contains("Stop this session"));
    }

    #[tokio::test]
    async fn a_lane_rejects_a_second_session_even_if_the_first_is_archived() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        let key = f.service.create(lane.id, "fake").await.unwrap().key;
        let error = f.service.create(lane.id, "fake").await.unwrap_err();
        assert!(error.message.contains("already has a session"));
        assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);

        f.service.set_archived(&key, true).await.unwrap();
        let error = f.service.create(lane.id, "fake").await.unwrap_err();
        assert!(error.message.contains("already has a session"));
        let error = f
            .service
            .adopt(
                "fake",
                "fake:s2",
                "s2".into(),
                lane.worktrees[0].path.clone(),
                Some(lane.id),
            )
            .await
            .unwrap_err();
        assert!(error.message.contains("already has a session"));
        let listing = f.service.list().await;
        assert!(listing.navigation[0].sessions.is_empty());
        assert_eq!(listing.navigation[0].archived_sessions.len(), 1);
    }

    #[tokio::test]
    async fn an_archived_lane_reopens_its_existing_session_read_only() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        let key = f.service.create(lane.id, "fake").await.unwrap().key;
        f.service.close(&key).await;
        f.service.archive_lane(lane.id, None).await.unwrap();
        assert!(f.service.create(lane.id, "fake").await.is_err());

        f.service
            .open("fake", &key, String::new(), String::new())
            .await
            .unwrap();
        assert_eq!(f.harness.read_only(), [false, true]);
    }

    #[tokio::test]
    async fn closing_a_session_does_not_report_an_exit() {
        let f = fixture(FakeHarness::default());
        f.service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap();
        f.service.close("fake:s1").await;
        settle().await;
        assert_eq!(exits(&f), 0);
        assert!(!f.service.list().await.sessions[0].open);
    }

    #[tokio::test]
    async fn a_process_that_dies_while_starting_is_not_left_open() {
        let f = fixture(FakeHarness {
            dies_on_start: true,
            ..FakeHarness::default()
        });
        let error = f
            .service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Exited);
        assert!(error.message.contains("no model configured"), "{error}");
        assert!(!f.service.list().await.sessions[0].open);
        let error = f.service.history("fake:s1").await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotOpen);
    }

    #[tokio::test]
    async fn a_crash_is_reported_and_closes_the_session() {
        let f = fixture(FakeHarness::default());
        f.service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap();
        assert!(f.service.list().await.sessions[0].open);

        let sink = f.harness.sinks.lock().unwrap()[0].clone();
        sink.send(HarnessEvent::Exited {
            code: Some(1),
            stderr_tail: "boom".into(),
        })
        .unwrap();
        settle().await;
        assert_eq!(exits(&f), 1);
        assert_eq!(statuses(&f).last(), Some(&SessionStatus::Failed));
        assert!(!f.service.list().await.sessions[0].open);
        let error = f.service.history("fake:s1").await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotOpen);
    }

    #[tokio::test]
    async fn a_run_goes_from_running_through_waiting_to_done() {
        let f = fixture(FakeHarness::default());
        let sink = open_s1(&f).await;
        settle().await;
        assert_eq!(statuses(&f), [SessionStatus::Idle]);

        sink.send(HarnessEvent::RunStarted).unwrap();
        sink.send(question("q1", None)).unwrap();
        settle().await;
        f.service
            .answer("fake:s1", "q1".into(), Answer::Cancelled)
            .await
            .unwrap();
        sink.send(assistant(None)).unwrap();
        sink.send(HarnessEvent::RunSettled { aborted: false })
            .unwrap();
        settle().await;
        assert_eq!(
            statuses(&f),
            [
                SessionStatus::Idle,
                SessionStatus::Running,
                SessionStatus::Waiting,
                SessionStatus::Running,
                SessionStatus::Done,
            ]
        );
    }

    #[tokio::test]
    async fn a_run_that_ends_on_an_error_fails() {
        let f = fixture(FakeHarness::default());
        let sink = open_s1(&f).await;
        sink.send(HarnessEvent::RunStarted).unwrap();
        sink.send(assistant(Some("overloaded"))).unwrap();
        sink.send(HarnessEvent::RunSettled { aborted: false })
            .unwrap();
        settle().await;
        assert_eq!(statuses(&f).last(), Some(&SessionStatus::Failed));

        // The next run starts clean, and a retried error does not count.
        sink.send(HarnessEvent::RunStarted).unwrap();
        sink.send(assistant(Some("overloaded"))).unwrap();
        sink.send(assistant(None)).unwrap();
        sink.send(HarnessEvent::RunSettled { aborted: false })
            .unwrap();
        settle().await;
        assert_eq!(
            statuses(&f)[3..],
            [SessionStatus::Running, SessionStatus::Done]
        );
    }

    #[test]
    fn an_approval_waits_until_decided() {
        let mut tracker = StatusTracker::default();
        tracker.apply(&HarnessEvent::RunStarted, 0);
        let approval = HarnessEvent::Approval {
            approval: panoptes_harness::Approval {
                id: "a1".into(),
                tool_call_id: "c1".into(),
                title: "Allow?".into(),
                message: None,
                expires_at: None,
            },
        };
        assert_eq!(tracker.apply(&approval, 10), Some(SessionStatus::Waiting));
        assert_eq!(tracker.answered("a1", 20), Some(SessionStatus::Running));
    }

    #[test]
    fn an_expired_question_stops_waiting() {
        let mut tracker = StatusTracker::default();
        tracker.apply(&HarnessEvent::RunStarted, 0);
        assert_eq!(
            tracker.apply(&question("q1", Some(100)), 50),
            Some(SessionStatus::Waiting)
        );
        let notice = HarnessEvent::Notice {
            level: panoptes_harness::NoticeLevel::Info,
            text: "still going".into(),
        };
        assert_eq!(tracker.apply(&notice, 150), Some(SessionStatus::Running));
    }

    #[test]
    fn status_events_serialise_like_harness_events() {
        let update = SessionUpdate::Status(StatusChanged {
            status: SessionStatus::Waiting,
        });
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({"type": "statusChanged", "status": "waiting"})
        );
        let update = SessionUpdate::Harness(HarnessEvent::RunStarted);
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            serde_json::json!({"type": "runStarted"})
        );
    }

    #[test]
    fn harness_errors_keep_their_kind() {
        let error: ServiceError = anyhow::Error::from(HarnessError::timed_out("slow"))
            .context("Could not prompt")
            .into();
        assert_eq!(error.kind, ErrorKind::TimedOut);
        assert_eq!(error.message, "Could not prompt: slow");
        let error: ServiceError = anyhow::anyhow!("disk full").into();
        assert_eq!(error.kind, ErrorKind::Other);
        assert_eq!(
            serde_json::to_value(ServiceError::new(ErrorKind::NotOpen, "x")).unwrap(),
            serde_json::json!({"kind": "notOpen", "message": "x"})
        );
    }

    #[tokio::test]
    async fn a_failing_harness_is_reported_not_fatal() {
        let f = fixture(FakeHarness {
            fail_listing: true,
            ..Default::default()
        });
        let listing = f.service.list().await;
        assert!(listing.sessions.is_empty());
        assert_eq!(
            listing.errors,
            ["Could not list fake sessions: disk on fire"]
        );
    }

    #[tokio::test]
    async fn pins_are_saved_to_the_store() {
        let f = fixture(FakeHarness::default());
        f.service.set_pinned("fake:s1".into(), true).await.unwrap();
        assert!(f.service.list().await.sessions[0].pinned);
        let saved = Store::open(f._dir.path()).unwrap().pinned().unwrap();
        assert!(saved.contains("fake:s1"));

        f.service.set_pinned("fake:s1".into(), false).await.unwrap();
        assert!(!f.service.list().await.sessions[0].pinned);
    }

    #[tokio::test]
    async fn unreadable_pins_are_reported_and_the_sessions_still_listed() {
        let f = fixture(FakeHarness::default());
        f.service.set_pinned("fake:s1".into(), true).await.unwrap();
        rusqlite::Connection::open(f._dir.path().join("panoptes.db"))
            .unwrap()
            .execute_batch("DROP TABLE pins;")
            .unwrap();

        let listing = f.service.list().await;
        assert_eq!(listing.sessions.len(), 1);
        assert!(!listing.sessions[0].pinned);
        assert_eq!(listing.errors.len(), 1);
        assert!(
            listing.errors[0].starts_with("Could not read pins:"),
            "{:?}",
            listing.errors
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
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

    #[tokio::test]
    async fn sessions_are_matched_to_projects_by_their_folder() {
        let code = tempfile::tempdir().unwrap();
        let code = std::fs::canonicalize(code.path()).unwrap();
        let repo = code.join("web");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let worktree = code.join("workspaces/fix/web");
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
        let other = code.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let f = fixture(FakeHarness {
            cwds: vec![path(&repo.join("src")), path(&worktree), path(&other)],
            ..FakeHarness::default()
        });

        // Registering from a worktree registers the main checkout.
        let project = f.service.add_project(path(&worktree)).await.unwrap();
        assert_eq!(project.repo, path(&repo));
        assert_eq!(project.settings.name, "web");
        assert_eq!(project.settings.base_branch, "trunk");
        assert_eq!(project.settings.copy_files, [".env"]);
        let again = f.service.add_project(path(&repo)).await.unwrap_err();
        assert!(again.message.contains("already registered"), "{again}");

        let listing = f.service.list().await;
        assert_eq!(listing.projects, std::slice::from_ref(&project));
        let matched: Vec<_> = listing.sessions.iter().map(|s| s.project_id).collect();
        assert_eq!(matched, [Some(project.id), Some(project.id), None]);
        assert_eq!(listing.sessions[1].project_root, path(&worktree));
        assert_eq!(listing.sessions[1].checkout, Some(path(&repo)));
        assert_eq!(listing.sessions[2].checkout, None);

        f.service.remove_project(project.id).await.unwrap();
        let listing = f.service.list().await;
        assert!(listing.projects.is_empty());
        assert!(listing.sessions.iter().all(|s| s.project_id.is_none()));
    }

    #[tokio::test]
    async fn project_settings_are_checked_before_they_are_saved() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "--quiet"]);
        let f = fixture(FakeHarness::default());
        let project = f
            .service
            .add_project(repo.path().to_string_lossy().into_owned())
            .await
            .unwrap();

        let mut settings = project.settings.clone();
        settings.copy_files = vec!["../outside".into()];
        let error = f
            .service
            .update_project(project.id, settings)
            .await
            .unwrap_err();
        assert!(error.message.contains("not a path inside"), "{error}");

        let mut settings = project.settings.clone();
        settings.setup = Some(" pnpm install ".into());
        let saved = f
            .service
            .update_project(project.id, settings)
            .await
            .unwrap();
        assert_eq!(saved.settings.setup.as_deref(), Some("pnpm install"));
        assert_eq!(f.service.list().await.projects, [saved]);

        let outside = tempfile::tempdir().unwrap();
        let error = f
            .service
            .add_project(outside.path().to_string_lossy().into_owned())
            .await
            .unwrap_err();
        assert!(error.message.contains("not in a git repository"), "{error}");
    }

    #[tokio::test]
    async fn lanes_exist_without_sessions_and_dirty_archives_wait_for_a_choice() {
        let code = tempfile::tempdir().unwrap();
        let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("tracked.txt"), "start\n").unwrap();
        std::fs::write(repo.join(".env"), "secret\n").unwrap();
        git(&repo, &["add", "tracked.txt"]);
        git(&repo, &["commit", "--quiet", "-m", "start"]);
        let f = fixture(FakeHarness::default());
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let mut settings = project.settings.clone();
        settings.setup = Some("printf ready > setup.txt".into());
        f.service
            .update_project(project.id, settings)
            .await
            .unwrap();

        let lane = f
            .service
            .create_lane(project.id, " Fix login! ".into(), false)
            .await
            .unwrap();
        assert_eq!(lane.name, "fix-login");
        assert_eq!(lane.task, "Fix login!");
        let path = PathBuf::from(&lane.worktrees[0].path);
        assert_eq!(
            std::fs::read_to_string(path.join(".env")).unwrap(),
            "secret\n"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("setup.txt")).unwrap(),
            "ready"
        );
        assert_eq!(f.service.list().await.lanes, std::slice::from_ref(&lane));

        let second = f
            .service
            .create_lane(project.id, "Fix login".into(), false)
            .await
            .unwrap();
        assert_eq!(second.name, "fix-login-2");
        std::fs::write(path.join("tracked.txt"), "work\n").unwrap();
        let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
        assert!(matches!(outcome, ArchiveLaneOutcome::Dirty { .. }));
        assert!(path.exists());
        assert_eq!(f.service.list().await.lanes[0].state, LaneState::Active);

        let outcome = f
            .service
            .archive_lane(lane.id, Some(ArchiveStrategy::Checkpoint))
            .await
            .unwrap();
        assert!(matches!(outcome, ArchiveLaneOutcome::Archived));
        assert!(!path.exists());
        let archived = &f.service.list().await.lanes[0];
        assert_eq!(archived.state, LaneState::Archived);
        assert!(archived.worktrees[0].checkpoint_ref.is_some());

        let restored = f.service.restore_lane(lane.id).await.unwrap();
        assert_eq!(restored.state, LaneState::Active);
        assert_eq!(
            std::fs::read_to_string(path.join("tracked.txt")).unwrap(),
            "work\n"
        );
    }

    /// A project with two worktree lanes, and a session started in the first.
    async fn two_lanes(f: &Fixture) -> (tempfile::TempDir, Lane, Lane, EventSink) {
        let code = tempfile::tempdir().unwrap();
        let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("tracked.txt"), "start\n").unwrap();
        git(&repo, &["add", "tracked.txt"]);
        git(&repo, &["commit", "--quiet", "-m", "start"]);
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let a = f
            .service
            .create_lane(project.id, "Plan the work".into(), false)
            .await
            .unwrap();
        let b = f
            .service
            .create_lane(project.id, "Build the parser".into(), false)
            .await
            .unwrap();
        f.service.create(a.id, "fake").await.unwrap();
        let sink = f.harness.sinks.lock().unwrap()[0].clone();
        (code, a, b, sink)
    }

    fn edit(id: &str, path: &str) -> HarnessEvent {
        HarnessEvent::ToolCallUpdated {
            call: Box::new(panoptes_harness::ToolCall {
                id: id.into(),
                name: "edit".into(),
                kind: panoptes_harness::ToolKind::Edit,
                subject: Some(path.into()),
                input: serde_json::Value::Null,
                status: panoptes_harness::ToolStatus::Done,
                output: None,
                started_at: None,
                ended_at: None,
            }),
        }
    }

    fn entry(listing: &Listing, lane: i64) -> &LaneNavigation {
        listing
            .navigation
            .iter()
            .find(|entry| entry.lane_id == lane)
            .unwrap()
    }

    /// Lists until `done` holds, for what the forwarder records on its own.
    async fn eventually(f: &Fixture, done: impl Fn(&Listing) -> bool) -> Listing {
        for _ in 0..100 {
            let listing = f.service.list().await;
            if done(&listing) {
                return listing;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("never happened: {:?}", f.service.list().await.navigation);
    }

    #[tokio::test]
    async fn a_session_editing_another_lane_holds_back_that_lanes_own_session() {
        let f = fixture(FakeHarness::default());
        let (_code, a, b, sink) = two_lanes(&f).await;
        let b_path = PathBuf::from(&b.worktrees[0].path);

        let listing = f.service.list().await;
        assert_eq!(
            entry(&listing, b.id).work,
            Some(LaneWork {
                changed_files: 0,
                commits_ahead: Some(0)
            })
        );
        assert!(entry(&listing, b.id).busy_writer.is_none());

        sink.send(HarnessEvent::RunStarted).unwrap();
        // Its own worktree is not another lane's.
        sink.send(edit("t1", "own.txt")).unwrap();
        std::fs::write(b_path.join("parser.rs"), "fn parse() {}\n").unwrap();
        sink.send(edit("t2", &b_path.join("parser.rs").to_string_lossy()))
            .unwrap();
        let listing = eventually(&f, |l| entry(l, b.id).busy_writer.is_some()).await;
        let writer = entry(&listing, b.id).busy_writer.clone().unwrap();
        assert_eq!(writer.session_key, "panoptes:1");
        assert_eq!(writer.title, "New session");
        assert!(entry(&listing, a.id).busy_writer.is_none());
        assert_eq!(entry(&listing, b.id).work.unwrap().changed_files, 1);
        assert!(
            f.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.event == SessionUpdate::LaneWrites(LaneWritesChanged { lane_id: b.id }))
        );

        let refused = f.service.create(b.id, "fake").await.unwrap_err();
        assert_eq!(refused.kind, ErrorKind::Rejected);

        // Once the writer is Done, the lane can have its own session.
        sink.send(HarnessEvent::RunSettled { aborted: false })
            .unwrap();
        let listing = eventually(&f, |l| entry(l, b.id).busy_writer.is_none()).await;
        assert_eq!(entry(&listing, b.id).work.unwrap().changed_files, 1);
        assert_eq!(
            f.service.create(b.id, "fake").await.unwrap().key,
            "panoptes:2"
        );
    }

    #[tokio::test]
    async fn a_writer_that_stops_no_longer_holds_a_lane_back() {
        let f = fixture(FakeHarness::default());
        let (_code, _a, b, sink) = two_lanes(&f).await;
        let b_path = PathBuf::from(&b.worktrees[0].path);
        sink.send(HarnessEvent::RunStarted).unwrap();
        sink.send(edit("t1", &b_path.join("x.rs").to_string_lossy()))
            .unwrap();
        eventually(&f, |l| entry(l, b.id).busy_writer.is_some()).await;
        f.service.close("panoptes:1").await;
        eventually(&f, |l| entry(l, b.id).busy_writer.is_none()).await;
        assert!(f.service.lane_writers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_lane_setup_leaves_no_worktree_or_lane_record() {
        let code = tempfile::tempdir().unwrap();
        let repo = code.path().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let f = fixture(FakeHarness::default());
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let mut settings = project.settings.clone();
        settings.setup = Some("exit 9".into());
        f.service
            .update_project(project.id, settings)
            .await
            .unwrap();

        let error = f
            .service
            .create_lane(project.id, "Broken setup".into(), false)
            .await
            .unwrap_err();
        assert!(error.message.contains("project setup failed"), "{error}");
        assert!(f.service.list().await.lanes.is_empty());
        assert!(!f._dir.path().join("lanes/broken-setup").exists());
    }

    #[tokio::test]
    async fn cancelling_lane_setup_rolls_back_the_partial_worktree() {
        let code = tempfile::tempdir().unwrap();
        let repo = code.path().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let f = fixture(FakeHarness::default());
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let mut settings = project.settings.clone();
        settings.setup = Some("touch started; exec sleep 30".into());
        f.service
            .update_project(project.id, settings)
            .await
            .unwrap();
        let lane_path = f._dir.path().join("lanes/slow-setup/web");
        let started = lane_path.join("started");

        let creating = tokio::spawn({
            let service = f.service.clone();
            async move {
                service
                    .create_lane(project.id, "Slow setup".into(), false)
                    .await
            }
        });
        for _ in 0..200 {
            if started.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(started.exists(), "setup did not start");
        f.service.cancel_lane_creation(project.id);
        let error = tokio::time::timeout(Duration::from_secs(5), creating)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert!(f.service.list().await.lanes.is_empty());
        assert!(!lane_path.exists());
    }

    #[tokio::test]
    async fn main_checkout_lanes_are_marked_and_never_remove_the_checkout() {
        let code = tempfile::tempdir().unwrap();
        let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
        git(
            &repo,
            &["commit", "--quiet", "--allow-empty", "-m", "start"],
        );
        let f = fixture(FakeHarness::default());
        let project = f
            .service
            .add_project(repo.to_string_lossy().into_owned())
            .await
            .unwrap();
        let lane = f
            .service
            .create_lane(project.id, "Quick question".into(), true)
            .await
            .unwrap();
        assert!(lane.worktrees[0].main_checkout);
        assert_eq!(lane.worktrees[0].path, project.repo);
        let error = f
            .service
            .create_lane(project.id, "Another main".into(), true)
            .await
            .unwrap_err();
        assert!(
            error
                .message
                .contains("already has an active main-checkout")
        );

        std::fs::write(repo.join("uncommitted.txt"), "keep me").unwrap();
        let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
        assert!(matches!(outcome, ArchiveLaneOutcome::Archived));
        assert!(repo.join("uncommitted.txt").exists());
        assert_eq!(f.service.list().await.lanes[0].state, LaneState::Archived);

        let again = f
            .service
            .create_lane(project.id, "Quick question".into(), true)
            .await
            .unwrap();
        assert_eq!(again.name, "quick-question-2");
        let error = f.service.restore_lane(lane.id).await.unwrap_err();
        assert!(
            error
                .message
                .contains("already has an active main-checkout"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_project_whose_repository_moved_can_still_be_cleared_away() {
        let f = fixture(FakeHarness::default());
        let (code, lane) = lane_of(&f).await;
        let repo = PathBuf::from(&lane.worktrees[0].path);
        std::fs::rename(&repo, code.path().join("moved")).unwrap();

        f.service.list().await;
        f.service.delete_lane(lane.id).await.unwrap();
        f.service.remove_project(lane.project_id).await.unwrap();
        assert!(f.service.list().await.projects.is_empty());
    }

    #[tokio::test]
    async fn deleting_a_lane_requires_a_closed_session_and_frees_the_project() {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        let repo = PathBuf::from(&lane.worktrees[0].path);
        std::fs::write(repo.join("keep.txt"), "main checkout work").unwrap();
        let opened = f.service.create(lane.id, "fake").await.unwrap();

        let error = f.service.delete_lane(lane.id).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Rejected);
        assert!(
            error.message.contains("Close this lane's session"),
            "{error}"
        );
        assert_eq!(f.service.list().await.lanes.len(), 1);

        f.service.close(&opened.key).await;
        f.service.delete_lane(lane.id).await.unwrap();
        let listing = f.service.list().await;
        assert!(listing.lanes.is_empty());
        assert!(
            listing
                .sessions
                .iter()
                .all(|session| session.key != opened.key && session.lane_id.is_none())
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("keep.txt")).unwrap(),
            "main checkout work"
        );

        f.service.remove_project(lane.project_id).await.unwrap();
        assert!(f.service.list().await.projects.is_empty());
    }

    #[tokio::test]
    async fn unknown_harnesses_and_sessions_are_errors() {
        let f = fixture(FakeHarness::default());
        fn kind<T: std::fmt::Debug>(result: ServiceResult<T>) -> ErrorKind {
            result.unwrap_err().kind
        }
        assert_eq!(
            kind(f.service.create(1, "nope").await),
            ErrorKind::Unavailable
        );
        assert_eq!(
            kind(f.service.abort("fake:missing").await),
            ErrorKind::NotOpen
        );
        assert_eq!(
            kind(
                f.service
                    .answer("fake:missing", "q".into(), Answer::Cancelled)
                    .await
            ),
            ErrorKind::NotOpen
        );
    }

    #[tokio::test]
    async fn a_harness_without_models_reports_no_settings() {
        let f = fixture(FakeHarness::default());
        f.service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
            .unwrap();
        let settings = f.service.settings("fake:s1").await.unwrap();
        assert_eq!(settings, SessionSettings::default());
        assert!(f.service.models("fake:s1").await.unwrap().is_empty());
        let error = f
            .service
            .set_thinking_level("fake:s1", "high".into())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ServiceError::new(ErrorKind::Rejected, "This harness has no thinking levels")
        );
    }
}
