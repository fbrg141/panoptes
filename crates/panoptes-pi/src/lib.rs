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
    AgentSession, Answer, EventSink, Harness, Message, PromptMode, PromptOutcome, SessionRef,
    SessionSummary, StartSession,
};
use serde_json::{Value, json};

pub use locate::{PiInstall, SUPPORTED_MAJOR_VERSIONS, locate, sessions_dir};

pub struct PiHarness {
    install: PiInstall,
    sessions_dir: PathBuf,
    cache: Arc<Mutex<sessions::SessionCache>>,
}

impl PiHarness {
    /// Uses the session folder Pi itself is configured with.
    pub fn new(install: PiInstall) -> Self {
        Self::with_sessions_dir(install, sessions_dir())
    }

    pub fn with_sessions_dir(install: PiInstall, sessions_dir: PathBuf) -> Self {
        Self {
            install,
            sessions_dir,
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
        // A session whose folder has since been deleted still opens, from
        // the root, so its history stays readable.
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
            id: state["sessionId"]
                .as_str()
                .context("Pi did not report a session id")?
                .to_string(),
            locator: state["sessionFile"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
        };
        Ok(Box::new(PiSession {
            process,
            session_ref,
            entries: tokio::sync::Mutex::default(),
        }))
    }
}

/// Session entries fetched so far. Pi's entry ids are durable cursors, so
/// later fetches ask only for what was appended since the last one.
#[derive(Default)]
struct EntryCache {
    entries: Vec<Value>,
}

impl EntryCache {
    fn last_id(&self) -> Option<&str> {
        self.entries.last().and_then(|entry| entry["id"].as_str())
    }
}

struct PiSession {
    process: rpc::RpcProcess,
    session_ref: SessionRef,
    entries: tokio::sync::Mutex<EntryCache>,
}

#[async_trait]
impl AgentSession for PiSession {
    fn session_ref(&self) -> SessionRef {
        self.session_ref.clone()
    }

    async fn history(&self) -> anyhow::Result<Vec<Message>> {
        let mut cache = self.entries.lock().await;
        let data = match cache.last_id() {
            None => self.process.call(json!({"type": "get_entries"})).await?,
            Some(since) => {
                let since = since.to_string();
                match self
                    .process
                    .call(json!({"type": "get_entries", "since": since}))
                    .await
                {
                    Ok(data) => data,
                    // The cursor is unknown to Pi (the session was rewritten
                    // elsewhere): start over from the full list.
                    Err(_) => {
                        cache.entries.clear();
                        self.process.call(json!({"type": "get_entries"})).await?
                    }
                }
            }
        };
        if let Some(new_entries) = data["entries"].as_array() {
            cache.entries.extend(new_entries.iter().cloned());
        }
        Ok(map::history(&cache.entries, data["leafId"].as_str()))
    }

    async fn prompt(&self, text: String, mode: PromptMode) -> anyhow::Result<PromptOutcome> {
        let mut command = json!({"type": "prompt", "message": text});
        match mode {
            PromptMode::Start => {}
            PromptMode::Steer => command["streamingBehavior"] = json!("steer"),
            PromptMode::FollowUp => command["streamingBehavior"] = json!("followUp"),
        }
        let data = self.process.call(command).await?;
        Ok(match data["disposition"].as_str() {
            Some("queued") => PromptOutcome::Queued,
            Some("handled") => PromptOutcome::Handled,
            _ => PromptOutcome::Started,
        })
    }

    async fn abort(&self) -> anyhow::Result<()> {
        self.process
            .call(json!({"type": "abort"}))
            .await
            .map(|_| ())
    }

    async fn answer(&self, question_id: String, answer: Answer) -> anyhow::Result<()> {
        let mut response = json!({"type": "extension_ui_response", "id": question_id});
        match answer {
            Answer::Value { value } => response["value"] = json!(value),
            Answer::Confirmed { confirmed } => response["confirmed"] = json!(confirmed),
            Answer::Cancelled => response["cancelled"] = json!(true),
        }
        self.process.notify(response)
    }

    async fn shutdown(&self) {
        self.process.shutdown().await;
    }
}
