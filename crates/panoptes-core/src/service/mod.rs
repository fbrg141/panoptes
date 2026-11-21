//! The app's session registry: which harnesses are loaded, which sessions
//! are open, and Panoptes' own store. Independent of the UI so it can be
//! tested with a fake harness.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use panoptes_harness::{
    AgentSession, Answer, CommandInfo, DiscoveredSession, Harness, HarnessEvent, HarnessRef,
    ImageAttachment, Message, ModelInfo, PromptMode, PromptOutcome, SessionSettings, SessionStatus,
    StartSession,
};
use serde::Serialize;

use crate::changes::{self, Changes, DiffOptions, Hunk, TreeRange};
use crate::cleanup::{self, IdleLane, LaneCleanup};
use crate::eventlog::EventLog;
use crate::lanes::{self, ArchiveResult, ArchiveStrategy, DeleteResult, DetachedHead, LaneWork};
use crate::projects::{self, Location};
use crate::store::{Lane, LaneState, Preferences, ProfileInput, Project, ProjectSettings, Store};
use serde_json::json;

mod error;
mod events;
mod forward;
mod key;
mod registry;
mod status;
mod store;
mod writers;

pub use error::{ErrorKind, ServiceError, ServiceResult};
pub use events::{Emit, LaneWritesChanged, SessionEvent, SessionUpdate, StatusChanged};
pub use key::{harness_key, panoptes_key};

