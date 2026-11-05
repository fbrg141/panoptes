//! The boundary between Panoptes and the agent harnesses it drives.
//!
//! Everything the app and the UI see is expressed in these types. A harness
//! adapter (Pi first, others later) maps its own protocol onto them, so no
//! harness-specific detail leaks past this crate. Features that only one
//! harness has travel as [`HarnessEvent::Raw`] until they earn a neutral type.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

/// A session as listed in the sidebar, before it is opened.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// Which harness owns the session, e.g. `"pi"`.
    pub harness: String,
    /// The harness's own session id.
    pub id: String,
    /// Opaque harness-specific handle used to resume the session
    /// (for Pi, the session file path).
    pub locator: String,
    /// Working directory the session runs in.
    pub cwd: String,
    /// Display name, or the first user message when the session has no name.
    pub title: String,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last activity, Unix milliseconds.
    pub updated_at: i64,
    pub message_count: u32,
    /// Locator of the session this one was forked or cloned from.
    pub parent_locator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
    Tool,
    /// Anything else a harness stores in the conversation (summaries,
    /// extension messages). Rendered as a muted note.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Block {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        call_id: String,
        name: String,
        args: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        mime_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Stable id from the harness's history when known; a live id
    /// (see [`HarnessEvent::MessageStarted`]) while a message streams.
    pub id: String,
    pub role: Role,
    pub blocks: Vec<Block>,
    /// Unix milliseconds.
    pub timestamp: Option<i64>,
    /// Set when the harness reports the message failed (provider error, abort).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeltaKind {
    Text,
    Thinking,
    ToolArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// Something the harness (or one of its extensions) needs the user to answer
/// before it can continue.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    pub kind: QuestionKind,
    /// When the harness gives up waiting and picks a default, Unix
    /// milliseconds. The UI should stop offering the question then.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum QuestionKind {
    /// Pick one of `options`; answered with [`Answer::Value`].
    Select { options: Vec<String> },
    /// Yes or no; answered with [`Answer::Confirmed`].
    Confirm,
    /// One line of text; answered with [`Answer::Value`].
    Input { placeholder: Option<String> },
    /// Multi-line text; answered with [`Answer::Value`].
    Editor { prefill: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Answer {
    Value { value: String },
    Confirmed { confirmed: bool },
    Cancelled,
}

/// Everything a running session reports. Streaming state is rebuilt from
/// these, and the completed forms replace the streamed ones.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HarnessEvent {
    /// The agent started working on a prompt.
    RunStarted,
    #[serde(rename_all = "camelCase")]
    MessageStarted {
        message_id: String,
        role: Role,
    },
    #[serde(rename_all = "camelCase")]
    BlockStarted {
        message_id: String,
        index: usize,
        block: Block,
    },
    #[serde(rename_all = "camelCase")]
    BlockDelta {
        message_id: String,
        index: usize,
        kind: DeltaKind,
        delta: String,
    },
    /// The authoritative final message. Replaces whatever was streamed under
    /// the same `message.id`.
    MessageCompleted {
        message: Message,
    },
    #[serde(rename_all = "camelCase")]
    ToolStarted {
        call_id: String,
        name: String,
        args: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolUpdated {
        call_id: String,
        output: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolFinished {
        call_id: String,
        output: String,
        is_error: bool,
    },
    /// The agent will do no more work until the next prompt.
    RunSettled {
        aborted: bool,
    },
    TitleChanged {
        title: Option<String>,
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
    /// Answer with [`AgentSession::answer`].
    Question {
        question: Question,
    },
    /// The harness process ended. The session can be reopened.
    #[serde(rename_all = "camelCase")]
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    /// A harness-specific record with no neutral meaning yet.
    Raw {
        payload: Value,
    },
}

pub type EventSink = UnboundedSender<HarnessEvent>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartSession {
    New { cwd: PathBuf },
    Resume { locator: String, cwd: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptMode {
    /// Start a run. Fails if the agent is busy.
    Start,
    /// Deliver while the agent is running, before its next model call.
    Steer,
    /// Deliver once the agent has finished.
    FollowUp,
}

/// An image sent with a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub mime_type: String,
    /// The image bytes, base64-encoded.
    pub data: String,
}

/// A model the harness can run a session on.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// Opaque handle passed back to [`AgentSession::set_model`].
    pub id: String,
    /// Display name.
    pub name: String,
    /// Who serves the model, e.g. `"anthropic"`.
    pub provider: String,
    /// Context window in tokens, when known.
    pub context_window: Option<u64>,
    /// Accepts images in prompts.
    pub images: bool,
}

/// How full the model's context window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens in the context now; `None` while unknown (just after the
    /// harness compacted the conversation, say).
    pub tokens: Option<u64>,
    pub window: u64,
}

/// A session's model, reasoning effort and context use.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSettings {
    pub model: Option<ModelInfo>,
    /// Current thinking level, one of `thinking_levels`.
    pub thinking_level: Option<String>,
    /// Levels the current model supports, lowest first. Empty when the
    /// harness has no such setting.
    pub thinking_levels: Vec<String>,
    pub context: Option<ContextUsage>,
}

/// What the harness did with a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptOutcome {
    /// A run started; [`HarnessEvent::RunSettled`] will follow.
    Started,
    /// Queued behind the current run.
    Queued,
    /// Consumed straight away (a slash command, say); no run follows.
    Handled,
}

/// Identity of a started session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub harness: String,
    pub id: String,
    pub locator: String,
    pub cwd: String,
}

/// An installed harness: lists its sessions and starts them.
#[async_trait]
pub trait Harness: Send + Sync {
    /// Short stable name, e.g. `"pi"`.
    fn id(&self) -> &'static str;

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>>;

    /// Starts (or resumes) a session. Its events go to `events` until the
    /// session shuts down or the harness exits.
    async fn start_session(
        &self,
        request: StartSession,
        events: EventSink,
    ) -> anyhow::Result<Box<dyn AgentSession>>;
}

/// A live, open session.
#[async_trait]
pub trait AgentSession: Send + Sync {
    fn session_ref(&self) -> SessionRef;

    /// The active conversation, oldest first, with stable message ids.
    async fn history(&self) -> anyhow::Result<Vec<Message>>;

    async fn prompt(
        &self,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> anyhow::Result<PromptOutcome>;

    async fn abort(&self) -> anyhow::Result<()>;

    /// Answers a [`HarnessEvent::Question`].
    async fn answer(&self, question_id: String, answer: Answer) -> anyhow::Result<()>;

    /// The session's model, thinking level and context use. A harness
    /// without these settings reports none.
    async fn settings(&self) -> anyhow::Result<SessionSettings> {
        Ok(SessionSettings::default())
    }

    /// Models the session can switch to.
    async fn models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    /// Switches model from the next message on. `model_id` is a
    /// [`ModelInfo::id`].
    async fn set_model(&self, _model_id: String) -> anyhow::Result<()> {
        anyhow::bail!("This harness cannot switch models")
    }

    /// Sets the reasoning effort from the next message on. `level` is one of
    /// [`SessionSettings::thinking_levels`].
    async fn set_thinking_level(&self, _level: String) -> anyhow::Result<()> {
        anyhow::bail!("This harness has no thinking levels")
    }

    /// Ends the session's process. The session stays resumable.
    async fn shutdown(&self);
}
