use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use panoptes_harness::{AgentSession, HarnessEvent, Message, SessionStatus, StartSession};
use serde::Serialize;
use serde_json::json;

use super::error::{ServiceError, ServiceResult, err, only_in_a_lane};
use super::forward::{Forwarder, WriteWatch};
use super::key::{SessionKey, panoptes_id, panoptes_key};
use super::lanes::{is_busy, lane_cwd, lane_session_busy};
use super::registry::{EarlyExit, Launch, OpenSession, busy_writer};
use super::status::{Tracker, now_ms};
use super::{ErrorKind, Service, SessionEvent, SessionUpdate, StatusChanged};
use crate::store::{Lane, LaneState};

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

impl Service {
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
            SessionKey::Panoptes => key.to_string(),
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
        let current = self.registry.live_in_lane(lane_id).await;
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
    pub(super) async fn close_lane_previews(&self, lane_id: i64) {
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
            .live_in_lane(lane_id)
            .await
            .and_then(|key| panoptes_id(&key));
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
        if matches!(SessionKey::parse(harness, key), SessionKey::Panoptes) {
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

    /// Closes lane `id`'s open sessions so the lane can be archived or
    /// deleted, and returns their keys. A session that is starting, Running
    /// or Waiting is refused rather than stopped underneath its user. Any
    /// other holds nothing to lose, since its conversation stays in the
    /// harness's files. A session that starts running while the others are
    /// being closed is left open, and the archive or delete then refuses it.
    pub async fn close_lane_sessions(&self, id: i64) -> ServiceResult<Vec<String>> {
        if self.registry.is_starting_in_lane(id) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "A session in this lane is still starting; try again once it has opened",
            ));
        }
        let keys: Vec<String> = {
            let open = self.registry.open().await;
            let in_lane = || open.iter().filter(|(_, o)| o.lane == Some(id));
            if let Some(status) = in_lane()
                .map(|(_, o)| o.status.lock().unwrap().status())
                .find(|status| is_busy(*status))
            {
                return Err(lane_session_busy(status));
            }
            in_lane().map(|(key, _)| key.clone()).collect()
        };
        let mut closed = Vec::new();
        for key in keys {
            let _guard = self.registry.lifecycle(&key).await;
            let session = {
                let mut open = self.registry.open().await;
                let idle = open
                    .get(&key)
                    .is_some_and(|o| !is_busy(o.status.lock().unwrap().status()));
                if !idle {
                    continue;
                }
                open.remove(&key)
            };
            if let Some(session) = session {
                self.log.record("sessionClosed", json!({"key": key}));
                session.session.shutdown().await;
                closed.push(key);
            }
        }
        Ok(closed)
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
}

fn same_or_inside(path: &Path, root: &Path) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.starts_with(root)
}
