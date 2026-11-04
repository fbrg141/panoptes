//! The app's session registry: which harnesses are loaded, which sessions
//! are open, and Panoptes' own index. Independent of Tauri so it can be
//! tested with a fake harness; `lib.rs` exposes it as commands.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use panoptes_harness::{
    AgentSession, Answer, Harness, HarnessEvent, Message, PromptMode, PromptOutcome, SessionRef,
    SessionSummary, StartSession,
};
use serde::Serialize;
use tokio::sync::{Mutex, oneshot};

use crate::index::Index;

/// Errors cross into the UI as display strings.
pub type ServiceResult<T> = Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

pub fn session_key(harness: &str, id: &str) -> String {
    format!("{harness}:{id}")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub key: String,
    pub event: HarnessEvent,
}

pub type Emit = Arc<dyn Fn(SessionEvent) + Send + Sync>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarSession {
    pub key: String,
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub project_root: String,
    pub pinned: bool,
    pub open: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub sessions: Vec<SidebarSession>,
    /// One line per harness whose sessions could not be listed.
    pub errors: Vec<String>,
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
}

pub struct Service {
    harnesses: HashMap<String, Arc<dyn Harness>>,
    open: Arc<Mutex<HashMap<String, OpenSession>>>,
    /// One lock per session key while it is being opened, so concurrent
    /// opens of one session start one process, never two writers on one
    /// session file.
    opening: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    generations: AtomicU64,
    index: std::sync::Mutex<Index>,
    emit: Emit,
}

impl Service {
    pub fn new(harnesses: Vec<Arc<dyn Harness>>, index: Index, emit: Emit) -> Self {
        Self {
            harnesses: harnesses
                .into_iter()
                .map(|harness| (harness.id().to_string(), harness))
                .collect(),
            open: Arc::default(),
            opening: Mutex::default(),
            generations: AtomicU64::new(1),
            index: std::sync::Mutex::new(index),
            emit,
        }
    }

    fn harness(&self, id: &str) -> ServiceResult<Arc<dyn Harness>> {
        self.harnesses
            .get(id)
            .cloned()
            .ok_or_else(|| format!("The {id} harness is not available"))
    }

    async fn session(&self, key: &str) -> ServiceResult<Arc<dyn AgentSession>> {
        self.open
            .lock()
            .await
            .get(key)
            .map(|open| open.session.clone())
            .ok_or_else(|| "This session is not open".to_string())
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
        let open = self.open.lock().await;
        let mut index = self.index.lock().unwrap();
        let sessions = summaries
            .into_iter()
            .map(|summary| {
                let key = session_key(&summary.harness, &summary.id);
                SidebarSession {
                    project_root: index.project_root(&summary.cwd),
                    pinned: index.file.pinned.contains(&key),
                    open: open.contains_key(&key),
                    key,
                    summary,
                }
            })
            .collect();
        Listing { sessions, errors }
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
        {
            let emit = self.emit.clone();
            let open = self.open.clone();
            tokio::spawn(async move {
                // The key is known only once the harness reports the session
                // id; anything said before that is held back until then.
                let mut early = Vec::new();
                let key = loop {
                    tokio::select! {
                        key = &mut key_rx => match key {
                            Ok(key) => break key,
                            Err(_) => return, // the start failed
                        },
                        Some(event) = rx.recv() => early.push(event),
                    }
                };
                for event in early {
                    emit(SessionEvent {
                        key: key.clone(),
                        event,
                    });
                }
                while let Some(event) = rx.recv().await {
                    if matches!(event, HarnessEvent::Exited { .. }) {
                        let mut open = open.lock().await;
                        let current = open.get(&key).is_some_and(|o| o.generation == generation);
                        if !current {
                            // Closed on purpose or replaced: not news.
                            continue;
                        }
                        open.remove(&key);
                    }
                    emit(SessionEvent {
                        key: key.clone(),
                        event,
                    });
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
        mode: PromptMode,
    ) -> ServiceResult<PromptOutcome> {
        self.session(key)
            .await?
            .prompt(text, mode)
            .await
            .map_err(err)
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
        self.session(key)
            .await?
            .answer(question_id, answer)
            .await
            .map_err(err)
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

    pub fn set_pinned(&self, key: String, pinned: bool) -> ServiceResult<()> {
        let mut index = self.index.lock().unwrap();
        if pinned {
            index.file.pinned.insert(key);
        } else {
            index.file.pinned.remove(&key);
        }
        index.save().map_err(err)
    }
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
            Ok(vec![SessionSummary {
                harness: "fake".into(),
                id: "s1".into(),
                locator: "s1".into(),
                cwd: "/tmp".into(),
                title: "One".into(),
                created_at: 0,
                updated_at: 0,
                message_count: 0,
                parent_locator: None,
            }])
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
        async fn prompt(&self, _: String, _: PromptMode) -> anyhow::Result<PromptOutcome> {
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
            Index::load(dir.path().join("index.json")),
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
            .filter(|e| matches!(e.event, HarnessEvent::Exited { .. }))
            .count()
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
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key, "fake:new");
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
        assert!(!f.service.list().await.sessions[0].open);
        assert!(f.service.history("fake:s1").await.is_err());
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
    async fn pins_are_saved_to_the_index() {
        let f = fixture(FakeHarness::default());
        f.service.set_pinned("fake:s1".into(), true).unwrap();
        assert!(f.service.list().await.sessions[0].pinned);
        let saved = std::fs::read_to_string(f._dir.path().join("index.json")).unwrap();
        assert!(saved.contains("fake:s1"));
    }

    #[tokio::test]
    async fn unknown_harnesses_and_sessions_are_errors() {
        let f = fixture(FakeHarness::default());
        assert!(f.service.create("nope", "/tmp".into()).await.is_err());
        assert!(f.service.abort("fake:missing").await.is_err());
    }
}
