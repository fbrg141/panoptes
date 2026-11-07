//! The app's session registry: which harnesses are loaded, which sessions
//! are open, and Panoptes' own store. Independent of Tauri so it can be
//! tested with a fake harness; `lib.rs` exposes it as commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::Context;
use panoptes_harness::{
    AgentSession, Answer, Harness, HarnessError, HarnessErrorKind, HarnessEvent, ImageAttachment,
    Message, ModelInfo, PromptMode, PromptOutcome, Role, SessionRef, SessionSettings,
    SessionStatus, SessionSummary, StartSession,
};
use serde::Serialize;
use tokio::sync::{Mutex, oneshot};

use crate::lanes::{self, ArchiveResult, ArchiveStrategy};
use crate::projects::{self, Location};
use crate::store::{Lane, LaneState, Project, ProjectSettings, Store};

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

pub fn session_key(harness: &str, id: &str) -> String {
    format!("{harness}:{id}")
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarSession {
    pub key: String,
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub project_root: String,
    /// The main checkout of the git repository the session runs in, shared
    /// by all its worktrees. `None` outside git.
    pub checkout: Option<String>,
    /// The registered project whose repository the session runs in, in its
    /// main checkout or one of its worktrees.
    pub project_id: Option<i64>,
    pub pinned: bool,
    pub open: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub sessions: Vec<SidebarSession>,
    /// Every registered project, by name, with or without sessions.
    pub projects: Vec<Project>,
    /// Every lane, including archived ones; navigation decides which to show.
    pub lanes: Vec<Lane>,
    /// One line per harness whose sessions could not be listed, and one
    /// each if the pins, projects or lanes could not be read.
    pub errors: Vec<String>,
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
    pub key: String,
    pub session: SessionRef,
    pub history: Vec<Message>,
}

struct OpenSession {
    /// Distinguishes this process from earlier ones for the same session, so
    /// a late exit from a replaced process cannot close its successor.
    generation: u64,
    session: Arc<dyn AgentSession>,
    status: Arc<std::sync::Mutex<StatusTracker>>,
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
        let mut summaries = Vec::new();
        let mut errors = Vec::new();
        for harness in self.harnesses.values() {
            match harness.list_sessions().await {
                Ok(found) => summaries.extend(found),
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
        let by_checkout: HashMap<&Path, i64> = projects
            .iter()
            .map(|project| (Path::new(&project.repo), project.id))
            .collect();
        let open = self.open.lock().await;
        let mut locations = self.locations.lock().unwrap();
        let sessions = summaries
            .into_iter()
            .map(|summary| {
                let key = session_key(&summary.harness, &summary.id);
                let location = cached_location(&mut locations, &summary.cwd);
                SidebarSession {
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
                    summary,
                }
            })
            .collect();
        drop(locations);
        Listing {
            sessions,
            projects,
            lanes,
            errors,
        }
    }

    /// Opens a listed session, or returns it if it is already open.
    pub async fn open(
        &self,
        harness: &str,
        key: &str,
        locator: String,
        cwd: String,
    ) -> ServiceResult<OpenedSession> {
        let lock = self
            .opening
            .lock()
            .await
            .entry(key.to_string())
            .or_default()
            .clone();
        let _guard = lock.lock().await;

        let existing = self.open.lock().await.get(key).map(|o| o.session.clone());
        let result = match existing {
            Some(session) => session
                .history()
                .await
                .map(|history| OpenedSession {
                    key: key.to_string(),
                    session: session.session_ref(),
                    history,
                })
                .map_err(err),
            None => {
                let request = StartSession::Resume {
                    locator,
                    cwd: PathBuf::from(cwd),
                };
                self.start(harness, request).await
            }
        };
        drop(_guard);
        self.opening.lock().await.remove(key);
        result
    }

    pub async fn create(&self, harness: &str, cwd: String) -> ServiceResult<OpenedSession> {
        let request = StartSession::New {
            cwd: PathBuf::from(cwd),
        };
        self.start(harness, request).await
    }

    /// Starts a session's process and forwards its events through `emit`,
    /// tagged with the session key.
    async fn start(&self, harness_id: &str, request: StartSession) -> ServiceResult<OpenedSession> {
        let harness = self.harness(harness_id)?;
        let generation = self.generations.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<HarnessEvent>();
        let (key_tx, mut key_rx) = oneshot::channel::<String>();
        let status = Arc::new(std::sync::Mutex::new(StatusTracker::default()));
        {
            let emit = self.emit.clone();
            let open = self.open.clone();
            let status = status.clone();
            tokio::spawn(async move {
                // The key is known only once the harness reports the session
                // id; anything said before that is held back until then.
                let mut early = std::collections::VecDeque::new();
                let key = loop {
                    tokio::select! {
                        // Once the key is known, stop buffering.
                        biased;
                        key = &mut key_rx => match key {
                            Ok(key) => break key,
                            Err(_) => return, // the start failed
                        },
                        Some(event) = rx.recv() => early.push_back(event),
                    }
                };
                emit(SessionEvent {
                    key: key.clone(),
                    event: SessionUpdate::Status(StatusChanged {
                        status: SessionStatus::Idle,
                    }),
                });
                loop {
                    let event = match early.pop_front() {
                        Some(event) => event,
                        None => match rx.recv().await {
                            Some(event) => event,
                            None => return,
                        },
                    };
                    if matches!(event, HarnessEvent::Exited { .. }) {
                        let mut open = open.lock().await;
                        let current = open.get(&key).is_some_and(|o| o.generation == generation);
                        if !current {
                            // Closed on purpose or replaced: not news.
                            continue;
                        }
                        open.remove(&key);
                    }
                    // Holding the tracker while emitting keeps statuses in
                    // order with the ones `answer` sends.
                    let mut tracker = status.lock().unwrap();
                    let changed = tracker.apply(&event, now_ms());
                    emit(SessionEvent {
                        key: key.clone(),
                        event: SessionUpdate::Harness(event),
                    });
                    if let Some(status) = changed {
                        emit(SessionEvent {
                            key: key.clone(),
                            event: SessionUpdate::Status(StatusChanged { status }),
                        });
                    }
                }
            });
        }

        let session: Arc<dyn AgentSession> =
            Arc::from(harness.start_session(request, tx).await.map_err(err)?);
        let session_ref = session.session_ref();
        let key = session_key(&session_ref.harness, &session_ref.id);
        let history = match session.history().await {
            Ok(history) => history,
            Err(error) => {
                session.shutdown().await;
                return Err(err(error));
            }
        };
        self.open.lock().await.insert(
            key.clone(),
            OpenSession {
                generation,
                session,
                status,
            },
        );
        let _ = key_tx.send(key.clone());
        Ok(OpenedSession {
            key,
            session: session_ref,
            history,
        })
    }

    pub async fn history(&self, key: &str) -> ServiceResult<Vec<Message>> {
        self.session(key).await?.history().await.map_err(err)
    }

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

    pub async fn set_pinned(&self, key: String, pinned: bool) -> ServiceResult<()> {
        self.with_store(move |store| store.set_pinned(&key, pinned))
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

    /// Creates a durable lane before any session exists. The store mutex
    /// serialises name allocation and the one-active-main-checkout rule.
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
        let result = self
            .with_store(move |store| {
                let task = task.trim().to_string();
                if task.is_empty() {
                    anyhow::bail!("A lane needs a task");
                }
                let project = store.project(project_id)?;
                let existing = store.lanes()?;
                if main_checkout
                    && existing.iter().any(|lane| {
                        lane.project_id == project_id
                            && lane.state == LaneState::Active
                            && lane.worktrees.iter().any(|tree| tree.main_checkout)
                    })
                {
                    anyhow::bail!("This project already has an active main-checkout lane");
                }
                let base = lanes::slug(&task);
                let mut name = base.clone();
                let mut suffix = 2;
                while existing
                    .iter()
                    .any(|lane| lane.project_id == project_id && lane.name == name)
                {
                    name = format!("{base}-{suffix}");
                    suffix += 1;
                }
                let new = lanes::create(&root, &project, &name, &task, main_checkout, &in_setup)?;
                match store.add_lane(&new) {
                    Ok(lane) => Ok(lane),
                    Err(error) => {
                        lanes::discard_created(&project, &new);
                        Err(error)
                    }
                }
            })
            .await;
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
        self.with_store(move |store| {
            let lane = store.lane(id)?;
            if lane.state != LaneState::Active {
                anyhow::bail!("That lane is already archived");
            }
            let project = store.project(lane.project_id)?;
            match lanes::archive(&project, &lane, strategy)? {
                ArchiveResult::Dirty => Ok(ArchiveLaneOutcome::Dirty { lane }),
                ArchiveResult::Archived { checkpoint_ref } => {
                    store.archive_lane(id, checkpoint_ref.as_deref())?;
                    Ok(ArchiveLaneOutcome::Archived)
                }
            }
        })
        .await
        .map_err(err)
    }

    pub async fn restore_lane(&self, id: i64) -> ServiceResult<Lane> {
        self.with_store(move |store| {
            let lane = store.lane(id)?;
            if lane.state != LaneState::Archived {
                anyhow::bail!("That lane is not archived");
            }
            let project = store.project(lane.project_id)?;
            lanes::restore(&project, &lane)?;
            store.restore_lane(id)
        })
        .await
        .map_err(err)
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
    }

    struct FakeSession {
        id: String,
        events: EventSink,
    }

    #[async_trait]
    impl Harness for FakeHarness {
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
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
                .map(|(i, cwd)| SessionSummary {
                    harness: "fake".into(),
                    id: format!("s{}", i + 1),
                    locator: format!("s{}", i + 1),
                    cwd,
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
            self.starts.fetch_add(1, Ordering::SeqCst);
            // Slow enough that concurrent opens overlap.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let id = match request {
                StartSession::Resume { locator, .. } => locator,
                StartSession::New { .. } => "new".into(),
            };
            let _ = events.send(HarnessEvent::Raw {
                payload: serde_json::json!("early"),
            });
            self.sinks.lock().unwrap().push(events.clone());
            Ok(Box::new(FakeSession { id, events }))
        }
    }

    #[async_trait]
    impl AgentSession for FakeSession {
        fn session_ref(&self) -> SessionRef {
            SessionRef {
                harness: "fake".into(),
                id: self.id.clone(),
                locator: self.id.clone(),
                cwd: "/tmp".into(),
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

    #[tokio::test]
    async fn events_from_before_the_id_is_known_are_delivered() {
        let f = fixture(FakeHarness::default());
        f.service.create("fake", "/tmp".into()).await.unwrap();
        settle().await;
        let events = f.events.lock().unwrap();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(events.iter().all(|e| e.key == "fake:new"));
        assert!(matches!(
            events[1].event,
            SessionUpdate::Harness(HarnessEvent::Raw { .. })
        ));
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
    }

    #[tokio::test]
    async fn unknown_harnesses_and_sessions_are_errors() {
        let f = fixture(FakeHarness::default());
        fn kind<T: std::fmt::Debug>(result: ServiceResult<T>) -> ErrorKind {
            result.unwrap_err().kind
        }
        assert_eq!(
            kind(f.service.create("nope", "/tmp".into()).await),
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