use error::{err, not_open, only_in_a_lane};
use forward::{Forwarder, WriteWatch};
use key::{SessionKey, panoptes_id};
use registry::{EarlyExit, Launch, OpenSession, Registry};
use status::{Tracker, now_ms};
use store::SharedStore;
use writers::LaneWriters;

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
    Dirty {
        lane: Lane,
    },
    DetachedHead {
        lane: Lane,
        detached_head: DetachedHead,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DeleteLaneOutcome {
    Deleted,
    DetachedHead {
        lane: Lane,
        detached_head: DetachedHead,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSession {
    /// Opening an adopted conversation by its harness key answers with its
    /// Panoptes key.
    pub key: String,
    pub history: Vec<Message>,
    /// Writable tools that run without asking for approval.
    pub unguarded_tools: bool,
    /// A read-only look at an earlier lane session. Sending resumes it.
    pub preview: bool,
    /// The session's status now, so a reloaded window does not show a
    /// Running or Waiting session as idle until its next change.
    pub status: SessionStatus,
    /// Events to apply over the history to catch up: the run's start while
    /// it runs, then the questions and approvals still waiting.
    pub replay: Vec<HarnessEvent>,
}

pub struct Service {
    registry: Registry,
    harnesses: HashMap<String, Arc<dyn Harness>>,
    /// One lifecycle lock per session key, held by every step that starts,
    /// stops or re-homes its process: open, create, close and adopt. So a
    /// session never gets a second process while its first one is starting
    /// or shutting down. Entries are never removed, so two callers can never
    /// hold different locks for one key.
    /// A second process for a lane never starts while another start or stop
    /// is in flight. Session records persist; only their processes are exclusive.
    /// Sessions whose process is starting, with their lane. They are not in
    /// `open` yet, and a lane must not be archived or deleted under them.
    /// Shared with the blocking threads that run every store call.
    store: SharedStore,
    /// Central folder for lane worktrees, normally `~/Panoptes/lanes`.
    lanes_root: PathBuf,
    /// Cancellation flags for setup commands, keyed by their project.
    lane_cancellations: Arc<std::sync::Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    /// Lanes with an archive, restore or delete in progress.
    busy_lanes: Arc<std::sync::Mutex<HashSet<i64>>>,
    /// Open sessions seen editing a lane that is not their own.
    /// Panoptes' own record of what happened; disabled unless given one.
    log: EventLog,
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
            registry: Registry::new(),
            harnesses: harnesses
                .into_iter()
                .map(|harness| (harness.id().to_string(), harness))
                .collect(),
            store: SharedStore::new(store),
            lanes_root,
            lane_cancellations: Arc::default(),
            busy_lanes: Arc::default(),
            log: EventLog::default(),
            locations: std::sync::Mutex::default(),
            emit,
        }
    }

    /// Records what happens from now on into `log`.
    pub fn with_event_log(mut self, log: EventLog) -> Self {
        self.log = log;
        self
    }

    /// Records an error a command returned to the UI.
    pub fn record_error(&self, command: &str, error: &ServiceError) {
        self.log.record(
            "error",
            json!({"command": command, "kind": error.kind, "message": error.message}),
        );
    }

    fn harness(&self, id: &str) -> ServiceResult<Arc<dyn Harness>> {
        self.harnesses.get(id).cloned().ok_or_else(|| {
            ServiceError::new(
                ErrorKind::Unavailable,
                format!("The {id} harness is not available"),
            )
        })
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
            .store
            .call(|store| store.pinned())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read pins: {}", err(error)));
                Default::default()
            });
        let projects = self
            .store
            .call(|store| store.projects())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read projects: {}", err(error)));
                Vec::new()
            });
        let lanes = self
            .store
            .call(|store| store.lanes())
            .await
            .unwrap_or_else(|error| {
                errors.push(format!("Could not read lanes: {}", err(error)));
                Vec::new()
            });
        let records = self
            .store
            .call(|store| store.sessions())
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
        let open = self.registry.open().await;
        let busy_writers: HashMap<i64, String> = lanes
            .iter()
            .filter_map(|lane| {
                Some((
                    lane.id,
                    busy_writer(&open, &self.registry.lane_writers(), lane.id)?,
                ))
            })
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
                // A preview is open for reading, but is not the lane's live
                // session for navigation, status or session changes.
                open: open.get(&key).is_some_and(|session| !session.preview),
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
        let mut guard = self.registry.lifecycle(key).await;
        let key = match SessionKey::parse(harness, key) {
            SessionKey::Panoptes(id) => panoptes_key(id),
            SessionKey::Harness { id: harness_id } => {
                // Looked up under the conversation's own lock, which `adopt`
                // holds while it records an adoption: an open never starts
                // a raw process on a conversation a lane has just taken.
                let harness_id = harness_id.to_string();
                let name = harness.to_string();
                let adopted = self
                    .store
                    .call(move |store| store.session_for(&name, &harness_id))
                    .await
                    .map_err(err)?;
                match adopted {
                    Some(id) => {
                        drop(guard);
                        let key = panoptes_key(id);
                        guard = self.registry.lifecycle(&key).await;
                        key
                    }
                    None => key.to_string(),
                }
            }
        };

        let existing = self
            .registry
            .open()
            .await
            .get(&key)
            .map(|o| (o.session.clone(), o.status.clone(), o.preview));
        let result = match existing {
            Some((session, status, preview)) => match session.history().await {
                Ok(history) => {
                    // Taken after the history, so nothing asked meanwhile
                    // is missed; the UI drops what it already has by id.
                    let (status, replay) = status.lock().unwrap().snapshot(now_ms());
                    Ok(OpenedSession {
                        key: key.clone(),
                        history,
                        unguarded_tools: session.unguarded_tools(),
                        preview,
                        status,
                        replay,
                    })
                }
                Err(error) => Err(err(error)),
            },
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
                        preview: false,
                        instructions: None,
                    };
                    self.start(&key, launch, None).await
                }
            },
        };
        drop(guard);
        result
    }

    /// Opens an earlier lane session's history without stopping its live
    /// session. The preview has no writing tools; sending a message resumes it.
    pub async fn preview(&self, key: &str) -> ServiceResult<OpenedSession> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("previewed"))?;
        let guard = self.registry.lifecycle(key).await;
        let existing = self
            .registry
            .open()
            .await
            .get(key)
            .map(|o| (o.session.clone(), o.status.clone(), o.preview));
        if let Some((session, status, preview)) = existing {
            let history = session.history().await.map_err(err)?;
            let (status, replay) = status.lock().unwrap().snapshot(now_ms());
            return Ok(OpenedSession {
                key: key.to_string(),
                history,
                unguarded_tools: session.unguarded_tools(),
                preview,
                status,
                replay,
            });
        }
        let mut launch = self.recorded_launch(id).await?;
        launch.read_only = true;
        launch.preview = true;
        let result = self.start(key, launch, None).await;
        drop(guard);
        result
    }

    /// How a recorded session starts: in its lane's worktree, resuming its
    /// conversation if the harness ever reported one.
    async fn recorded_launch(&self, id: i64) -> ServiceResult<Launch> {
        let (record, lane) = self
            .store
            .call(move |store| {
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
            preview: false,
            instructions: record.instructions,
        })
    }

    /// Stops the lane's current process, if any, so another conversation can
    /// take the lane. It refuses while that agent runs, asks, or holds queued
    /// prompts. The idle check and removal from `open` happen under one lock,
    /// so a prompt arriving meanwhile finds no session instead of being lost
    /// to the shutdown. Returns the key that was stopped.
    async fn stop_for_lane_change(&self, lane_id: i64) -> ServiceResult<Option<String>> {
        if self.registry.is_starting_in_lane(lane_id) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Wait for the current session to finish starting before changing sessions",
            ));
        }
        let current = self
            .registry
            .open()
            .await
            .iter()
            .find(|(_, session)| session.lane == Some(lane_id) && !session.preview)
            .map(|(key, _)| key.clone());
        let _guard = match &current {
            Some(key) => Some(self.registry.lifecycle(key).await),
            None => None,
        };
        let removed = {
            let mut open = self.registry.open().await;
            if busy_writer(&open, &self.registry.lane_writers(), lane_id).is_some() {
                return Err(ServiceError::new(
                    ErrorKind::Rejected,
                    "Another session is still changing this lane. Change sessions once it is Done",
                ));
            }
            let now = open
                .iter()
                .find(|(_, session)| session.lane == Some(lane_id) && !session.preview)
                .map(|(key, _)| key.clone());
            if now != current {
                return Err(ServiceError::new(
                    ErrorKind::Rejected,
                    "This lane's session changed meanwhile. Try again",
                ));
            }
            match now {
                Some(key) => {
                    {
                        let status = open[&key].status.lock().unwrap();
                        if !status.is_idle() {
                            return Err(ServiceError::new(
                                ErrorKind::Rejected,
                                "Stop the agent, answer its questions and clear queued messages before changing sessions",
                            ));
                        }
                    }
                    open.remove(&key).map(|session| (key, session))
                }
                None => None,
            }
        };
        let stopped = match removed {
            Some((key, session)) => {
                self.log.record("sessionClosed", json!({"key": key}));
                session.session.shutdown().await;
                Some(key)
            }
            None => None,
        };
        self.close_lane_previews(lane_id).await;
        Ok(stopped)
    }

    /// Closes a lane's read-only previews. They hold no work, so nothing that
    /// needs the lane to itself waits for the user to close them.
    async fn close_lane_previews(&self, lane_id: i64) {
        let previews: Vec<String> = self
            .registry
            .open()
            .await
            .iter()
            .filter(|(_, session)| session.lane == Some(lane_id) && session.preview)
            .map(|(key, _)| key.clone())
            .collect();
        for key in previews {
            self.close(&key).await;
        }
    }

    /// Stops the lane's current process and starts a fresh conversation record
    /// in the same worktree, retaining its harness and saved instructions.
    pub async fn replace_lane_session(&self, lane_id: i64) -> ServiceResult<OpenedSession> {
        let current_id = self
            .registry
            .open()
            .await
            .iter()
            .find(|(_, session)| session.lane == Some(lane_id) && !session.preview)
            .and_then(|(key, _)| panoptes_id(key));
        let (harness, instructions) = self
            .store
            .call(move |store| {
                let latest = match current_id {
                    Some(id) => Some(store.session(id)?),
                    None => store
                        .sessions()?
                        .into_iter()
                        .filter(|session| session.lane_id == lane_id)
                        .max_by_key(|session| (session.created_at, session.id)),
                };
                Ok(latest.map_or(("pi".to_string(), None), |session| {
                    (session.harness, session.instructions)
                }))
            })
            .await
            .map_err(err)?;
        self.stop_for_lane_change(lane_id).await?;
        self.create_with(lane_id, &harness, instructions).await
    }

    /// Stops the lane's current process and resumes exactly the selected
    /// Panoptes record, never a conversation found by scanning its cwd.
    pub async fn resume_lane_session(
        &self,
        lane_id: i64,
        session_id: i64,
    ) -> ServiceResult<OpenedSession> {
        let harness = self
            .store
            .call(move |store| {
                let record = store.session(session_id)?;
                if record.lane_id != lane_id {
                    anyhow::bail!("That session does not belong to this lane");
                }
                if record.archived {
                    anyhow::bail!("That session is archived; restore it before resuming");
                }
                Ok(record.harness)
            })
            .await
            .map_err(err)?;
        let key = panoptes_key(session_id);
        let already_open = self
            .registry
            .open()
            .await
            .get(&key)
            .is_some_and(|session| !session.preview);
        if !already_open {
            self.stop_for_lane_change(lane_id).await?;
        }
        self.open(&harness, &key, String::new(), String::new())
            .await
    }

    /// Starts a new Panoptes session in a lane's worktree. Its id, and so
    /// its key, exist before the harness starts.
    /// A lane that another Running or Waiting session has edited gets no
    /// session of its own until that one is Done: two writers otherwise.
    #[cfg(test)]
    pub async fn create(&self, lane_id: i64, harness: &str) -> ServiceResult<OpenedSession> {
        self.create_with(lane_id, harness, None).await
    }

    /// Creates a session with instructions to append to its system prompt.
    pub async fn create_with(
        &self,
        lane_id: i64,
        harness: &str,
        instructions: Option<String>,
    ) -> ServiceResult<OpenedSession> {
        self.harness(harness)?;
        let busy = {
            let open = self.registry.open().await;
            busy_writer(&open, &self.registry.lane_writers(), lane_id)
        };
        if busy.is_some() {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Another session is still changing this lane. Start one here once it is Done",
            ));
        }
        let name = harness.to_string();
        let stored = instructions.clone();
        let (record, lane) = self
            .store
            .call(move |store| {
                let record = store.add_session_configured(lane_id, &name, stored.as_deref())?;
                Ok((record, store.lane(lane_id)?))
            })
            .await
            .map_err(err)?;
        let _guard = self.registry.lifecycle(&panoptes_key(record.id)).await;
        self.start_recorded(record.id, &lane, harness, instructions)
            .await
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
        if matches!(SessionKey::parse(harness, key), SessionKey::Panoptes(_)) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "This session already belongs to a lane",
            ));
        }
        let SessionKey::Harness { id: harness_id } = SessionKey::parse(harness, key) else {
            unreachable!("Panoptes keys return above")
        };
        if harness_id.is_empty() {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Invalid harness session key",
            ));
        }
        let harness_id = harness_id.to_string();
        // Held until the adoption is recorded, so no open of this
        // conversation can start a raw process in between.
        let _guard = self.registry.lifecycle(key).await;
        let name = harness.to_string();
        let id_for_lookup = harness_id.clone();
        if let Some(id) = self
            .store
            .call(move |store| store.session_for(&name, &id_for_lookup))
            .await
            .map_err(err)?
        {
            return Ok(AdoptionOutcome::Adopted {
                key: panoptes_key(id),
            });
        }
        if self.registry.open().await.contains_key(key) {
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
                    .store
                    .call(|store| store.sessions())
                    .await
                    .map_err(err)?
                    .into_iter()
                    .map(|record| record.lane_id)
                    .collect::<HashSet<_>>();
                let candidates = self
                    .store
                    .call(|store| store.lanes())
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
            .store
            .call(move |store| store.adopt_session(lane_id, &harness, &harness_id, &locator))
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
        instructions: Option<String>,
    ) -> ServiceResult<OpenedSession> {
        let launch = Launch {
            harness: harness.to_string(),
            cwd: PathBuf::from(lane_cwd(lane)),
            resume: None,
            lane: Some(lane.id),
            read_only: false,
            preview: false,
            instructions,
        };
        let result = self.start(&panoptes_key(id), launch, None).await;
        if result.is_err() {
            let _ = self
                .store
                .call(move |store| store.discard_session(id))
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
        // Listed as starting before the busy check, while archive and
        // delete mark the lane busy before checking for sessions: whichever
        // comes second sees the other.
        let _lane_guard = match launch.lane {
            Some(lane) => Some(self.registry.lane_lifecycle(lane).await),
            None => None,
        };
        if launch.preview {
            // Keep one preview per lane, alongside its one live session. The
            // lane start lock keeps two quick preview clicks from overlapping.
            let previous: Vec<String> = self
                .registry
                .open()
                .await
                .iter()
                .filter(|(other, session)| {
                    other.as_str() != key && session.lane == launch.lane && session.preview
                })
                .map(|(key, _)| key.clone())
                .collect();
            for key in previous {
                self.close(&key).await;
            }
        }
        if let Some(lane) = launch.lane
            && !launch.preview
            && self
                .registry
                .open()
                .await
                .iter()
                // A preview of another session sits beside the live one; a
                // preview of this one would be a second process for it.
                .any(|(other, session)| {
                    session.lane == Some(lane) && (!session.preview || other.as_str() == key)
                })
        {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Another session is already open in this lane",
            ));
        }
        let _starting = self.registry.starting(key, launch.lane);
        if let Some(lane) = launch.lane
            && self.busy_lanes.lock().unwrap().contains(&lane)
        {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "This lane is being archived, restored or deleted",
            ));
        }
        let generation = self.registry.next_generation();
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
                open: self.registry.open_handle(),
                status: status.clone(),
                emit: self.emit.clone(),
                writes: WriteWatch {
                    writers: self.registry.lane_writers(),
                    store: self.store.clone(),
                    own: launch.lane,
                    cwd: launch.cwd.clone(),
                },
                early_exit: early_exit.clone(),
                log: self.log.clone(),
            }
            .run(rx),
        );

        let resumed = launch.resume.is_some();
        let request = StartSession {
            cwd: launch.cwd,
            resume: launch.resume,
            read_only: launch.read_only,
            instructions: launch.instructions,
        };
        let started = async {
            let session: Arc<dyn AgentSession> =
                Arc::from(harness.start_session(request, tx).await.map_err(err)?);
            let ready = async {
                let history = session.history().await.map_err(err)?;
                if let Some(id) = panoptes_id(key) {
                    let reference = session.harness_ref();
                    self.store
                        .call(move |store| {
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
        let unguarded_tools = session.unguarded_tools();
        let (now, replay) = {
            let mut open = self.registry.open().await;
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
            self.log.record(
                "sessionStarted",
                json!({
                    "key": key,
                    "harness": launch.harness,
                    "lane": launch.lane,
                    "resumed": resumed,
                    "readOnly": launch.read_only,
                }),
            );
            let (now, replay) = status.lock().unwrap().snapshot(now_ms());
            open.insert(
                key.to_string(),
                OpenSession {
                    generation,
                    session,
                    status,
                    lane: launch.lane,
                    preview: launch.preview,
                },
            );
            (now, replay)
        };
        Ok(OpenedSession {
            key: key.to_string(),
            history,
            unguarded_tools,
            preview: launch.preview,
            status: now,
            replay,
        })
    }

    pub async fn history(&self, key: &str) -> ServiceResult<Vec<Message>> {
        self.registry
            .session(key)
            .await?
            .history()
            .await
            .map_err(err)
    }

    /// Sends a prompt to the addressed session.
    pub async fn prompt(
        &self,
        key: &str,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> ServiceResult<PromptOutcome> {
        self.registry
            .session(key)
            .await?
            .prompt(text, images, mode)
            .await
            .map_err(err)
    }

    pub async fn settings(&self, key: &str) -> ServiceResult<SessionSettings> {
        self.registry
            .session(key)
            .await?
            .settings()
            .await
            .map_err(err)
    }

    pub async fn models(&self, key: &str) -> ServiceResult<Vec<ModelInfo>> {
        self.registry
            .session(key)
            .await?
            .models()
            .await
            .map_err(err)
    }

    pub async fn commands(&self, key: &str) -> ServiceResult<Vec<CommandInfo>> {
        self.registry
            .session(key)
            .await?
            .commands()
            .await
            .map_err(err)
    }

    pub async fn preferences(&self) -> ServiceResult<Preferences> {
        self.store
            .call(|store| store.preferences())
            .await
            .map_err(err)
    }

    pub async fn dismiss_permission_warning(&self) -> ServiceResult<Preferences> {
        self.store
            .call(|store| store.dismiss_permission_warning())
            .await
            .map_err(err)?;
        self.preferences().await
    }

    pub async fn save_profile(
        &self,
        id: Option<i64>,
        input: ProfileInput,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.save_profile(id, &input))
            .await?;
        self.preferences().await
    }

    pub async fn delete_profile(&self, id: i64) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.delete_profile(id))
            .await?;
        self.preferences().await
    }

    pub async fn set_model_favorite(
        &self,
        model_id: String,
        favorite: bool,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_model_favorite(&model_id, favorite))
            .await?;
        self.preferences().await
    }

    pub async fn set_model_hidden(
        &self,
        model_id: String,
        hidden: bool,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_model_hidden(&model_id, hidden))
            .await?;
        self.preferences().await
    }

    pub async fn set_cleanup_idle_days(&self, days: u32) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_cleanup_idle_days(days))
            .await?;
        self.preferences().await
    }

    /// The lane's complete diff against its merge-base, without patch text.
    pub async fn lane_changes(&self, lane_id: i64) -> ServiceResult<Changes> {
        let (project, lane) = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                let project = store.project(lane.project_id)?;
                Ok((project, lane))
            })
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || {
            let range = changes::lane_range(&project, &lane)?;
            changes::diff(Path::new(&project.repo), &range, DiffOptions::default())
        })
        .await
        .context("The lane diff stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Patch hunks for one file in the range the lane list returned.
    pub async fn file_hunks(
        &self,
        lane_id: i64,
        path: String,
        range: TreeRange,
    ) -> ServiceResult<Vec<Hunk>> {
        let repo = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                Ok(store.project(lane.project_id)?.repo)
            })
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || {
            changes::hunks(Path::new(&repo), &range, &path, DiffOptions::default())
        })
        .await
        .context("The file diff stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Switches the session's model and returns its settings after the
    /// switch, since the thinking levels on offer depend on the model.
    pub async fn set_model(&self, key: &str, model_id: String) -> ServiceResult<SessionSettings> {
        let session = self.registry.session(key).await?;
        session.set_model(model_id).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn set_thinking_level(
        &self,
        key: &str,
        level: String,
    ) -> ServiceResult<SessionSettings> {
        let session = self.registry.session(key).await?;
        session.set_thinking_level(level).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn abort(&self, key: &str) -> ServiceResult<()> {
        self.registry.session(key).await?.abort().await.map_err(err)
    }

    pub async fn answer(
        &self,
        key: &str,
        question_id: String,
        answer: Answer,
    ) -> ServiceResult<()> {
        let (session, status) = self
            .registry
            .open()
            .await
            .get(key)
            .map(|open| (open.session.clone(), open.status.clone()))
            .ok_or_else(not_open)?;
        session
            .answer(question_id.clone(), answer)
            .await
            .map_err(err)?;
        self.log
            .record("answered", json!({"key": key, "id": question_id}));
        let mut tracker = status.lock().unwrap();
        if let Some(status) = tracker.answered(&question_id, now_ms()) {
            self.log
                .record("status", json!({"key": key, "status": status}));
            (self.emit)(SessionEvent {
                key: key.to_string(),
                event: SessionUpdate::Status(StatusChanged { status }),
            });
        }
        Ok(())
    }

    /// Stops a session's process. It waits for a start in progress to
    /// finish, so that process is stopped too, and holds the session's lock
    /// until the process has gone, so a reopen never overlaps it.
    pub async fn close(&self, key: &str) {
        let _guard = self.registry.lifecycle(key).await;
        let open = self.registry.open().await.remove(key);
        if let Some(open) = open {
            self.log.record("sessionClosed", json!({"key": key}));
            open.session.shutdown().await;
        }
    }

    /// Ends every open session's process, all at once.
    pub async fn close_all(&self) {
        let sessions: Vec<_> = self
            .registry
            .open()
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
        self.store
            .call(move |store| match panoptes_id(&key) {
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
        self.store
            .call(move |store| store.set_session_archived(id, archived))
            .await
            .map_err(err)
    }

    pub async fn rename(&self, key: &str, title: Option<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("renamed"))?;
        self.store
            .call(move |store| store.set_session_title(id, title.as_deref()))
            .await
            .map_err(err)
    }

    pub async fn set_tags(&self, key: &str, tags: Vec<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("tagged"))?;
        self.store
            .call(move |store| store.set_session_tags(id, &tags))
            .await
            .map_err(err)
    }

    /// Registers the git repository `folder` is in, by its main checkout,
    /// with the default settings. Registering a repository twice fails.
    pub async fn add_project(&self, folder: String) -> ServiceResult<Project> {
        self.store
            .call(move |store| {
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
        self.store
            .call(move |store| store.update_project(id, &projects::validate(settings)?))
            .await
            .map_err(err)
    }

    /// Forgets a project. Nothing on disk changes, and its sessions are
    /// listed by folder again.
    pub async fn remove_project(&self, id: i64) -> ServiceResult<()> {
        self.store
            .call(move |store| store.remove_project(id))
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
                let store = store.lock();
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
            let added = store.lock().add_lane(&new);
            added.inspect_err(|_| lanes::discard_created(&project, &new))
        })
        .await
        .context("The lane creation stopped")
        .and_then(|result| result);
        self.lane_cancellations.lock().unwrap().remove(&project_id);
        match result {
            Ok(lane) => {
                self.log.record(
                    "laneCreated",
                    json!({
                        "lane": lane.id,
                        "project": lane.project_id,
                        "mainCheckout": lane.worktrees.iter().any(|tree| tree.main_checkout),
                    }),
                );
                Ok(lane)
            }
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

    /// Disk usage and safely archivable idle lanes for one project. This is
    /// deliberately on demand: measuring worktrees is too costly for `list`.
    pub async fn lane_cleanup(&self, project_id: i64) -> ServiceResult<LaneCleanup> {
        let listing = self.list().await;
        let activity_complete = listing.errors.is_empty();
        let (idle_days, creation_times) = self
            .store
            .call(move |store| {
                store.project(project_id)?;
                Ok((
                    store.preferences()?.cleanup_idle_days,
                    store.lane_creation_times()?,
                ))
            })
            .await
            .map_err(err)?;
        let created: HashMap<i64, i64> = creation_times.into_iter().collect();
        let active: Vec<(Project, Lane)> = listing
            .lanes
            .iter()
            .filter(|lane| lane.state == LaneState::Active)
            .filter_map(|lane| {
                let project = listing
                    .projects
                    .iter()
                    .find(|project| project.id == lane.project_id)?;
                Some((project.clone(), lane.clone()))
            })
            .collect();
        let scan = tokio::task::spawn_blocking(move || cleanup::scan(project_id, &active))
            .await
            .context("The worktree cleanup scan stopped")
            .map_err(err)?;

        // Starting sessions are not listed as open yet. Include both sets so
        // cleanup can never archive a lane under a process that is starting.
        let in_use = self.registry.lanes_in_use().await;
        let busy_lanes = self.busy_lanes.lock().unwrap().clone();
        let busy_writers: HashSet<i64> = listing
            .navigation
            .iter()
            .filter(|entry| entry.busy_writer.is_some())
            .map(|entry| entry.lane_id)
            .collect();

        let mut candidates = Vec::new();
        if activity_complete {
            let now = now_ms();
            for lane in listing
                .lanes
                .iter()
                .filter(|lane| lane.project_id == project_id && lane.state == LaneState::Active)
            {
                if in_use.contains(&lane.id)
                    || busy_lanes.contains(&lane.id)
                    || busy_writers.contains(&lane.id)
                {
                    continue;
                }
                let last_activity = listing
                    .sessions
                    .iter()
                    .filter(|session| session.lane_id == Some(lane.id))
                    .fold(
                        created.get(&lane.id).copied().unwrap_or(0),
                        |latest, session| latest.max(session.updated_at),
                    );
                if !cleanup::is_idle(last_activity, now, idle_days) {
                    continue;
                }
                let Some(scanned) = scan.by_lane.get(&lane.id) else {
                    continue;
                };
                let Some(preservation) = scanned.preservation else {
                    continue;
                };
                candidates.push(IdleLane {
                    lane: lane.clone(),
                    last_activity,
                    bytes: scanned.bytes,
                    preservation,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.bytes
                .unwrap_or(0)
                .cmp(&a.bytes.unwrap_or(0))
                .then_with(|| a.last_activity.cmp(&b.last_activity))
        });
        let mut errors = listing.errors;
        errors.extend(scan.errors);
        Ok(LaneCleanup {
            project: scan.project,
            total: scan.total,
            idle_days,
            candidates,
            errors,
        })
    }

    /// Archives a clean lane immediately. A dirty one is returned unchanged
    /// until the caller picks how its work should be preserved.
    #[cfg(test)]
    pub async fn archive_lane(
        &self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
    ) -> ServiceResult<ArchiveLaneOutcome> {
        self.archive_lane_recovering(id, strategy, false).await
    }

    /// Archives after optionally performing the branch recovery the user
    /// explicitly chose in the detached-HEAD dialog.
    pub async fn archive_lane_recovering(
        &self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
        recreate_branch: bool,
    ) -> ServiceResult<ArchiveLaneOutcome> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
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
                let store = store.lock();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Active {
                    anyhow::bail!("That lane is already archived");
                }
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            // The checkpoint ref is stored before the folder goes, so an
            // archive that stops in between can be finished later.
            let record = |reference: Option<&str>| store.lock().set_lane_checkpoint(id, reference);
            match lanes::archive_recovering(&project, &lane, strategy, recreate_branch, &record)? {
                ArchiveResult::Dirty => Ok(ArchiveLaneOutcome::Dirty { lane }),
                ArchiveResult::DetachedHead(detached_head) => {
                    Ok(ArchiveLaneOutcome::DetachedHead {
                        lane,
                        detached_head,
                    })
                }
                ArchiveResult::Archived { checkpoint_ref } => {
                    store.lock().archive_lane(id, checkpoint_ref.as_deref())?;
                    Ok(ArchiveLaneOutcome::Archived)
                }
            }
        })
        .await
        .context("The lane archive stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|outcome| {
            if matches!(outcome, ArchiveLaneOutcome::Archived) {
                self.log
                    .record("laneArchived", json!({"lane": id, "strategy": strategy}));
            }
        })
    }

    /// Saves a detached HEAD on the lane branch without removing the
    /// worktree, so the user can then choose how to archive its changes.
    pub async fn create_lane_branch(&self, id: i64) -> ServiceResult<()> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's sessions before recovering its branch",
            ));
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let lane = {
                let store = store.lock();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Active {
                    anyhow::bail!("That lane is archived; restore it before recovering its branch");
                }
                lane
            };
            let worktree = lane
                .worktrees
                .first()
                .filter(|_| lane.worktrees.len() == 1)
                .context("This operation needs exactly one worktree")?;
            lanes::create_branch_here(Path::new(&worktree.path), &lane)
        })
        .await
        .context("The lane branch recovery stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    pub async fn restore_lane(&self, id: i64) -> ServiceResult<Lane> {
        let _busy = self.lane_busy(id)?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock();
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
            store.lock().restore_lane(id)
        })
        .await
        .context("The lane restore stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|_| self.log.record("laneRestored", json!({"lane": id})))
    }

    /// Permanently deletes a lane after the UI's explicit confirmation.
    /// Open sessions are refused rather than killed underneath their user.
    #[cfg(test)]
    pub async fn delete_lane(&self, id: i64) -> ServiceResult<DeleteLaneOutcome> {
        self.delete_lane_detached(id, false).await
    }

    /// `allow_detached` is accepted only for a clean, reachable detached HEAD
    /// and is checked again by the lane operation.
    pub async fn delete_lane_detached(
        &self,
        id: i64,
        allow_detached: bool,
    ) -> ServiceResult<DeleteLaneOutcome> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's session before deleting it",
            ));
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock();
                let lane = store.lane(id)?;
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            match lanes::delete_detached(&project, &lane, allow_detached)? {
                DeleteResult::Deleted => {
                    store.lock().delete_lane(id)?;
                    Ok(DeleteLaneOutcome::Deleted)
                }
                DeleteResult::DetachedHead(detached_head) => Ok(DeleteLaneOutcome::DetachedHead {
                    lane,
                    detached_head,
                }),
            }
        })
        .await
        .context("The lane deletion stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|outcome| {
            if matches!(outcome, DeleteLaneOutcome::Deleted) {
                self.log.record("laneDeleted", json!({"lane": id}));
            }
        })
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
mod tests;
