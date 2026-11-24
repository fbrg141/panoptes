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
    AgentSession, Answer, CommandInfo, CommandKind, DiscoveredSession, EventSink, Harness,
    HarnessError, HarnessRef, ImageAttachment, Message, ModelInfo, PromptMode, PromptOutcome,
    SessionSettings, SessionToggle, StartSession,
};
use serde_json::{Value, json};

pub use locate::{PiInstall, SUPPORTED_MAJOR_VERSIONS, locate, sessions_dir};

/// Why a session offers no codemode switch: Pi has no such tool, so the switch
/// would do nothing. A session that never reports at all is a different
/// fault, [`CONTROLS_MISSING`].
pub const CODEMODE_MISSING: &str = "Pi has no codemode tool in this session";

/// Panoptes' controls extension never reported itself: the adapter's extension
/// did not load, which is a bug rather than a Pi without codemode.
pub const CONTROLS_MISSING: &str =
    "Panoptes' session controls extension did not report in this session";

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

    async fn list_sessions(&self) -> anyhow::Result<Vec<DiscoveredSession>> {
        let cache = self.cache.clone();
        let root = self.sessions_dir.clone();
        Ok(tokio::task::spawn_blocking(move || cache.lock().unwrap().scan(&root)).await?)
    }

    async fn start_session(
        &self,
        request: StartSession,
        events: EventSink,
    ) -> anyhow::Result<Box<dyn AgentSession>> {
        let cwd = request.cwd.as_path();
        let file = request.resume.as_deref();
        // Old history stays readable if its folder has since been deleted,
        // but a fallback session can never write. A new session needs a real
        // working directory rather than silently starting at the root.
        let (cwd, read_only) = if cwd.is_dir() {
            (cwd, request.read_only)
        } else if file.is_some() {
            (Path::new("/"), true)
        } else {
            anyhow::bail!(
                "Working directory {} does not exist or is not a directory",
                cwd.display()
            );
        };
        let shared = map::Shared::default();
        for subject in request.allowed {
            shared.allow(&subject);
        }
        let mapper = map::EventMapper::new(shared.clone());
        let process = rpc::RpcProcess::spawn(
            &self.install,
            cwd,
            file,
            read_only,
            request.instructions.as_deref(),
            events,
            mapper,
        )?;

        let state = match process.call(json!({"type": "get_state"})).await {
            Ok(state) => state,
            Err(error) => {
                process.shutdown().await;
                return Err(error.context("Pi started but did not report its session"));
            }
        };
        let harness_ref = HarnessRef {
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
            harness_ref,
            entries: tokio::sync::Mutex::default(),
            shared,
            read_only,
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
    harness_ref: HarnessRef,
    entries: tokio::sync::Mutex<EntryCache>,
    shared: map::Shared,
    read_only: bool,
}

/// How long to wait for the controls extension to report the new codemode
/// state after its command was answered.
const CODEMODE_WAIT: std::time::Duration = std::time::Duration::from_millis(500);
const CODEMODE_STEP: std::time::Duration = std::time::Duration::from_millis(10);

impl PiSession {
    /// The switches the UI offers. Codemode is one only where Pi has the
    /// tool: offering it elsewhere would fail on every click.
    fn toggles(&self) -> Vec<SessionToggle> {
        if self.read_only {
            return Vec::new();
        }
        let enabled = match self.shared.codemode() {
            map::Codemode::Available { enabled } => enabled,
            // Not known before the first status line, and not there at all
            // without the tool: either way there is nothing to switch.
            map::Codemode::Unknown | map::Codemode::Unavailable => return Vec::new(),
        };
        vec![SessionToggle {
            id: "codemode".into(),
            label: "Codemode".into(),
            description: Some(
                "Let the model run JavaScript that calls tools and non-LLM models".into(),
            ),
            enabled,
        }]
    }

    /// Waits for the controls extension to report `enabled`. The extension
    /// writes its status before Pi answers the command, and both travel on
    /// the same stream, but the reader is a separate task: give it a moment
    /// rather than report a toggle that worked as unavailable.
    async fn wait_for_codemode(&self, enabled: bool) -> bool {
        let deadline = tokio::time::Instant::now() + CODEMODE_WAIT;
        loop {
            match self.shared.codemode() {
                map::Codemode::Available { enabled: actual } if actual == enabled => return true,
                map::Codemode::Unavailable => return false,
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(CODEMODE_STEP).await;
        }
    }
}

#[async_trait]
impl AgentSession for PiSession {
    fn harness_ref(&self) -> HarnessRef {
        self.harness_ref.clone()
    }

    fn unguarded_tools(&self) -> bool {
        !self.read_only && !self.shared.has_permission_guard()
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
        Ok(map::history(
            &cache.entries,
            data["leafId"].as_str(),
            &self.shared,
        ))
    }

    async fn prompt(
        &self,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> anyhow::Result<PromptOutcome> {
        let mut command = json!({"type": "prompt", "message": text});
        if !images.is_empty() {
            command["images"] = images
                .into_iter()
                .map(|image| json!({"type": "image", "data": image.data, "mimeType": image.mime_type}))
                .collect();
        }
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
        // An approval is a confirm or a select underneath; answer in kind.
        let approval = self.shared.take_approval(&question_id);
        let allowed = match (&answer, approval.as_ref()) {
            (Answer::Decision { allow }, _) => Some(*allow),
            // An option picked from a permission dialog allows when it reads
            // as an allow ("Allow Always").
            (Answer::Value { value }, Some(reply)) => {
                reply.options.as_ref().map(|_| map::is_allow(value))
            }
            _ => None,
        };
        let mut response = json!({"type": "extension_ui_response", "id": question_id});
        match &answer {
            Answer::Value { value } => response["value"] = json!(value),
            Answer::Confirmed { confirmed } => response["confirmed"] = json!(confirmed),
            Answer::Decision { allow } => {
                let options = approval.as_ref().and_then(|reply| reply.options.clone());
                response = map::dialog_response(&question_id, &options, *allow);
            }
            Answer::Cancelled => response["cancelled"] = json!(true),
        }
        if let (Some(true), Some(subject)) = (
            allowed,
            approval.as_ref().and_then(|reply| reply.subject.clone()),
        ) {
            // One allow covers the matching repeats in this session.
            self.shared.allow(&subject);
        }
        self.process.notify(response)
    }

    async fn settings(&self) -> anyhow::Result<SessionSettings> {
        let (state, levels, stats) = tokio::join!(
            self.process.call(json!({"type": "get_state"})),
            self.process
                .call(json!({"type": "get_available_thinking_levels"})),
            self.process.call(json!({"type": "get_session_stats"})),
        );
        let state = state?;
        let thinking_levels = levels?["levels"]
            .as_array()
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| Some(level.as_str()?.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(SessionSettings {
            model: map::model(&state["model"]),
            thinking_level: state["thinkingLevel"].as_str().map(str::to_string),
            thinking_levels,
            toggles: self.toggles(),
            // Usage is a nicety: a session without stats still has settings.
            context: stats.ok().as_ref().and_then(map::context_usage),
        })
    }

    async fn models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let data = self
            .process
            .call(json!({"type": "get_available_models"}))
            .await?;
        Ok(data["models"]
            .as_array()
            .map(|models| models.iter().filter_map(map::model).collect())
            .unwrap_or_default())
    }

    async fn commands(&self) -> anyhow::Result<Vec<CommandInfo>> {
        let data = self.process.call(json!({"type": "get_commands"})).await?;
        Ok(data["commands"]
            .as_array()
            .map(|commands| {
                commands
                    .iter()
                    .filter(|command| command["name"].as_str() != Some("panoptes-codemode"))
                    .filter_map(|command| {
                        Some(CommandInfo {
                            name: command["name"].as_str()?.to_string(),
                            description: command["description"].as_str().map(str::to_string),
                            kind: match command["source"].as_str() {
                                Some("extension") => CommandKind::Extension,
                                Some("prompt") => CommandKind::Prompt,
                                Some("skill") => CommandKind::Skill,
                                _ => CommandKind::Other,
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn set_model(&self, model_id: String) -> anyhow::Result<()> {
        let (provider, model) = map::model_ref(&model_id)
            .ok_or_else(|| HarnessError::rejected(format!("Unknown model {model_id}")))?;
        self.process
            .call(json!({"type": "set_model", "provider": provider, "modelId": model}))
            .await
            .map(|_| ())
    }

    async fn set_thinking_level(&self, level: String) -> anyhow::Result<()> {
        self.process
            .call(json!({"type": "set_thinking_level", "level": level}))
            .await
            .map(|_| ())
    }

    async fn set_toggle(&self, toggle_id: String, enabled: bool) -> anyhow::Result<()> {
        if toggle_id != "codemode" {
            return Err(
                HarnessError::rejected(format!("Unknown session toggle {toggle_id}")).into(),
            );
        }
        // A read-only session has no controls extension at all, and codemode
        // would let the model reach the tools that `--tools` leaves out.
        if self.read_only {
            return Err(HarnessError::rejected(
                "Codemode cannot be changed in a read-only session",
            )
            .into());
        }
        // Answer without sending anything when the switch is not there. A Pi
        // without the codemode tool would treat the command as a prompt, and
        // an extension that never reported is a fault worth naming rather
        // than hiding behind the same message.
        match self.shared.codemode() {
            map::Codemode::Unknown => return Err(anyhow::anyhow!(CONTROLS_MISSING)),
            map::Codemode::Unavailable => {
                return Err(HarnessError::rejected(CODEMODE_MISSING).into());
            }
            map::Codemode::Available { enabled: current } if current == enabled => return Ok(()),
            map::Codemode::Available { .. } => {}
        }
        let command = format!("/panoptes-codemode {}", if enabled { "on" } else { "off" });
        let data = self
            .process
            .call(json!({"type": "prompt", "message": command}))
            .await?;
        // The extension's status is what the switch reports, so it has to
        // have arrived before this answers; see `wait_for_codemode`.
        if data["disposition"].as_str() != Some("handled") || !self.wait_for_codemode(enabled).await
        {
            return Err(HarnessError::rejected(CODEMODE_MISSING).into());
        }
        Ok(())
    }

    async fn shutdown(&self) {
        self.process.shutdown().await;
    }
}
