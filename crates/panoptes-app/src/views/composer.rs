//! The message field under a session: @-mentions, slash commands, list
//! continuation, attachments, the model and thinking pickers and the session's
//! usage. The text logic is `model::composer`.

use std::collections::HashSet;

use base64::Engine as _;
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InlineToken, InputContent, InputEvent, MoveDown, MoveUp,
    OutdentInline, Textarea, TextareaState,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::files::Attachment;
use panoptes_harness::{CommandInfo, PromptMode};

use crate::ds::{
    Button, CONTROL, CONTROL_SM, RADIUS_MD, RADIUS_SM, RADIUS_XS, Tooltip, Type as _, icon,
    option_toggle, panel,
};
use crate::model::composer::{
    LaneRequest, Query, available_commands, command_at, compose_prompt, context_label,
    continue_list, insert_command, insert_mention, lane_request, match_commands, match_files,
    mention_at, mention_label, next_level, restore_unsent, segments,
};
use crate::model::picker::{chip_model_groups, session_thinking_groups, short_model_id};
use crate::model::usage::{session_usage_detail, session_usage_label, sum_usage};
use crate::store::{AppStore, Draft};
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::{Overlay, Ui};
use crate::views::picker::{Picker, PickerEvent, PickerState};
use crate::zoom::px;

/// Mentions offered at once; typing narrows them.
const MENTION_ROWS: usize = 8;

actions!(composer, [Queue, PickModel]);

const CONTEXT: &str = "Composer";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-enter", Queue, Some(CONTEXT)),
        // The model menu without leaving the keyboard; the picker's own
        // return focus puts the caret back in the field.
        KeyBinding::new("cmd-m", PickModel, Some(CONTEXT)),
    ]);
}

pub enum ComposerEvent {
    /// A prompt left for the agent; the transcript follows it.
    Sent,
    /// The New lane dialog accepted its first prompt.
    CreateLane(LaneRequest),
    /// The prompt was not accepted and its draft was restored.
    SendFailed,
}

impl EventEmitter<ComposerEvent> for Composer {}

/// What the completion list under the caret offers.
enum Completion {
    Files(Query, Vec<String>),
    Commands(Query, Vec<CommandInfo>),
}

impl Completion {
    fn len(&self) -> usize {
        match self {
            Completion::Files(_, rows) => rows.len(),
            Completion::Commands(_, rows) => rows.len(),
        }
    }
    fn query(&self) -> &str {
        match self {
            Completion::Files(q, _) | Completion::Commands(q, _) => &q.query,
        }
    }
}

pub struct Composer {
    key: String,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    input: Entity<TextareaState>,
    attachments: Vec<Attachment>,
    /// Paths picked from the mention list, drawn as chips.
    mentioned: HashSet<String>,
    /// The completion list was closed with Escape until the text changes.
    dismissed: bool,
    picked: usize,
    /// The query the highlight was last reset for.
    picked_for: Option<String>,
    files: Option<(String, Vec<String>)>,
    commands: Option<Vec<CommandInfo>>,
    dragging: bool,
    model_picker: Entity<PickerState>,
    thinking_picker: Entity<PickerState>,
    placeholder_busy: bool,
    /// A New lane composer lists files from the project's checkout.
    file_root: Option<String>,
    new_lane: bool,
    disabled: bool,
}

