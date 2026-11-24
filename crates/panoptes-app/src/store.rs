//! The app's state and every call to the backend.
//!
//! [`State`] is the plain data and the rules for changing it, testable
//! without a window or a backend. [`AppStore`] is the GPUI entity that owns
//! it, calls the service and applies the answers.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use gpui_kit::*;
use panoptes_core::cleanup::LaneCleanup;
use panoptes_core::files::Attachment;
use panoptes_core::lanes::{ArchiveStrategy, BranchRecovery};
use panoptes_core::service::{
    AdoptionOutcome, ArchiveLaneOutcome, DeleteLaneOutcome, ErrorKind, LaneNavigation, Listing,
    OpenedSession, Service, ServiceError, Session, SessionEvent, SessionUpdate,
};
use panoptes_core::store::{
    Lane, NotificationField, NotificationPreferences, Preferences, ProfileInput, Project,
};
use panoptes_harness::{
    Answer, CommandInfo, HarnessEvent, ImageAttachment, ModelInfo, PromptMode, PromptOutcome,
    SessionSettings, SessionStatus,
};

use crate::backend::Backend;
use crate::model::conversation::Conversation;
use crate::model::format::now_ms;
use crate::model::notify;

/// What a composer holds: its text, attachments and picked mentions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
    /// Paths picked from the mention list, to keep drawing them as chips.
    pub mentioned: Vec<String>,
}

/// How a new session is set up before its first prompt.
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub model: Option<String>,
    pub thinking: Option<String>,
    /// Stored with a new session and appended again when its process resumes.
    pub instructions: Option<String>,
    pub prompt: Option<String>,
    pub images: Vec<ImageAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    Loading,
    Ready,
    Error(String),
}

#[derive(Debug, Default)]
pub struct ArchiveLanesResult {
    pub archived: usize,
    pub failed: Vec<(i64, String)>,
}

/// Follow-up work an event asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Refresh,
    ReloadHistory(String),
    LoadSettings(String),
}

pub struct State {
    pub startup: Startup,
    pub sessions: Vec<Session>,
    /// Registered projects, as the backend lists them.
    pub projects: Vec<Project>,
    /// Durable lanes, including archived ones.
    pub lanes: Vec<Lane>,
    /// Backend-owned session membership for each lane.
    pub navigation: Vec<LaneNavigation>,
    pub conversations: HashMap<String, Conversation>,
    /// Whether each open session's writable tools have no approval guard.
    pub unguarded_tools: HashMap<String, bool>,
    /// Earlier lane sessions shown read-only until their next prompt.
    pub previews: HashSet<String>,
    /// A failed first open, shown in its pane instead of loading forever.
    pub open_errors: HashMap<String, String>,
    /// Each open session's status, as the backend reports it.
    pub statuses: HashMap<String, SessionStatus>,
    /// Keys with a prompt on its way to the harness.
    pub sending: HashSet<String>,
    /// Model, thinking level and context use, per open session.
    pub settings: HashMap<String, SessionSettings>,
    /// Models each open session can switch to, once asked for.
    pub models: HashMap<String, Vec<ModelInfo>>,
    pub preferences: Preferences,
    pub active_key: Option<String>,
    /// Problems listing sessions, one line per harness.
    pub list_errors: Vec<String>,
    pub error: Option<String>,
    /// What each session's messages were last decided by: a run starting or
    /// a history request. A history reply is only applied if nothing newer
    /// happened while it was on its way.
    epochs: HashMap<String, u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            startup: Startup::Loading,
            sessions: Vec::new(),
            projects: Vec::new(),
            lanes: Vec::new(),
            navigation: Vec::new(),
            conversations: HashMap::new(),
            unguarded_tools: HashMap::new(),
            previews: HashSet::new(),
            open_errors: HashMap::new(),
            statuses: HashMap::new(),
            sending: HashSet::new(),
            settings: HashMap::new(),
            models: HashMap::new(),
            preferences: Preferences {
                profiles: Vec::new(),
                favorite_models: Vec::new(),
                hidden_models: Vec::new(),
                cleanup_idle_days: 4,
                permission_warning_dismissed: false,
                notifications: NotificationPreferences::default(),
                dark_mode: None,
                font_size: panoptes_core::store::DEFAULT_FONT_SIZE,
            },
            active_key: None,
            list_errors: Vec::new(),
            error: None,
            epochs: HashMap::new(),
        }
    }
}

impl State {
    pub fn status(&self, key: &str) -> SessionStatus {
        self.statuses
            .get(key)
            .copied()
            .unwrap_or(SessionStatus::Idle)
    }

