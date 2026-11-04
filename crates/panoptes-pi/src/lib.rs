//! Pi adapter: one `pi --mode rpc` process per open session.

mod locate;
mod map;
mod rpc;
mod sessions;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use panoptes_harness::{
    AgentSession, EventSink, Harness, Message, PromptMode, SessionRef, SessionSummary, StartSession,
};
use serde_json::json;

pub use locate::{PiInstall, SUPPORTED_MAJOR_VERSIONS, locate};

pub struct PiHarness {
    install: PiInstall,
    sessions_dir: PathBuf,
    cache: Arc<Mutex<sessions::SessionCache>>,
}

impl PiHarness {
    pub fn new(install: PiInstall) -> Self {
        Self {
            install,
            sessions_dir: locate::sessions_dir(),
            cache: Arc::default(),
        }
    }

    pub fn install(&self) -> &PiInstall {
        &self.install
    }
}

#[async_trait]
impl Harness for PiHarness {
    fn id(&self) -> &'static str {
        "pi"
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let cache = self.cache.clone();
        let root = self.sessions_dir.clone();
        Ok(tokio::task::spawn_blocking(move || cache.lock().unwrap().scan(&root)).await?)
    }

    async fn start_session(
        &self,
        request: StartSession,
        events: EventSink,
    ) -> anyhow::Result<Box<dyn AgentSession>> {
        let (cwd, file) = match &request {
            StartSession::New { cwd } => (cwd.as_path(), None),
            StartSession::Resume { locator, cwd } => (cwd.as_path(), Some(locator.as_str())),
        };
        let cwd = if cwd.is_dir() { cwd } else { Path::new("/") };
        let process = rpc::RpcProcess::spawn(&self.install, cwd, file, events)?;

        let state = match process.call(json!({"type": "get_state"})).await {
            Ok(state) => state,
            Err(error) => {
                process.shutdown().await;
                return Err(error.context("Pi started but did not report its session"));
            }
        };
        let session_ref = SessionRef {
            harness: "pi".to_string(),
            id: state["sessionId"].as_str().context("Pi did not report a session id")?.to_string(),
            locator: state["sessionFile"].as_str().unwrap_or_default().to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
        };
        Ok(Box::new(PiSession { process, session_ref }))
    }
}

struct PiSession {
    process: rpc::RpcProcess,
    session_ref: SessionRef,
}

#[async_trait]
impl AgentSession for PiSession {
    fn session_ref(&self) -> SessionRef {
        self.session_ref.clone()
    }

    async fn history(&self) -> anyhow::Result<Vec<Message>> {
        let data = self.process.call(json!({"type": "get_entries"})).await?;
        let entries = data["entries"].as_array().cloned().unwrap_or_default();
        Ok(map::history(&entries, data["leafId"].as_str()))
    }

    async fn prompt(&self, text: String, mode: PromptMode) -> anyhow::Result<()> {
        let mut command = json!({"type": "prompt", "message": text});
        match mode {
            PromptMode::Start => {}
            PromptMode::Steer => command["streamingBehavior"] = json!("steer"),
            PromptMode::FollowUp => command["streamingBehavior"] = json!("followUp"),
        }
        self.process.call(command).await.map(|_| ())
    }

    async fn abort(&self) -> anyhow::Result<()> {
        self.process.call(json!({"type": "abort"})).await.map(|_| ())
    }

    async fn shutdown(&self) {
        self.process.shutdown().await;
    }
}