impl Composer {
    pub fn new(
        key: String,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message the agent, @ for files, / for commands")
                .auto_grow(1, 12)
        });
        cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Change => this.changed(window, cx),
            InputEvent::Blur => cx.notify(),
            _ => {}
        })
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        let model_picker =
            cx.new(|cx| PickerState::new("Model", window, cx).return_focus(focus.clone()));
        cx.subscribe(&model_picker, |this, _, event, cx| match event {
            PickerEvent::Opened => {
                let key = this.key.clone();
                this.store
                    .update(cx, |store, cx| store.load_models(key, cx).detach());
            }
            PickerEvent::Selected(id) => {
                let key = this.key.clone();
                let current = this
                    .store
                    .read(cx)
                    .state
                    .settings
                    .get(&key)
                    .and_then(|s| s.model.as_ref().map(|m| m.id.clone()));
                if current.as_deref() != Some(id.as_str()) {
                    let id = id.clone();
                    this.store
                        .update(cx, |store, cx| store.set_model(key, id, cx));
                }
            }
        })
        .detach();
        let thinking_picker =
            cx.new(|cx| PickerState::new("Thinking", window, cx).return_focus(focus));
        cx.subscribe(&thinking_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(level) = event {
                let key = this.key.clone();
                let current = this
                    .store
                    .read(cx)
                    .state
                    .settings
                    .get(&key)
                    .and_then(|settings| settings.thinking_level.clone());
                if current.as_deref() != Some(level.as_str()) {
                    let level = level.clone();
                    this.store
                        .update(cx, |store, cx| store.set_thinking_level(key, level, cx));
                }
            }
        })
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            key,
            store,
            ui,
            input,
            attachments: Vec::new(),
            mentioned: HashSet::new(),
            dismissed: false,
            picked: 0,
            picked_for: None,
            files: None,
            commands: None,
            dragging: false,
            model_picker,
            thinking_picker,
            placeholder_busy: false,
            file_root: None,
            new_lane: false,
            disabled: false,
        }
    }

    /// The same prompt editor before a session exists. Its files come from the
    /// project, and submitting asks the New lane dialog to create the session.
    pub fn new_lane(
        cwd: String,
        prompt: Option<String>,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::new(String::new(), store, ui, window, cx);
        this.file_root = Some(cwd);
        this.new_lane = true;
        this.input.update(cx, |input, cx| {
            input.set_placeholder("Message the agent, @ for files", window, cx)
        });
        if let Some(prompt) = prompt {
            let caret = prompt.len();
            this.set_text(&prompt, caret, window, cx);
        }
        this
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Text and caret in, mentions drawn as chips.
    fn set_text(&mut self, text: &str, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.content(text);
        self.input.update(cx, |input, cx| {
            input.set_value(content, window, cx);
            input.set_selected_range(caret..caret, cx);
        });
    }

    fn content(&self, text: &str) -> InputContent {
        let mut content = InputContent::new(text.to_string());
        let mut at = 0;
        for segment in segments(text, &self.mentioned) {
            let end = at + segment.text.len();
            if segment.mention {
                let label = mention_label(&segment.text[1..]).to_string();
                let token = InlineToken::new(label, segment.text.clone());
                if let Ok(next) = content.clone().with_token(at..end, token) {
                    content = next;
                }
            }
            at = end;
        }
        content
    }

    fn restore(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.attachments = draft.attachments;
        self.mentioned = draft.mentioned.into_iter().collect();
        let caret = draft.text.len();
        self.set_text(&draft.text, caret, window, cx);
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attachments.clear();
        self.mentioned.clear();
        self.dismissed = false;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.dismissed = false;
        // A mention whose text was edited away is no longer one.
        let text = self.text(cx);
        self.mentioned
            .retain(|path| text.contains(&format!("@{path}")));
        cx.notify();
    }

    /// The completion list the caret is in, fetching the files or commands
    /// it needs on first use.
    fn completion(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Completion> {
        let input = self.input.read(cx);
        if self.dismissed || !input.focus_handle(cx).is_focused(window) {
            return None;
        }
        let range = input.selected_range();
        if !range.is_empty() {
            return None;
        }
        let text = input.value().to_string();
        let caret = input.cursor();
        if let Some(mention) = mention_at(&text, caret) {
            let cwd = match &self.file_root {
                Some(cwd) => cwd.clone(),
                None => self.store.read(cx).state.session(&self.key)?.cwd.clone(),
            };
            match &self.files {
                Some((loaded, list)) if *loaded == cwd => {
                    let rows = match_files(&mention.query, list, MENTION_ROWS);
                    return Some(Completion::Files(mention, rows));
                }
                _ => {
                    self.files = Some((cwd.clone(), Vec::new()));
                    let list = self.store.read(cx).list_files(cwd.clone());
                    cx.spawn(async move |this, cx| {
                        let list = list.await;
                        this.update(cx, |this, cx| {
                            this.files = Some((cwd, list));
                            cx.notify();
                        })
                    })
                    .detach();
                    return None;
                }
            }
        }
        if self.new_lane {
            return None;
        }
        let slash = command_at(&text, caret)?;
        match &self.commands {
            Some(list) => {
                let lane = self
                    .store
                    .read(cx)
                    .state
                    .session(&self.key)
                    .and_then(|s| s.lane_id)
                    .is_some();
                let rows = match_commands(&slash.query, &available_commands(list, lane), 8);
                Some(Completion::Commands(slash, rows))
            }
            None => {
                self.commands = Some(Vec::new());
                let list = self.store.read(cx).list_commands(self.key.clone());
                cx.spawn(async move |this, cx| {
                    let list = list.await.unwrap_or_default();
                    this.update(cx, |this, cx| {
                        this.commands = Some(list);
                        cx.notify();
                    })
                })
                .detach();
                None
            }
        }
    }

    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(completion) = self.completion(window, cx) else {
            return;
        };
        let text = self.text(cx);
        let caret = self.input.read(cx).cursor();
        let edited = match completion {
            Completion::Files(mention, rows) => {
                let Some(path) = rows.get(index) else { return };
                self.mentioned.insert(path.clone());
                insert_mention(&text, caret, &mention, path)
            }
            Completion::Commands(_, rows) => {
                let Some(command) = rows.get(index) else {
                    return;
                };
                insert_command(&text, caret, command)
            }
        };
        self.set_text(&edited.text, edited.caret, window, cx);
        self.dismissed = true;
        cx.notify();
    }

    fn submit(&mut self, queue: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if self.disabled || (text.trim().is_empty() && self.attachments.is_empty()) {
            return;
        }
        if self.new_lane {
            if let Some(request) = lane_request(&text, &self.attachments) {
                cx.emit(ComposerEvent::CreateLane(request));
            }
            return;
        }
        let state = &self.store.read(cx).state;
        if state.sending.contains(&self.key) {
            return;
        }
        let lane_command = text.trim();
        if lane_command == "/new" || lane_command == "/resume" {
            let lane_id = state.session(&self.key).and_then(|s| s.lane_id);
            let resume = lane_command == "/resume";
            self.clear(window, cx);
            match lane_id {
                None => self.store.update(cx, |store, cx| {
                    store.show_error("This command is only available in a lane", cx)
                }),
                Some(lane_id) if resume => self.ui.update(cx, |ui, cx| {
                    ui.open(
                        Overlay::ResumeSession {
                            lane_id,
                            session_id: None,
                        },
                        cx,
                    )
                }),
                Some(lane_id) => self
                    .store
                    .update(cx, |store, cx| store.new_lane_session(lane_id, cx).detach()),
            }
            return;
        }
        let (prompt, images) = compose_prompt(&text, &self.attachments);
        let unsent = Draft {
            text,
            attachments: self.attachments.clone(),
            mentioned: self.mentioned.iter().cloned().collect(),
        };
        self.clear(window, cx);
        let key = self.key.clone();
        let sent = self
            .store
            .update(cx, |store, cx| store.send(key, prompt, images, queue, cx));
        cx.emit(ComposerEvent::Sent);
        cx.spawn_in(window, async move |this, cx| {
            if sent.await {
                return;
            }
            // The prompt was not accepted: give back what was typed. If the
            // process exited, it may still have arrived, and the error says
            // to check first.
            let _ = this.update_in(cx, |this, window, cx| {
                cx.emit(ComposerEvent::SendFailed);
                let current = Draft {
                    text: this.text(cx),
                    attachments: this.attachments.clone(),
                    mentioned: this.mentioned.iter().cloned().collect(),
                };
                let draft = restore_unsent(unsent, Some(&current));
                this.restore(draft, window, cx);
            });
        })
        .detach();
    }

    /// Opens the model picker from the keyboard, so choosing a model never
    /// leaves the text field: the picker's own `Opened` event loads the
    /// models and its return focus puts the caret back afterwards.
    /// Without the session's settings the chip is not drawn, so neither is
    /// the list: the shortcut does nothing rather than hide focus in it.
    fn open_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.store.read(cx).state;
        let Some(settings) = state.settings.get(&self.key) else {
            return;
        };
        let models = state.models.get(&self.key).cloned().unwrap_or_default();
        let current = settings.model.as_ref().map_or("", |m| m.id.as_str());
        let groups = chip_model_groups(&models, current, &state.preferences);
        self.model_picker
            .update(cx, |picker, cx| picker.open(&groups, window, cx));
    }

    fn cycle_thinking(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.store.read(cx).state.settings.get(&self.key) else {
            return;
        };
        if settings.thinking_levels.len() < 2 {
            return;
        }
        if let Some(level) = next_level(
            &settings.thinking_levels,
            settings.thinking_level.as_deref(),
        ) {
            let key = self.key.clone();
            self.store
                .update(cx, |store, cx| store.set_thinking_level(key, level, cx));
        }
    }

    fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        let read = self
            .store
            .update(cx, |store, cx| store.read_attachments(paths, cx));
        cx.spawn(async move |this, cx| {
            if let Some(read) = read.await {
                let _ = this.update(cx, |this, cx| {
                    this.attachments.extend(read);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update(cx, |this, cx| this.attach_paths(paths, cx));
            }
            let _ = this.update_in(cx, |this, window, cx| this.focus(window, cx));
        })
        .detach();
    }

    /// Takes pasted images and copied files; text falls through to the field.
    fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        let mut taken = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    self.attachments.push(Attachment {
                        name: "Pasted image".into(),
                        path: String::new(),
                        mime_type: Some(image.format.mime_type().into()),
                        data: Some(base64::engine::general_purpose::STANDARD.encode(&image.bytes)),
                    });
                    taken = true;
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    self.attach_paths(paths.paths().to_vec(), cx);
                    taken = true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if taken {
            cx.notify();
        }
        taken
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let completion = self.completion(window, cx);
        let query = completion.as_ref().map(|c| c.query().to_string());
        if query != self.picked_for {
            self.picked_for = query;
            self.picked = 0;
        }
        let rows = completion.as_ref().map_or(0, Completion::len);
        let picked = self.picked.min(rows.saturating_sub(1));

        let state = &self.store.read(cx).state;
        let conversation = state.conversations.get(&self.key);
        let busy = conversation.is_some_and(|c| c.busy);
        let sending = state.sending.contains(&self.key);
        let queued = conversation.map(|c| c.queued.clone()).unwrap_or_default();
        let settings = state.settings.get(&self.key).cloned();
        let usage = conversation.and_then(|c| sum_usage(&c.messages));
        let models = state.models.get(&self.key).cloned().unwrap_or_default();
        let preferences = state.preferences.clone();
        let images = settings
            .as_ref()
            .and_then(|s| s.model.as_ref())
            .is_none_or(|m| m.images);
        let empty = if self.new_lane {
            self.text(cx).trim().is_empty()
        } else {
            self.text(cx).trim().is_empty() && self.attachments.is_empty()
        };
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let preview = state.previews.contains(&self.key);
        let placeholder = if preview {
            "Send a message to resume this session"
        } else if busy {
            "Steer the agent, \u{2325}\u{21b5} to queue for after"
        } else {
            "Message the agent, @ for files, / for commands"
        };
        if self.placeholder_busy != busy {
            self.placeholder_busy = busy;
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }

        let this = cx.entity().downgrade();
        let on_enter = this.clone();
        let on_tab = this.clone();
        let on_shift_tab = this.clone();
        let on_escape = this.clone();
        let on_up = this.clone();
        let on_down = this.clone();
        let on_queue = this.clone();
        let on_model = this.clone();
        let on_paste = this.clone();
        let on_drop = this.clone();
        let store = self.store.clone();
        let key = self.key.clone();

        let mut frame = div()
            .id("composer")
            .key_context(CONTEXT)
            .relative()
            .mx_auto()
            .flex()
            .flex_col()
            .w_full()
            .max_w(px(768.))
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(if self.dragging {
                p.focus
            } else if focused {
                p.line_strong
            } else {
                p.line
            })
            .bg(p.surface)
            .capture_action(move |action: &Enter, window, cx| {
                let _ = on_enter.update(cx, |this, cx| {
                    if action.shift {
                        let text = this.text(cx);
                        let range = this.input.read(cx).selected_range();
                        if let Some(next) = continue_list(&text, range.start, range.end) {
                            cx.stop_propagation();
                            this.set_text(&next.text, next.caret, window, cx);
                        }
                        return;
                    }
                    cx.stop_propagation();
                    if rows > 0 && !action.secondary {
                        return this.pick(picked, window, cx);
                    }
                    this.submit(busy && action.secondary, window, cx);
                });
            })
            .on_action(move |_: &Queue, window, cx| {
                let _ = on_queue.update(cx, |this, cx| this.submit(busy, window, cx));
            })
            .on_action(move |_: &PickModel, window, cx| {
                let _ = on_model.update(cx, |this, cx| this.open_model_picker(window, cx));
            })
            .capture_action(move |_: &IndentInline, window, cx| {
                if rows > 0 {
                    cx.stop_propagation();
                    let _ = on_tab.update(cx, |this, cx| this.pick(picked, window, cx));
                }
            })
            .capture_action(move |_: &OutdentInline, _, cx| {
                cx.stop_propagation();
                let _ = on_shift_tab.update(cx, |this, cx| this.cycle_thinking(cx));
            })
            .capture_action(move |_: &MoveUp, _, cx| {
                if rows > 0 {
                    cx.stop_propagation();
                    let _ = on_up.update(cx, |this, cx| {
                        this.picked = (picked + rows - 1) % rows;
                        cx.notify();
                    });
                }
            })
            .capture_action(move |_: &MoveDown, _, cx| {
                if rows > 0 {
                    cx.stop_propagation();
                    let _ = on_down.update(cx, |this, cx| {
                        this.picked = (picked + 1) % rows;
                        cx.notify();
                    });
                }
            })
            .capture_action(move |_: &Escape, _, cx| {
                let _ = on_escape.update(cx, |this, cx| {
                    if rows > 0 {
                        cx.stop_propagation();
                        this.dismissed = true;
                        cx.notify();
                    } else if busy {
                        cx.stop_propagation();
                        let key = this.key.clone();
                        this.store.update(cx, |store, cx| store.abort(key, cx));
                    }
                });
            })
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(p.focus))
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let _ = on_drop.update(cx, |this, cx| {
                    this.dragging = false;
                    if !this.disabled {
                        this.attach_paths(paths.paths().to_vec(), cx);
                    }
                });
            });

        if let Some(completion) = completion.filter(|c| c.len() > 0) {
            frame = frame.child(self.completion_list(completion, picked, &p, cx));
        }

        let chips = !queued.is_empty() || !self.attachments.is_empty();
        frame = frame.when(chips, |el| {
            el.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .px(px(8.))
                    .pt(px(8.))
                    .children(queued.iter().map(|q| {
                        let title = if q.mode == PromptMode::Steer {
                            "Steering the agent next"
                        } else {
                            "Queued for when the agent finishes"
                        };
                        chip(
                            SharedString::from(format!("queued-{}", q.id)),
                            ChipLead::Icon("history"),
                            q.text.clone(),
                            title.to_string(),
                            false,
                            false,
                            None,
                            &p,
                        )
                    }))
                    .children(self.attachments.iter().enumerate().map(|(i, file)| {
                        let lead = match (&file.mime_type, &file.data) {
                            (Some(mime), Some(data)) => ChipLead::Image(mime.clone(), data.clone()),
                            _ => ChipLead::Icon("file"),
                        };
                        let muted = file.data.is_some() && !images;
                        let title = if muted {
                            format!("{} (this model does not read images)", file.path)
                        } else {
                            file.path.clone()
                        };
                        let remove = this.clone();
                        let on_remove: OnRemove = Box::new(move |cx| {
                            let _ = remove.update(cx, |this, cx| {
                                if i < this.attachments.len() {
                                    this.attachments.remove(i);
                                }
                                cx.notify();
                            });
                        });
                        chip(
                            SharedString::from(format!("attachment-{i}")),
                            lead,
                            file.name.clone(),
                            title,
                            muted,
                            true,
                            Some(on_remove),
                            &p,
                        )
                    })),
            )
        });

        let link = p.link;
        let selection = p.selection;
        frame = frame.child(
            div().child(
                Textarea::new(&self.input)
                    .appearance(false)
                    .disabled(self.disabled)
                    .t_body()
                    .px(px(12.))
                    .pt(px(12.))
                    .pb(px(4.))
                    .token(move |token, _, _| {
                        div()
                            .rounded(px(RADIUS_XS))
                            .bg(selection)
                            .text_color(link)
                            .child(token.token().text().clone())
                    })
                    .on_paste(move |item, _, cx| {
                        on_paste
                            .update(cx, |this, cx| this.paste(item, cx))
                            .unwrap_or(false)
                    }),
            ),
        );

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .pb(px(8.))
            .child(toolbar_button(
                "attach",
                icon("plus", 16.).into_any_element(),
                "Attach files",
                self.disabled,
                false,
                &p,
                {
                    let this = this.clone();
                    move |window, cx| {
                        let _ = this.update(cx, |this, cx| this.pick_files(window, cx));
                    }
                },
            ))
            .children(settings.as_ref().map(|settings| {
                let current = settings.model.as_ref();
                let groups =
                    chip_model_groups(&models, current.map_or("", |m| m.id.as_str()), &preferences);
                let value = current.map_or("No model".to_string(), |m| {
                    short_model_id(&m.id, &m.provider)
                });
                let title = current.map_or("Choose a model".to_string(), |m| {
                    format!("Model: {} ({})", m.name, m.provider)
                });
                Picker::new(
                    "model-picker",
                    &self.model_picker,
                    groups,
                    div().t_code_sm().child(value),
                    &p,
                )
                .ghost()
                .up()
                .width(352.)
                .title(title)
                .empty_text("Loading models\u{2026}")
            }))
            .children(
                settings
                    .as_ref()
                    .filter(|s| !s.thinking_levels.is_empty())
                    .map(|settings| self.thinking_menu(settings, &p)),
            )
            .children(settings.as_ref().into_iter().flat_map(|settings| {
                settings.toggles.iter().map(|toggle| {
                    let toggle_store = store.clone();
                    let session = key.clone();
                    let id = toggle.id.clone();
                    option_toggle(
                        SharedString::from(format!("session-toggle-{}", toggle.id)),
                        toggle.label.clone(),
                        toggle.description.clone().map(SharedString::from),
                        toggle.enabled,
                        self.disabled,
                        &p,
                        move |enabled, _, cx| {
                            toggle_store.update(cx, |store, cx| {
                                store.set_toggle(session.clone(), id.clone(), enabled, cx)
                            });
                        },
                    )
                })
            }))
            .child(div().flex_1())
            .children(usage.map(|total| {
                let detail = session_usage_detail(&total);
                div()
                    .id("session-usage")
                    .pr(px(12.))
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                    .child(session_usage_label(&total))
            }))
            .children(settings.as_ref().and_then(|s| s.context).map(|context| {
                div()
                    .id("context-usage")
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .tooltip(|window, cx| {
                        Tooltip::new("Context used out of the model's window").build(window, cx)
                    })
                    .child(context_label(&context))
            }))
            .child(if busy {
                Button::new("stop")
                    .icon("square")
                    .tooltip("Stop (Esc)")
                    .on_click(move |_, _, cx| {
                        let key = key.clone();
                        store.update(cx, |store, cx| store.abort(key, cx));
                    })
            } else {
                let send = this.clone();
                Button::new("send")
                    .primary()
                    .icon("arrow-up")
                    .disabled(empty || sending || self.disabled)
                    .tooltip(if self.new_lane {
                        "Create lane (Enter)"
                    } else {
                        "Send (Enter)"
                    })
                    .on_click(move |_, window, cx| {
                        let _ = send.update(cx, |this, cx| this.submit(false, window, cx));
                    })
            });
        frame.child(toolbar)
    }
}