    pub fn session(&self, key: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.key == key)
    }

    /// How a session is named in a notification: its lane, and its title
    /// when it has one beyond the generic "New session".
    pub fn session_label(&self, key: &str) -> String {
        let Some(session) = self.session(key) else {
            return "A session".into();
        };
        let lane = session
            .lane_id
            .and_then(|id| self.lanes.iter().find(|lane| lane.id == id))
            .map(|lane| lane.name.clone());
        match (lane, session.title.as_str()) {
            (Some(lane), "New session") => lane,
            (Some(lane), title) => format!("{lane}: {title}"),
            (None, title) => title.to_string(),
        }
    }

    fn bump(&mut self, key: &str) -> u64 {
        let next = self.epochs.get(key).copied().unwrap_or(0) + 1;
        self.epochs.insert(key.to_string(), next);
        next
    }

    /// Whether a history fetched at `epoch` may still replace the messages.
    fn history_applies(&self, key: &str, epoch: u64) -> bool {
        // During a run the events build the transcript, and the fetched
        // history does not hold the message that is streaming.
        let streaming = self
            .conversations
            .get(key)
            .is_some_and(|c| c.busy && !c.messages.is_empty());
        self.epochs.get(key) == Some(&epoch) && !streaming
    }

    fn conversation(&mut self, key: &str) -> &mut Conversation {
        self.conversations
            .entry(key.to_string())
            .or_insert_with(|| Conversation::new(Vec::new()))
    }

    pub fn apply_listing(&mut self, listing: Listing) {
        self.sessions = listing.sessions;
        self.projects = listing.projects;
        self.lanes = listing.lanes;
        self.navigation = listing.navigation;
        self.list_errors = listing.errors;
    }

    /// Takes an opened session as the active one. `epoch` is the one its
    /// open was started at; `None` bumps a new one.
    pub fn adopt_opened(&mut self, opened: OpenedSession, epoch: Option<u64>) {
        let key = opened.key.clone();
        let epoch = epoch.unwrap_or_else(|| self.bump(&key));
        self.active_key = Some(key.clone());
        self.unguarded_tools
            .insert(key.clone(), opened.unguarded_tools);
        if opened.preview {
            self.previews.insert(key.clone());
        } else {
            self.previews.remove(&key);
        }
        let applies = self.history_applies(&key, epoch);
        let now = now_ms();
        let conversation = self.conversation(&key);
        // Keep anything the session already said (questions, notices) and
        // take the history as the authoritative message list.
        if applies {
            conversation.messages = opened.history;
        }
        conversation.exited = None;
        // A reloaded window catches up on the run and on what is still
        // waiting for an answer.
        for event in &opened.replay {
            conversation.apply(event, now);
        }
        // A status that arrived while opening is newer than this one.
        self.statuses.entry(key).or_insert(opened.status);
    }

    /// Drops everything kept about a session that is no longer open.
    pub fn forget(&mut self, key: &str) {
        self.conversations.remove(key);
        self.unguarded_tools.remove(key);
        self.previews.remove(key);
        self.open_errors.remove(key);
        self.statuses.remove(key);
        self.settings.remove(key);
        self.models.remove(key);
        self.epochs.remove(key);
        if self.active_key.as_deref() == Some(key) {
            self.active_key = None;
        }
    }

    pub fn on_event(&mut self, event: SessionEvent) -> Vec<Effect> {
        let key = event.key;
        let event = match event.event {
            SessionUpdate::Status(changed) => {
                self.statuses.insert(key, changed.status);
                return Vec::new();
            }
            // A session was first seen editing another lane: hold that lane back now.
            SessionUpdate::LaneWrites(_) => return vec![Effect::Refresh],
            SessionUpdate::Harness(event) => event,
        };
        if matches!(event, HarnessEvent::RunStarted) {
            self.bump(&key);
        }
        self.conversation(&key).apply(&event, now_ms());
        match event {
            // The history is authoritative; it keeps the ids the run streamed
            // under. The run also moved the context use.
            HarnessEvent::RunSettled { .. } => vec![
                Effect::ReloadHistory(key.clone()),
                Effect::LoadSettings(key),
                Effect::Refresh,
            ],
            HarnessEvent::Exited { .. } | HarnessEvent::TitleChanged { .. } => {
                vec![Effect::Refresh]
            }
            _ => Vec::new(),
        }
    }

    /// The prompt mode a message takes now.
    pub fn prompt_mode(&self, key: &str, queue: bool) -> PromptMode {
        let busy = self.conversations.get(key).is_some_and(|c| c.busy);
        match (busy, queue) {
            (false, _) => PromptMode::Start,
            (true, true) => PromptMode::FollowUp,
            (true, false) => PromptMode::Steer,
        }
    }

    /// The conversation waiting on `question_id`, if any.
    fn asker_of(&self, question_id: &str) -> Option<String> {
        self.conversations.iter().find_map(|(key, c)| {
            (c.questions.iter().any(|q| q.id == question_id)
                || c.approvals.iter().any(|a| a.id == question_id))
            .then(|| key.clone())
        })
    }
}

/// The session's process is gone; opening it again starts a new one.
fn needs_reopen(error: &ServiceError) -> bool {
    matches!(error.kind, ErrorKind::NotOpen | ErrorKind::Exited)
}

pub struct AppStore {
    backend: Arc<Backend>,
    pub state: State,
}

/// Opens a listed session by its harness locator.
type Opening = std::pin::Pin<Box<dyn Future<Output = Result<OpenedSession, ServiceError>> + Send>>;

fn open_call(session: &Session) -> impl FnOnce(Arc<Service>) -> Opening + use<> {
    let harness = session.harness.clone();
    let key = session.key.clone();
    let locator = session
        .harness_ref
        .as_ref()
        .map(|r| r.locator.clone())
        .unwrap_or_default();
    let cwd = session.cwd.clone();
    move |service| Box::pin(async move { service.open(&harness, &key, locator, cwd).await })
}

