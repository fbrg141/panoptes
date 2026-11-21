use super::super::*;
use async_trait::async_trait;
use panoptes_harness::{EventSink, Role};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

/// A harness whose sessions are in-memory; it counts how many processes
/// it was asked to start.
#[derive(Default)]
pub(super) struct FakeHarness {
    pub(super) starts: AtomicUsize,
    pub(super) fail_listing: bool,
    /// Where its sessions run, one session per folder; `/tmp` when empty.
    pub(super) cwds: Vec<String>,
    pub(super) sinks: std::sync::Mutex<Vec<EventSink>>,
    /// Every start it was asked for, in order.
    pub(super) requests: std::sync::Mutex<Vec<StartSession>>,
    /// Its process ends right after it reports its session.
    pub(super) dies_on_start: bool,
    /// Processes started and not yet shut down, and the most there ever
    /// were at once.
    pub(super) live: Arc<Live>,
}

#[derive(Default)]
pub(super) struct Live {
    pub(super) now: AtomicUsize,
    pub(super) most: AtomicUsize,
}

impl FakeHarness {
    /// Which starts were read-only, in order.
    pub(super) fn read_only(&self) -> Vec<bool> {
        let requests = self.requests.lock().unwrap();
        requests.iter().map(|request| request.read_only).collect()
    }
}

pub(super) struct FakeSession {
    pub(super) id: String,
    pub(super) cwd: String,
    pub(super) events: EventSink,
    pub(super) live: Arc<Live>,
    pub(super) down: AtomicBool,
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
        let now = self.live.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.live.most.fetch_max(now, Ordering::SeqCst);
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
        Ok(Box::new(FakeSession {
            id,
            cwd,
            events,
            live: self.live.clone(),
            down: AtomicBool::new(false),
        }))
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
    fn unguarded_tools(&self) -> bool {
        false
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
        // A process takes a moment to go, as Pi's does.
        tokio::time::sleep(Duration::from_millis(30)).await;
        if !self.down.swap(true, Ordering::SeqCst) {
            self.live.now.fetch_sub(1, Ordering::SeqCst);
        }
        let _ = self.events.send(HarnessEvent::Exited {
            code: Some(0),
            stderr_tail: String::new(),
        });
    }
}

pub(super) struct Fixture {
    pub(super) service: Arc<Service>,
    pub(super) harness: Arc<FakeHarness>,
    pub(super) events: Arc<std::sync::Mutex<Vec<SessionEvent>>>,
    /// Woken whenever an event is emitted, so a test can wait for one
    /// rather than sleep for how long it usually takes.
    pub(super) emitted: Arc<tokio::sync::Notify>,
    pub(super) _dir: tempfile::TempDir,
}

pub(super) fn fixture(harness: FakeHarness) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let harness = Arc::new(harness);
    let events: Arc<std::sync::Mutex<Vec<SessionEvent>>> = Arc::default();
    let sink = events.clone();
    let emitted: Arc<tokio::sync::Notify> = Arc::default();
    let wake = emitted.clone();
    let service = Service::new(
        vec![harness.clone()],
        Store::open(dir.path()).unwrap(),
        dir.path().join("lanes"),
        Arc::new(move |event| {
            sink.lock().unwrap().push(event);
            // `notify_one` keeps a permit when nothing is waiting, so an
            // event emitted before a test starts waiting is not lost.
            wake.notify_one();
        }),
    )
    .with_event_log(EventLog::open(&dir.path().join("logs")));
    Fixture {
        service: Arc::new(service),
        harness,
        events,
        emitted,
        _dir: dir,
    }
}

pub(super) async fn settle() {
    tokio::time::sleep(Duration::from_millis(20)).await;
}

/// Waits until an emitted event matches `wanted`, which is a real sync
/// point rather than a guess at how long a step takes.
pub(super) async fn wait_for_event(
    f: &Fixture,
    what: &str,
    wanted: impl Fn(&SessionEvent) -> bool,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let woken = f.emitted.notified();
        if f.events.lock().unwrap().iter().any(&wanted) {
            return;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::time::timeout(left, woken)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }
}

pub(super) fn exits(fixture: &Fixture) -> usize {
    let events = fixture.events.lock().unwrap();
    events
        .iter()
        .filter(|e| matches!(e.event, SessionUpdate::Harness(HarnessEvent::Exited { .. })))
        .count()
}

pub(super) fn statuses(fixture: &Fixture) -> Vec<SessionStatus> {
    let events = fixture.events.lock().unwrap();
    events
        .iter()
        .filter_map(|e| match e.event {
            SessionUpdate::Status(StatusChanged { status }) => Some(status),
            _ => None,
        })
        .collect()
}

pub(super) async fn open_s1(fixture: &Fixture) -> EventSink {
    fixture
        .service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    fixture.harness.sinks.lock().unwrap()[0].clone()
}

pub(super) fn assistant(error: Option<&str>) -> HarnessEvent {
    HarnessEvent::MessageCompleted {
        message: Message {
            id: "m1".into(),
            role: Role::Assistant,
            blocks: Vec::new(),
            timestamp: None,
            error: error.map(Into::into),
            stopped: false,
            usage: None,
        },
    }
}

pub(super) fn question(id: &str, expires_at: Option<i64>) -> HarnessEvent {
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

pub(super) async fn lane_of(f: &Fixture) -> (tempfile::TempDir, Lane) {
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

pub(super) fn git(dir: &Path, args: &[&str]) {
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

pub(super) async fn two_lanes(f: &Fixture) -> (tempfile::TempDir, Lane, Lane, EventSink) {
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