impl Composer {
    fn completion_list(
        &self,
        completion: Completion,
        picked: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let row = |index: usize, content: Vec<AnyElement>| {
            let (hover, click) = (this.clone(), this.clone());
            div()
                .id(SharedString::from(format!("completion-{index}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .min_h(px(CONTROL))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(RADIUS_SM))
                .when(index == picked, |el| el.bg(p.selection))
                .on_mouse_move(move |_, _, cx| {
                    let _ = hover.update(cx, |this, cx| {
                        if this.picked != index {
                            this.picked = index;
                            cx.notify();
                        }
                    });
                })
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_, window, cx| {
                    let _ = click.update(cx, |this, cx| this.pick(index, window, cx));
                })
                .children(content)
        };
        let rows: Vec<_> = match completion {
            Completion::Files(_, files) => files
                .iter()
                .enumerate()
                .map(|(i, path)| {
                    let slash = path.rfind('/').map_or(0, |s| s + 1);
                    row(
                        i,
                        vec![
                            icon("file", 14.)
                                .flex_none()
                                .text_color(p.ink_muted)
                                .into_any_element(),
                            div()
                                .flex_none()
                                .t_code_sm()
                                .text_color(p.ink)
                                .child(path[slash..].to_string())
                                .into_any_element(),
                            div()
                                .min_w_0()
                                .truncate()
                                .t_code_sm()
                                .text_color(p.ink_muted)
                                .child(path[..slash].to_string())
                                .into_any_element(),
                        ],
                    )
                })
                .collect(),
            Completion::Commands(_, commands) => commands
                .iter()
                .enumerate()
                .map(|(i, command)| {
                    let detail = command
                        .description
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", command.kind).to_lowercase());
                    row(
                        i,
                        vec![
                            icon("terminal", 14.)
                                .flex_none()
                                .text_color(p.ink_muted)
                                .into_any_element(),
                            div()
                                .flex_none()
                                .t_code_sm()
                                .text_color(p.ink)
                                .child(format!("/{}", command.name))
                                .into_any_element(),
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .t_small()
                                .text_color(p.ink_muted)
                                .child(detail)
                                .into_any_element(),
                        ],
                    )
                })
                .collect(),
        };
        deferred(
            div()
                .absolute()
                .bottom_full()
                .left_0()
                .mb(px(8.))
                .w_full()
                .max_w(px(448.))
                .child(
                    panel(p)
                        .id("completion-list")
                        .occlude()
                        .p(px(4.))
                        .t_body()
                        .children(rows),
                ),
        )
        .with_priority(2)
    }

    fn thinking_menu(
        &self,
        settings: &panoptes_harness::SessionSettings,
        p: &Palette,
    ) -> impl IntoElement {
        let fixed = settings.thinking_levels.len() < 2;
        let level = settings.thinking_level.clone();
        let title = if fixed {
            "This model does not think"
        } else {
            "Thinking level (Shift+Tab to cycle)"
        };
        Picker::new(
            "thinking-picker",
            &self.thinking_picker,
            session_thinking_groups(
                &settings.thinking_levels,
                level.as_deref().unwrap_or_default(),
            ),
            div().t_code_sm().child(format!(
                "Thinking: {}",
                level.unwrap_or_else(|| "off".into())
            )),
            p,
        )
        .ghost()
        .up()
        .width(180.)
        .searchable(false)
        .disabled(fixed)
        .title(title)
    }
}

type OnRemove = Box<dyn Fn(&mut App)>;

enum ChipLead {
    Icon(&'static str),
    Image(String, String),
}

#[allow(clippy::too_many_arguments)]
fn chip(
    id: SharedString,
    lead: ChipLead,
    label: String,
    title: String,
    muted: bool,
    mono: bool,
    on_remove: Option<OnRemove>,
    p: &Palette,
) -> impl IntoElement {
    let removable = on_remove.is_some();
    let lead = match lead {
        ChipLead::Icon(name) => icon(name, 12.)
            .flex_none()
            .text_color(p.ink_muted)
            .into_any_element(),
        ChipLead::Image(mime, data) => match crate::views::turn::decoded(&mime, &data) {
            Some(image) => img(image)
                .size(px(16.))
                .flex_none()
                .rounded(px(RADIUS_XS))
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            None => icon("image", 12.).flex_none().into_any_element(),
        },
    };
    let remove_label = format!("Remove {label}");
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(4.))
        .h(px(CONTROL_SM))
        .max_w(px(256.))
        .pl(px(4.))
        .when(!removable, |el| el.pr(px(8.)))
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_sunken)
        .text_color(if muted { p.ink_faint } else { p.ink })
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .child(lead)
        .child(
            div()
                .min_w_0()
                .truncate()
                .when(mono, |el| el.t_code_sm())
                .when(!mono, |el| el.t_body())
                .child(label),
        )
        .children(on_remove.map(|remove| {
            let (muted_ink, ink, selection) = (p.ink_muted, p.ink, p.selection);
            div()
                .id("remove")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded(px(RADIUS_XS))
                .text_color(muted_ink)
                .hover(move |s| s.bg(selection).text_color(ink))
                .tooltip(move |window, cx| Tooltip::new(remove_label.clone()).build(window, cx))
                .on_click(move |_, _, cx| remove(cx))
                .child(icon("x", 12.))
        }))
}

fn toolbar_button(
    id: &'static str,
    content: AnyElement,
    title: &'static str,
    disabled: bool,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (selection, ink) = (p.selection, p.ink);
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .h(px(CONTROL_SM))
        .min_w(px(CONTROL_SM))
        .px(px(6.))
        .rounded(px(RADIUS_SM))
        .t_body()
        .text_color(if disabled { p.ink_faint } else { p.ink_muted })
        .when(active, |el| el.bg(selection).text_color(ink))
        .when(!disabled, |el| {
            el.cursor_pointer()
                .hover(move |s| s.bg(selection).text_color(ink))
                .on_click(move |_, window, cx| on_click(window, cx))
        })
        .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
        .child(content)
}