impl AppStore {
    pub fn new(backend: Arc<Backend>, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            backend,
            state: State::default(),
        };
        store.init(cx);
        store
    }

    pub fn backend(&self) -> &Arc<Backend> {
        &self.backend
    }

    fn fail(&mut self, error: ServiceError, cx: &mut Context<Self>) {
        self.state.error = Some(error.message);
        cx.notify();
    }

    /// Spawns `work`, then hands its answer to `then` on the UI thread.
    fn spawn<T: 'static, R: 'static>(
        &self,
        cx: &mut Context<Self>,
        work: impl Future<Output = T> + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) -> R + 'static,
    ) -> Task<Option<R>> {
        cx.spawn(async move |this, cx| {
            let answer = work.await;
            this.update(cx, |this, cx| {
                let result = then(this, answer, cx);
                cx.notify();
                result
            })
            .ok()
        })
    }

    pub fn init(&mut self, cx: &mut Context<Self>) {
        self.state.startup = Startup::Loading;
        let listing = self.backend.call(|s| async move { s.list().await });
        let preferences = self
            .backend
            .run("preferences", |s| async move { s.preferences().await });
        self.spawn(
            cx,
            async move { (listing.await, preferences.await) },
            |this, (listing, preferences), cx| {
                match listing {
                    Ok(listing) => {
                        this.state.apply_listing(listing);
                        this.state.startup = Startup::Ready;
                    }
                    Err(error) => this.state.startup = Startup::Error(format!("{error:#}")),
                }
                match preferences {
                    Ok(preferences) => this.state.preferences = preferences,
                    Err(error) => this.fail(error, cx),
                }
            },
        )
        .detach();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Task<Option<()>> {
        let listing = self.backend.call(|s| async move { s.list().await });
        self.spawn(cx, listing, |this, listing, cx| match listing {
            Ok(listing) => this.state.apply_listing(listing),
            Err(error) => this.fail(ServiceError::other(format!("{error:#}")), cx),
        })
    }

    fn run_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Refresh => self.refresh(cx).detach(),
                Effect::ReloadHistory(key) => self.reload_history(key, cx),
                Effect::LoadSettings(key) => self.load_settings(key, cx),
            }
        }
    }

    pub fn on_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        let key = event.key.clone();
        let status = match &event.event {
            SessionUpdate::Status(changed) => Some(changed.status),
            _ => None,
        };
        let previous = self.state.statuses.get(&key).copied();
        let effects = self.state.on_event(event);
        self.run_effects(effects, cx);
        if let Some(status) = status {
            self.notify_session(&key, previous, status, cx);
        }
        cx.notify();
    }

    /// Tells the user about a session, when their notification settings ask
    /// for it and they are not already looking at that session.
    fn notify_session(
        &self,
        key: &str,
        previous: Option<SessionStatus>,
        status: SessionStatus,
        cx: &mut Context<Self>,
    ) {
        // A backgrounded app's main window is not necessarily key/active.
        let window_active = cx
            .active_window()
            .map(|window| {
                window
                    .update(cx, |_, window, _| window.is_window_active())
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let attended = notify::attended(self.state.active_key.as_deref(), key, window_active);
        let label = self.state.session_label(key);
        if let Some(notice) = notify::notice(
            previous,
            status,
            &self.state.preferences.notifications,
            attended,
            &label,
        ) {
            crate::notify::post(key, &notice, cx);
        }
    }

    fn load_settings(&mut self, key: String, cx: &mut Context<Self>) {
        let for_key = key.clone();
        let settings = self
            .backend
            .call(move |s| async move { s.settings(&for_key).await });
        self.spawn(cx, settings, move |this, settings, _| {
            // Settings are a nicety; the composer works without them.
            if let Ok(Ok(settings)) = settings {
                this.state.settings.insert(key, settings);
            }
        })
        .detach();
    }

    fn reload_history(&mut self, key: String, cx: &mut Context<Self>) {
        let epoch = self.state.bump(&key);
        let for_key = key.clone();
        let history = self.backend.run("session_history", move |s| async move {
            s.history(&for_key).await
        });
        self.spawn(cx, history, move |this, history, cx| match history {
            Ok(messages) => {
                if this.state.conversations.contains_key(&key)
                    && this.state.history_applies(&key, epoch)
                {
                    this.state.conversation(&key).messages = messages;
                }
            }
            Err(error) => this.fail(error, cx),
        })
        .detach();
    }

    fn opened(&mut self, opened: OpenedSession, epoch: Option<u64>, cx: &mut Context<Self>) {
        let key = opened.key.clone();
        self.state.adopt_opened(opened, epoch);
        self.load_settings(key, cx);
    }

    pub fn open(&mut self, session: &Session, cx: &mut Context<Self>) -> Task<Option<()>> {
        let opened = self.backend.run("open_session", open_call(session));
        self.open_with(session, opened, cx)
    }

    /// Shows earlier history without replacing the lane's live session.
    pub fn preview(&mut self, session: &Session, cx: &mut Context<Self>) -> Task<Option<()>> {
        let key = session.key.clone();
        let opened = self.backend.run(
            "preview_session",
            move |s| async move { s.preview(&key).await },
        );
        self.open_with(session, opened, cx)
    }

    fn open_with(
        &mut self,
        session: &Session,
        opened: impl Future<Output = Result<OpenedSession, ServiceError>> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<()>> {
        let key = session.key.clone();
        self.state.open_errors.remove(&key);
        self.state.active_key = Some(key.clone());
        let epoch = self.state.bump(&key);
        cx.notify();
        self.spawn(cx, opened, move |this, opened, cx| match opened {
            Ok(opened) => {
                // An adopted conversation answers with its lane key, which
                // has its own history of events.
                let epoch = (opened.key == key).then_some(epoch);
                this.opened(opened, epoch, cx);
                this.refresh(cx).detach();
            }
            Err(error) => {
                // A reopened session keeps its transcript. A first open needs
                // its own error state, or the pane would show "Starting" forever.
                if !this.state.conversations.contains_key(&key)
                    && this.state.epochs.get(&key) == Some(&epoch)
                {
                    this.state.open_errors.insert(key, error.message.clone());
                }
                this.fail(error, cx);
            }
        })
    }

    /// Starts a new session in a lane's worktree, configured and prompted.
    pub fn create(
        &mut self,
        lane_id: i64,
        options: SessionOptions,
        cx: &mut Context<Self>,
    ) -> Task<Option<()>> {
        let instructions = options.instructions.clone();
        let opened = self.backend.run("new_session", move |s| async move {
            s.create_with(lane_id, "pi", instructions).await
        });
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let opened = opened.await;
            let key = this
                .update(cx, |this, cx| match opened {
                    Ok(opened) => {
                        let key = opened.key.clone();
                        this.opened(opened, None, cx);
                        this.refresh(cx).detach();
                        cx.notify();
                        Some(key)
                    }
                    Err(error) => {
                        this.fail(error, cx);
                        None
                    }
                })
                .ok()
                .flatten()?;
            // Everything the form chose is applied to the new session by its
            // own key, so it survives the user opening another session meanwhile.
            if let Some(model) = options.model {
                let k = key.clone();
                let settings = backend
                    .run(
                        "set_model",
                        move |s| async move { s.set_model(&k, model).await },
                    )
                    .await;
                this.update(cx, |this, cx| this.keep_settings(&key, settings, cx))
                    .ok()?;
            }
            if let Some(level) = options.thinking {
                let k = key.clone();
                let settings = backend
                    .run("set_thinking_level", move |s| async move {
                        s.set_thinking_level(&k, level).await
                    })
                    .await;
                this.update(cx, |this, cx| this.keep_settings(&key, settings, cx))
                    .ok()?;
            }
            if let Some(prompt) = options.prompt {
                let k = key.clone();
                let images = options.images;
                let outcome = backend
                    .run("send_prompt", move |s| async move {
                        s.prompt(&k, prompt, images, PromptMode::Start).await
                    })
                    .await;
                this.update(cx, |this, cx| match outcome {
                    Ok(outcome) => this.prompted(&key, outcome, cx),
                    Err(error) => this.fail(error, cx),
                })
                .ok()?;
            }
            Some(())
        })
    }

    fn keep_settings(
        &mut self,
        key: &str,
        settings: Result<SessionSettings, ServiceError>,
        cx: &mut Context<Self>,
    ) {
        match settings {
            Ok(settings) => {
                self.state.settings.insert(key.to_string(), settings);
                cx.notify();
            }
            Err(error) => self.fail(error, cx),
        }
    }

    fn prompted(&mut self, key: &str, outcome: PromptOutcome, cx: &mut Context<Self>) {
        match outcome {
            PromptOutcome::Started => self.state.conversation(key).busy = true,
            PromptOutcome::Handled => self.reload_history(key.to_string(), cx),
            PromptOutcome::Queued => {}
        }
        cx.notify();
    }

    fn replaced_lane_session(
        &mut self,
        lane_id: i64,
        opened: impl Future<Output = Result<OpenedSession, ServiceError>> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<bool>> {
        let previous = self
            .state
            .sessions
            .iter()
            .find(|s| s.lane_id == Some(lane_id) && s.open)
            .map(|s| s.key.clone());
        self.spawn(cx, opened, move |this, opened, cx| match opened {
            Ok(opened) => {
                if let Some(previous) = previous
                    && previous != opened.key
                {
                    this.state.forget(&previous);
                }
                this.opened(opened, None, cx);
                this.refresh(cx).detach();
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    /// Replaces the lane's live process with a fresh conversation record.
    pub fn new_lane_session(&mut self, lane_id: i64, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let opened = self
            .backend
            .run("replace_lane_session", move |s| async move {
                s.replace_lane_session(lane_id).await
            });
        self.replaced_lane_session(lane_id, opened, cx)
    }

    /// Reopens one of the lane's own earlier conversation records.
    pub fn resume_lane_session(
        &mut self,
        lane_id: i64,
        session_id: i64,
        cx: &mut Context<Self>,
    ) -> Task<Option<bool>> {
        let opened = self
            .backend
            .run("resume_lane_session", move |s| async move {
                s.resume_lane_session(lane_id, session_id).await
            });
        self.replaced_lane_session(lane_id, opened, cx)
    }

    /// Adopts a discovered conversation into a matching or chosen lane.
    pub fn adopt(
        &mut self,
        session: &Session,
        lane_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Option<AdoptionOutcome>>> {
        let harness = session.harness.clone();
        let key = session.key.clone();
        let locator = session
            .harness_ref
            .as_ref()
            .map(|r| r.locator.clone())
            .unwrap_or_default();
        let cwd = session.cwd.clone();
        let outcome = self.backend.run("adopt_session", move |s| async move {
            s.adopt(&harness, &key, locator, cwd, lane_id).await
        });
        self.spawn(cx, outcome, |this, outcome, cx| match outcome {
            Ok(outcome) => {
                if matches!(outcome, AdoptionOutcome::Adopted { .. }) {
                    this.refresh(cx).detach();
                }
                Some(outcome)
            }
            Err(error) => {
                this.fail(error, cx);
                None
            }
        })
    }

    /// Hides a lane session (closing it first) or brings it back.
    pub fn set_archived(&mut self, key: String, archived: bool, cx: &mut Context<Self>) {
        let k = key.clone();
        let done = self.backend.run("archive_session", move |s| async move {
            s.set_archived(&k, archived).await
        });
        self.spawn(cx, done, move |this, done, cx| match done {
            Ok(()) => {
                if archived {
                    this.state.forget(&key);
                }
                this.refresh(cx).detach();
            }
            Err(error) => this.fail(error, cx),
        })
        .detach();
    }

    /// Renames a lane session; an empty title falls back to the harness's.
    pub fn rename(&mut self, key: String, title: &str, cx: &mut Context<Self>) {
        let title = Some(title.trim().to_string()).filter(|t| !t.is_empty());
        let done = self.backend.run("rename_session", move |s| async move {
            s.rename(&key, title).await
        });
        self.spawn(cx, done, |this, done, cx| match done {
            Ok(()) => this.refresh(cx).detach(),
            Err(error) => this.fail(error, cx),
        })
        .detach();
    }

    /// Resolves to whether the harness took the prompt, so a caller can keep
    /// its draft.
    pub fn send(
        &mut self,
        key: String,
        text: String,
        images: Vec<ImageAttachment>,
        queue: bool,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.state.sending.contains(&key) {
            return Task::ready(false);
        }
        self.state.sending.insert(key.clone());
        cx.notify();
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let prompt = |mode: PromptMode| {
                let (key, text, images) = (key.clone(), text.clone(), images.clone());
                backend.run("send_prompt", move |s| async move {
                    s.prompt(&key, text, images, mode).await
                })
            };
            let preview = this
                .update(cx, |this, _| this.state.previews.contains(&key))
                .unwrap_or(false);
            let resumed = if preview {
                let session = this
                    .update(cx, |this, _| this.state.session(&key).cloned())
                    .ok()
                    .flatten();
                match session.and_then(|s| s.lane_id.zip(s.id)) {
                    None => Err(ServiceError::other(
                        "This preview is no longer available in its lane.",
                    )),
                    Some((lane_id, session_id)) => match backend
                        .run("resume_lane_session", move |s| async move {
                            s.resume_lane_session(lane_id, session_id).await
                        })
                        .await
                    {
                        Err(error) => Err(error),
                        Ok(opened) => {
                            let Ok(()) = this.update(cx, |this, cx| {
                                let previous = this
                                    .state
                                    .sessions
                                    .iter()
                                    .find(|s| s.lane_id == Some(lane_id) && s.open && s.key != key)
                                    .map(|s| s.key.clone());
                                if let Some(previous) = previous {
                                    this.state.forget(&previous);
                                }
                                this.opened(opened, None, cx);
                                this.refresh(cx).detach();
                            }) else {
                                return false;
                            };
                            Ok(())
                        }
                    },
                }
            } else {
                Ok(())
            };
            let mut outcome = match resumed {
                Err(error) => Err(error),
                Ok(()) => {
                    let Ok(mode) = this.update(cx, |this, _| this.state.prompt_mode(&key, queue))
                    else {
                        return false;
                    };
                    prompt(mode).await
                }
            };
            // Retry once only when nothing was sent. If the process exited
            // after receiving the prompt, reopening and resending could
            // duplicate it.
            if let Err(error) = &outcome
                && error.kind == ErrorKind::NotOpen
            {
                let session = this
                    .update(cx, |this, _| this.state.session(&key).cloned())
                    .ok()
                    .flatten();
                outcome = match session {
                    None => Err(ServiceError::other("This session is no longer listed.")),
                    Some(session) => match backend.run("open_session", open_call(&session)).await {
                        Err(error) => Err(error),
                        Ok(opened) => {
                            let Ok(mode) = this.update(cx, |this, cx| {
                                this.opened(opened, None, cx);
                                this.state.prompt_mode(&key, queue)
                            }) else {
                                return false;
                            };
                            prompt(mode).await
                        }
                    },
                };
            }
            this.update(cx, |this, cx| {
                this.state.sending.remove(&key);
                let sent = match outcome {
                    Ok(outcome) => {
                        this.prompted(&key, outcome, cx);
                        true
                    }
                    // The prompt may have arrived before the process ended, so
                    // the person checks the conversation rather than resending.
                    Err(error) if error.kind == ErrorKind::Exited => {
                        this.state.error = Some(format!(
                            "{}. Check whether your prompt arrived before sending it again.",
                            error.message
                        ));
                        false
                    }
                    Err(error) => {
                        this.fail(error, cx);
                        false
                    }
                };
                cx.notify();
                sent
            })
            .unwrap_or(false)
        })
    }

    /// The harness commands a session offers, for the composer's `/` list.
    pub fn list_commands(
        &self,
        key: String,
    ) -> impl Future<Output = Option<Vec<CommandInfo>>> + use<> {
        let commands = self.backend.run(
            "list_commands",
            move |s| async move { s.commands(&key).await },
        );
        async move { commands.await.ok() }
    }

    /// The workspace's files, for the composer's `@` list.
    pub fn list_files(&self, cwd: String) -> impl Future<Output = Vec<String>> + use<> {
        let files = self.backend.call(move |_| async move {
            tokio::task::spawn_blocking(move || {
                panoptes_core::files::list(
                    std::path::Path::new(&cwd),
                    panoptes_core::files::MAX_FILES,
                )
            })
            .await
            .unwrap_or_default()
        });
        async move { files.await.unwrap_or_default() }
    }

    /// Reads files picked or dropped on the composer; a failure shows in the
    /// error banner.
    pub fn read_attachments(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Vec<Attachment>>> {
        let read = self.backend.call(move |_| async move {
            tokio::task::spawn_blocking(move || {
                paths
                    .iter()
                    .map(|path| panoptes_core::files::attachment(path))
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|read| read)
        });
        self.spawn(
            cx,
            async move { read.await.and_then(|read| read) },
            |this, read, cx| match read {
                Ok(read) => read,
                Err(error) => {
                    this.fail(ServiceError::other(format!("{error:#}")), cx);
                    Vec::new()
                }
            },
        )
    }

    /// Shows a failure in the window's error banner.
    pub fn show_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.state.error = Some(message.into());
        cx.notify();
    }

    pub fn load_models(&mut self, key: String, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let k = key.clone();
        let models = self
            .backend
            .run("list_models", move |s| async move { s.models(&k).await });
        self.spawn(cx, models, move |this, models, cx| match models {
            Ok(models) => {
                this.state.models.insert(key, models);
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    fn set_preferences(
        &mut self,
        work: impl Future<Output = Result<Preferences, ServiceError>> + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Option<bool>> {
        self.spawn(cx, work, |this, preferences, cx| match preferences {
            Ok(preferences) => {
                this.state.preferences = preferences;
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    pub fn dismiss_permission_warning(&mut self, cx: &mut Context<Self>) {
        let work = self
            .backend
            .run("dismiss_permission_warning", |s| async move {
                s.dismiss_permission_warning().await
            });
        self.set_preferences(work, cx).detach();
    }

    pub fn save_profile(
        &mut self,
        id: Option<i64>,
        input: ProfileInput,
        cx: &mut Context<Self>,
    ) -> Task<Option<bool>> {
        let work = self.backend.run("save_profile", move |s| async move {
            s.save_profile(id, input).await
        });
        self.set_preferences(work, cx)
    }

    pub fn delete_profile(&mut self, id: i64, cx: &mut Context<Self>) {
        let work = self.backend.run("delete_profile", move |s| async move {
            s.delete_profile(id).await
        });
        self.set_preferences(work, cx).detach();
    }

    pub fn set_model_favorite(&mut self, model_id: String, favorite: bool, cx: &mut Context<Self>) {
        let work = self.backend.run("set_model_favorite", move |s| async move {
            s.set_model_favorite(model_id, favorite).await
        });
        self.set_preferences(work, cx).detach();
    }

    pub fn set_model_hidden(&mut self, model_id: String, hidden: bool, cx: &mut Context<Self>) {
        let work = self.backend.run("set_model_hidden", move |s| async move {
            s.set_model_hidden(model_id, hidden).await
        });
        self.set_preferences(work, cx).detach();
    }

    pub fn set_notification(
        &mut self,
        field: NotificationField,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        // Reflect the click at once. Each field has its own SQL update, so a
        // quick click on another switch cannot restore this one's old value.
        self.state.preferences.notifications.set(field, value);
        cx.notify();
        let work = self.backend.run("set_notification", move |s| async move {
            s.set_notification(field, value).await
        });
        self.spawn(cx, work, |this, result, cx| match result {
            Ok(()) => true,
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
        .detach();
    }

    pub fn set_cleanup_idle_days(
        &mut self,
        days: u32,
        cx: &mut Context<Self>,
    ) -> Task<Option<bool>> {
        let work = self
            .backend
            .run("set_cleanup_idle_days", move |s| async move {
                s.set_cleanup_idle_days(days).await
            });
        self.set_preferences(work, cx)
    }

    pub fn set_dark_mode(&mut self, dark: bool, cx: &mut Context<Self>) {
        let work = self.backend.run("set_dark_mode", move |s| async move {
            s.set_dark_mode(dark).await
        });
        self.set_preferences(work, cx).detach();
    }

    pub fn set_font_size(&mut self, percent: u32, cx: &mut Context<Self>) {
        let work = self.backend.run("set_font_size", move |s| async move {
            s.set_font_size(percent).await
        });
        self.set_preferences(work, cx).detach();
    }

    pub fn set_model(&mut self, key: String, model_id: String, cx: &mut Context<Self>) {
        let k = key.clone();
        let work = self.backend.run("set_model", move |s| async move {
            s.set_model(&k, model_id).await
        });
        self.spawn(cx, work, move |this, settings, cx| {
            this.keep_settings(&key, settings, cx)
        })
        .detach();
    }

    pub fn set_thinking_level(&mut self, key: String, level: String, cx: &mut Context<Self>) {
        let k = key.clone();
        let work = self.backend.run("set_thinking_level", move |s| async move {
            s.set_thinking_level(&k, level).await
        });
        self.spawn(cx, work, move |this, settings, cx| {
            this.keep_settings(&key, settings, cx)
        })
        .detach();
    }

    pub fn abort(&mut self, key: String, cx: &mut Context<Self>) {
        let work = self
            .backend
            .run("abort", move |s| async move { s.abort(&key).await });
        self.spawn(cx, work, |this, done, cx| {
            if let Err(error) = done {
                this.fail(error, cx);
            }
        })
        .detach();
    }

    /// Answers a question or approval. The card goes at once, and comes back
    /// if the harness did not take the answer.
    pub fn answer(
        &mut self,
        question_id: String,
        answer: Answer,
        key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = key
            .or_else(|| self.state.asker_of(&question_id))
            .or_else(|| self.state.active_key.clone())
        else {
            return;
        };
        let before = self.state.conversations.get(&key);
        let question =
            before.and_then(|c| c.questions.iter().find(|q| q.id == question_id).cloned());
        let approval =
            before.and_then(|c| c.approvals.iter().find(|a| a.id == question_id).cloned());
        self.state.conversation(&key).remove_question(&question_id);
        cx.notify();
        let (k, id) = (key.clone(), question_id.clone());
        let work = self.backend.run(
            "answer",
            move |s| async move { s.answer(&k, id, answer).await },
        );
        self.spawn(cx, work, move |this, done, cx| {
            let Err(error) = done else { return };
            let reopen = needs_reopen(&error);
            this.fail(error, cx);
            // The harness is still waiting, so the card comes back. A process
            // that has gone asks nothing any more.
            let Some(c) = this.state.conversations.get_mut(&key).filter(|_| !reopen) else {
                return;
            };
            if let Some(q) = question
                && !c.questions.iter().any(|q| q.id == question_id)
            {
                c.questions.push(q);
            }
            if let Some(a) = approval
                && !c.approvals.iter().any(|a| a.id == question_id)
            {
                c.approvals.push(a);
            }
        })
        .detach();
    }

    pub fn close(&mut self, key: String, cx: &mut Context<Self>) {
        let k = key.clone();
        let work = self.backend.call(move |s| async move { s.close(&k).await });
        self.spawn(cx, work, move |this, _, cx| {
            this.state.forget(&key);
            this.refresh(cx).detach();
        })
        .detach();
    }

    /// Asks for a folder and registers it as a project.
    pub fn pick_project(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.first() else { return };
            let folder = folder.to_string_lossy().to_string();
            let _ = this.update(cx, |store, cx| store.add_project(folder, cx));
        })
        .detach();
    }

    /// Registers the git repository `folder` is in.
    pub fn add_project(&mut self, folder: String, cx: &mut Context<Self>) {
        let work = self.backend.run("add_project", move |s| async move {
            s.add_project(folder).await
        });
        self.spawn(cx, work, |this, done, cx| match done {
            Ok(_) => this.refresh(cx).detach(),
            Err(error) => this.fail(error, cx),
        })
        .detach();
    }

    pub fn remove_project(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let work = self.backend.run("remove_project", move |s| async move {
            s.remove_project(id).await
        });
        self.spawn(cx, work, |this, done, cx| match done {
            Ok(()) => {
                this.refresh(cx).detach();
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    pub fn create_lane(
        &mut self,
        project_id: i64,
        task: String,
        main_checkout: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<Option<Lane>>> {
        let work = self.backend.run("create_lane", move |s| async move {
            s.create_lane(project_id, task, main_checkout).await
        });
        self.spawn(cx, work, |this, lane, cx| match lane {
            Ok(lane) => {
                this.refresh(cx).detach();
                Some(lane)
            }
            Err(error) => {
                if error.kind != ErrorKind::Cancelled {
                    this.fail(error, cx);
                }
                None
            }
        })
    }

    /// Creates a lane named after `task` and starts its first session with
    /// `options.prompt`, or the task itself when there is none.
    pub fn create_from_prompt(
        &mut self,
        project_id: i64,
        task: String,
        main_checkout: bool,
        options: SessionOptions,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let lane = self.create_lane(project_id, task.clone(), main_checkout, cx);
        cx.spawn(async move |this, cx| {
            let Some(Some(lane)) = lane.await else {
                return false;
            };
            let options = SessionOptions {
                prompt: options.prompt.or(Some(task)),
                ..options
            };
            let Ok(created) = this.update(cx, |this, cx| this.create(lane.id, options, cx)) else {
                return false;
            };
            created.await;
            true
        })
    }

    pub fn cancel_lane_creation(&mut self, project_id: i64) {
        // The call runs on the backend whether or not its answer is awaited.
        drop(
            self.backend
                .call(move |s| async move { s.cancel_lane_creation(project_id) }),
        );
    }

    pub fn archive_lane(
        &mut self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
        recovery: BranchRecovery,
        cx: &mut Context<Self>,
    ) -> Task<Option<Option<ArchiveLaneOutcome>>> {
        let work = self.closing_lane_sessions(id, "archive_lane", move |s| async move {
            s.archive_lane_recovering(id, strategy, recovery).await
        });
        self.spawn(cx, work, |this, (closed, outcome), cx| {
            this.forget_closed(&closed, cx);
            match outcome {
                Ok(outcome) => {
                    if matches!(outcome, ArchiveLaneOutcome::Archived) {
                        this.refresh(cx).detach();
                    }
                    Some(outcome)
                }
                Err(error) => {
                    this.fail(error, cx);
                    None
                }
            }
        })
    }

    /// Closes lane `id`'s idle sessions, then runs `then`. Archiving or
    /// deleting a lane implies closing them; the service refuses a session
    /// that is starting, Running or Waiting. Answers with the closed keys,
    /// which the caller forgets whether or not `then` worked.
    fn closing_lane_sessions<F, Fut, T>(
        &self,
        id: i64,
        command: &'static str,
        then: F,
    ) -> impl Future<Output = (Vec<String>, Result<T, ServiceError>)> + use<F, Fut, T>
    where
        F: FnOnce(Arc<Service>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>> + Send + 'static,
        T: Send + 'static,
    {
        let backend = self.backend.clone();
        async move {
            let closed = backend
                .run(
                    command,
                    move |s| async move { s.close_lane_sessions(id).await },
                )
                .await;
            match closed {
                Ok(closed) => (closed, backend.run(command, then).await),
                Err(error) => (Vec::new(), Err(error)),
            }
        }
    }

    fn forget_closed(&mut self, closed: &[String], cx: &mut Context<Self>) {
        for key in closed {
            self.state.forget(key);
        }
        if !closed.is_empty() {
            self.refresh(cx).detach();
        }
    }

    /// Worktree usage and safe idle lanes for one project, measured now.
    pub fn lane_cleanup(
        &self,
        project_id: i64,
    ) -> impl Future<Output = Result<LaneCleanup, String>> + use<> {
        let work = self.backend.run("lane_cleanup", move |s| async move {
            s.lane_cleanup(project_id).await
        });
        async move { work.await.map_err(|e| e.message) }
    }

    /// Removes stale Git administrative records, never a live worktree. A
    /// repository that could not be pruned is reported without hiding the
    /// ones that were.
    pub fn prune_worktrees(&mut self, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let work = self.backend.run(
            "prune_worktrees",
            |s| async move { s.prune_worktrees().await },
        );
        self.spawn(cx, work, |this, done, cx| match done {
            Ok(pruned) => {
                if !pruned.errors.is_empty() {
                    this.show_error(pruned.errors.join(" \u{b7} "), cx);
                }
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    /// Shows a worktree in Finder. `open -R` selects it there, and waiting on
    /// the child in a blocking task keeps it from being left a zombie.
    pub fn reveal_in_finder(&mut self, path: String, cx: &mut Context<Self>) -> Task<Option<()>> {
        let opened = self.backend.call(move |_| async move {
            tokio::task::spawn_blocking(move || {
                std::process::Command::new("open")
                    .arg("-R")
                    .arg(&path)
                    .status()
            })
            .await
            .map_err(anyhow::Error::from)
            .and_then(|status| status.map_err(anyhow::Error::from))
        });
        self.spawn(
            cx,
            async move { opened.await.and_then(|opened| opened) },
            |this, opened, cx| match opened {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    this.show_error(format!("Finder could not show the worktree ({status})"), cx)
                }
                Err(error) => {
                    this.show_error(format!("Could not reveal the worktree: {error:#}"), cx)
                }
            },
        )
    }

    /// Archives independently, preserving any dirty lane as a checkpoint.
    pub fn archive_lanes(
        &mut self,
        ids: Vec<i64>,
        cx: &mut Context<Self>,
    ) -> Task<ArchiveLanesResult> {
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let mut result = ArchiveLanesResult::default();
            // Different lanes can share a repository. Archive one at a time
            // rather than racing their Git housekeeping and checkpoint writes.
            for lane_id in ids {
                let outcome = backend
                    .run("archive_lane", move |s| async move {
                        s.archive_lane_recovering(
                            lane_id,
                            Some(ArchiveStrategy::Checkpoint),
                            BranchRecovery::None,
                        )
                        .await
                    })
                    .await;
                match outcome {
                    Ok(ArchiveLaneOutcome::Archived) => result.archived += 1,
                    Ok(ArchiveLaneOutcome::DetachedHead { .. }) => result.failed.push((
                        lane_id,
                        "Needs branch recovery; archive it from its lane menu.".into(),
                    )),
                    Ok(ArchiveLaneOutcome::OtherBranch { .. }) => result.failed.push((
                        lane_id,
                        "Is on another branch; archive it from its lane menu.".into(),
                    )),
                    Ok(ArchiveLaneOutcome::Dirty { .. }) => result.failed.push((
                        lane_id,
                        "Its worktree changed. Scan again before archiving it.".into(),
                    )),
                    Err(error) => result.failed.push((lane_id, error.message)),
                }
            }
            if let Ok(refreshed) = this.update(cx, |this, cx| this.refresh(cx)) {
                refreshed.await;
            }
            result
        })
    }

    /// Saves a detached HEAD on the lane branch, leaving the worktree in place.
    pub fn create_lane_branch(&mut self, id: i64, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let work = self.backend.run("create_lane_branch", move |s| async move {
            s.create_lane_branch(id).await
        });
        self.spawn(cx, work, |this, done, cx| match done {
            Ok(()) => {
                this.refresh(cx).detach();
                true
            }
            Err(error) => {
                this.fail(error, cx);
                false
            }
        })
    }

    pub fn restore_lane(&mut self, id: i64, cx: &mut Context<Self>) {
        let work = self.backend.run(
            "restore_lane",
            move |s| async move { s.restore_lane(id).await },
        );
        self.spawn(cx, work, |this, done, cx| match done {
            Ok(_) => this.refresh(cx).detach(),
            Err(error) => this.fail(error, cx),
        })
        .detach();
    }

    /// Permanently deletes a lane after explicit confirmation in the UI.
    pub fn delete_lane(
        &mut self,
        id: i64,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) -> Task<Option<Option<DeleteLaneOutcome>>> {
        let work = self.closing_lane_sessions(id, "delete_lane", move |s| async move {
            s.delete_lane_confirmed(id, confirmed).await
        });
        self.spawn(cx, work, move |this, (closed, outcome), cx| {
            this.forget_closed(&closed, cx);
            match outcome {
                Ok(outcome) => {
                    if matches!(outcome, DeleteLaneOutcome::Deleted) {
                        let keys: Vec<String> = this
                            .state
                            .sessions
                            .iter()
                            .filter(|s| s.lane_id == Some(id))
                            .map(|s| s.key.clone())
                            .collect();
                        for key in keys {
                            this.state.forget(&key);
                        }
                        this.refresh(cx).detach();
                    }
                    Some(outcome)
                }
                Err(error) => {
                    this.fail(error, cx);
                    None
                }
            }
        })
    }

    /// Opens the event log folder in the file manager.
    pub fn show_logs(&mut self, cx: &mut Context<Self>) {
        let dir = self.backend.logs_dir.clone();
        cx.open_with_system(&dir);
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.state.error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sidebar::tests::session;
    use panoptes_core::service::StatusChanged;
    use panoptes_core::store::LaneState;
    use panoptes_harness::{Message, Role};

    fn opened(key: &str, history: Vec<Message>) -> OpenedSession {
        OpenedSession {
            key: key.into(),
            history,
            unguarded_tools: false,
            preview: false,
            status: SessionStatus::Done,
            replay: Vec::new(),
        }
    }

    fn message(id: &str) -> Message {
        Message {
            id: id.into(),
            role: Role::User,
            blocks: Vec::new(),
            timestamp: None,
            error: None,
            stopped: false,
            usage: None,
        }
    }

    fn event(key: &str, event: HarnessEvent) -> SessionEvent {
        SessionEvent {
            key: key.into(),
            event: SessionUpdate::Harness(event),
        }
    }

    #[::core::prelude::v1::test]
    fn a_status_that_arrived_while_opening_wins() {
        let mut state = State::default();
        state.on_event(SessionEvent {
            key: "k".into(),
            event: SessionUpdate::Status(StatusChanged {
                status: SessionStatus::Running,
            }),
        });
        state.adopt_opened(opened("k", vec![message("a")]), None);
        assert_eq!(state.status("k"), SessionStatus::Running);
        assert_eq!(state.active_key.as_deref(), Some("k"));
        assert_eq!(state.conversations["k"].messages.len(), 1);
    }

    #[::core::prelude::v1::test]
    fn a_run_that_started_meanwhile_keeps_the_streamed_messages() {
        let mut state = State::default();
        let epoch = state.bump("k");
        state.on_event(event("k", HarnessEvent::RunStarted));
        state.adopt_opened(opened("k", vec![message("old")]), Some(epoch));
        assert!(state.conversations["k"].messages.is_empty());
    }

    #[::core::prelude::v1::test]
    fn a_settled_run_reloads_history_settings_and_the_listing() {
        let mut state = State::default();
        let effects = state.on_event(event("k", HarnessEvent::RunSettled { aborted: false }));
        assert_eq!(
            effects,
            [
                Effect::ReloadHistory("k".into()),
                Effect::LoadSettings("k".into()),
                Effect::Refresh
            ]
        );
    }

    #[::core::prelude::v1::test]
    fn forgetting_a_session_clears_what_was_kept() {
        let mut state = State::default();
        state.adopt_opened(opened("k", Vec::new()), None);
        state.forget("k");
        assert!(state.active_key.is_none());
        assert!(state.conversations.is_empty());
    }

    #[::core::prelude::v1::test]
    fn a_preview_is_marked_until_the_session_is_resumed() {
        let mut state = State::default();
        let mut preview = opened("k", Vec::new());
        preview.preview = true;
        state.adopt_opened(preview, None);
        assert!(state.previews.contains("k"));
        state.adopt_opened(opened("k", Vec::new()), None);
        assert!(!state.previews.contains("k"));
    }

    #[::core::prelude::v1::test]
    fn notification_labels_name_lanes_titles_and_unknown_sessions() {
        let mut state = State::default();
        state.lanes.push(Lane {
            id: 7,
            project_id: 1,
            name: "Fix login".into(),
            task: "Fix login".into(),
            branch: "fix-login".into(),
            state: LaneState::Active,
            worktrees: Vec::new(),
        });
        let mut new = session("new");
        new.lane_id = Some(7);
        new.title = "New session".into();
        let mut titled = session("titled");
        titled.lane_id = Some(7);
        titled.title = "Retry the request".into();
        let mut loose = session("loose");
        loose.title = "A conversation".into();
        state.sessions = vec![new, titled, loose];

        assert_eq!(state.session_label("new"), "Fix login");
        assert_eq!(
            state.session_label("titled"),
            "Fix login: Retry the request"
        );
        assert_eq!(state.session_label("loose"), "A conversation");
        assert_eq!(state.session_label("missing"), "A session");
    }

    #[::core::prelude::v1::test]
    fn a_busy_session_steers_unless_queued() {
        let mut state = State::default();
        assert_eq!(state.prompt_mode("k", true), PromptMode::Start);
        state.conversation("k").busy = true;
        assert_eq!(state.prompt_mode("k", false), PromptMode::Steer);
        assert_eq!(state.prompt_mode("k", true), PromptMode::FollowUp);
        let _ = session("x");
    }
}
