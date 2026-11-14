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

/// A harness-owned conversation discovered on disk, before Panoptes adds
/// its own session identity and lane membership.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSession {
    #[serde(flatten)]
    pub harness_ref: HarnessRef,
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
    /// A tool call with everything known about it so far. Its status and
    /// output are kept up to date here; there is no separate result.
    ToolCall(Box<ToolCall>),
    #[serde(rename_all = "camelCase")]
    Image {
        mime_type: String,
        /// The image bytes, base64-encoded, when the harness keeps them.
        data: Option<String>,
    },
}

/// One tool call: what it does, where it stands and what it produced.
/// History and live events give the same values, so the UI keeps no tool
/// state of its own.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    /// The harness's name for the tool, for display.
    pub name: String,
    /// What the call does, for choosing how to show it.
    pub kind: ToolKind,
    /// The argument that says what the call acts on: a path, a command
    /// or a search pattern.
    pub subject: Option<String>,
    /// The arguments as the model sent them; `null` while they stream.
    pub input: Value,
    pub status: ToolStatus,
    /// `None` until there is something to show. Edits carry their changes
    /// from the start, since the input already says what they will be.
    pub output: Option<ToolOutput>,
    /// Unix milliseconds, when the harness reports them.
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    /// Written by the model but not started, or its arguments still stream.
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolOutput {
    Text {
        text: String,
    },
    /// File changes. `text` is what the tool said, such as why it failed.
    Edits {
        edits: Vec<FileEdit>,
        text: Option<String>,
    },
    /// A command's output. `exit_code` is `None` while it runs, or when it
    /// was stopped or timed out before exiting.
    #[serde(rename_all = "camelCase")]
    Shell {
        text: String,
        exit_code: Option<i32>,
    },
}

/// What a tool call does, so the UI can show it without knowing a harness's
/// tool names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolKind {
    Read,
    Edit,
    Search,
    Execute,
    Other,
}

/// One change a tool call makes to a file. `old_text` is `None` when the
/// call writes the whole file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEdit {
    pub path: String,
    pub old_text: Option<String>,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Set by the first event that mentions the message and kept for as
    /// long as the session is open: history reuses the id a message
    /// streamed under. A session opened later gets the harness's own ids.
    pub id: String,
    pub role: Role,
    pub blocks: Vec<Block>,
    /// Unix milliseconds.
    pub timestamp: Option<i64>,
    /// Set when the harness reports the message failed (a provider error).
    pub error: Option<String>,
    /// The user stopped the agent while this message was being written.
    pub stopped: bool,
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

/// A request to let one tool call go ahead, answered with
/// [`Answer::Decision`] (or [`Answer::Cancelled`], which rejects it).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    /// The [`ToolCall::id`] the decision is about.
    pub tool_call_id: String,
    pub title: String,
    pub message: Option<String>,
    /// When the harness gives up waiting and picks a default, Unix
    /// milliseconds.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Answer {
    Value {
        value: String,
    },
    Confirmed {
        confirmed: bool,
    },
    /// Allow or reject an [`Approval`].
    Decision {
        allow: bool,
    },
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
    /// A tool call started, produced output or finished. Replaces the
    /// tool-call block with the same id, wherever it is.
    ToolCallUpdated {
        call: Box<ToolCall>,
    },
    /// The prompts waiting for the agent changed. Lists them all, in the
    /// order they will be delivered.
    QueueChanged {
        queued: Vec<QueuedPrompt>,
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
    /// A tool call needs the user's go-ahead. Answer with
    /// [`AgentSession::answer`].
    Approval {
        approval: Approval,
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

/// Where an open session stands, in the words the UI shows. Worked out by
/// the app from [`HarnessEvent`]s, so every harness gets the same rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    /// Open, and nothing has happened since.
    Idle,
    /// The agent is working.
    Running,
    /// The agent needs an answer or an approval from the user.
    Waiting,
    /// The last run errored, or the harness process ended unexpectedly.
    Failed,
    /// The last run finished.
    Done,
}

/// Why a harness could not do what it was asked. Adapters return these
/// (wrapped in `anyhow::Error`) so the app can tell the cases apart without
/// reading error text. An error without one has no kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessError {
    pub kind: HarnessErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessErrorKind {
    /// The harness answered and refused the command.
    Rejected,
    /// The harness process is gone, so nothing can answer.
    Exited,
    /// The harness did not answer in time.
    TimedOut,
}

impl HarnessError {
    pub fn new(kind: HarnessErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self::new(HarnessErrorKind::Rejected, message)
    }

    pub fn exited(message: impl Into<String>) -> Self {
        Self::new(HarnessErrorKind::Exited, message)
    }

    pub fn timed_out(message: impl Into<String>) -> Self {
        Self::new(HarnessErrorKind::TimedOut, message)
    }

    /// The kind of the first [`HarnessError`] in `error`'s chain, if any.
    pub fn kind_of(error: &anyhow::Error) -> Option<HarnessErrorKind> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<HarnessError>())
            .map(|error| error.kind)
    }
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HarnessError {}

/// How to start a session's process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSession {
    pub cwd: PathBuf,
    /// The [`HarnessRef::locator`] of the conversation to resume; `None`
    /// starts a new one.
    pub resume: Option<String>,
    /// Gives the agent only tools that cannot change files (reading,
    /// searching, listing). Used for archived lanes and safe history fallback.
    pub read_only: bool,
    /// Extra instructions for a new session, appended to the harness's own
    /// system prompt. Never reapplied when an existing conversation resumes.
    pub instructions: Option<String>,
}

impl StartSession {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            resume: None,
            read_only: false,
            instructions: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptMode {
    /// Start a run. Fails if the agent is busy.
    Start,
    /// Deliver while the agent is running, before its next model call.
    Steer,
    /// Deliver once the agent has finished.
    FollowUp,
}

/// A prompt the harness holds until the agent can take it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedPrompt {
    /// Stays the same while the prompt waits.
    pub id: String,
    /// The text as the agent will read it (expanded, for a template).
    pub text: String,
    /// [`PromptMode::Steer`] or [`PromptMode::FollowUp`].
    pub mode: PromptMode,
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

/// Identity of a harness-owned session. Panoptes wraps this in its own
/// durable session record rather than treating it as the session's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRef {
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

    async fn list_sessions(&self) -> anyhow::Result<Vec<DiscoveredSession>>;

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
    fn harness_ref(&self) -> HarnessRef;

    /// The active conversation, oldest first, with stable message ids.
    async fn history(&self) -> anyhow::Result<Vec<Message>>;

    async fn prompt(
        &self,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> anyhow::Result<PromptOutcome>;

    async fn abort(&self) -> anyhow::Result<()>;

    /// Answers a [`HarnessEvent::Question`] or [`HarnessEvent::Approval`].
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
        Err(HarnessError::rejected("This harness cannot switch models").into())
    }

    /// Sets the reasoning effort from the next message on. `level` is one of
    /// [`SessionSettings::thinking_levels`].
    async fn set_thinking_level(&self, _level: String) -> anyhow::Result<()> {
        Err(HarnessError::rejected("This harness has no thinking levels").into())
    }

    /// Ends the session's process. The session stays resumable.
    async fn shutdown(&self);
}
