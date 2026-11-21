//! One pane's session: header, transcript, notices, the cards waiting for an
//! answer, and the composer.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_harness::{Answer, Approval, NoticeLevel, SessionStatus};

use crate::ds::{Button, LoopState, Type as _, icon, loop_mark, status_pill};
use crate::model::format::now_ms;
use crate::model::panes::SplitDirection;
use crate::model::turns::{build_turns, loose_approvals, prompt_count, sent_turn};
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::Ui;
use crate::views::cards::{QuestionField, approval_card, question_card};
use crate::views::composer::{Composer, ComposerEvent};
use crate::views::turn::{Ctx, OnCopy, OnToggle, OnUnclamp, ViewState, turn_view};
use crate::zoom::px;

/// Turns rendered at first; earlier ones load on request.
const PAGE: usize = 50;
const PERMISSION_INSTALL_URL: &str =
    "https://www.npmjs.com/package/pi-permission-system#installation";

pub struct ChatPane {
    pub key: String,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    composer: Entity<Composer>,
    scroll: ScrollHandle,
    view: ViewState,
    shown: usize,
    /// Follow new output while at the bottom; stop once scrolled up.
    pinned: bool,
    /// How many messages the transcript had when last drawn.
    seen: usize,
    /// The text fields of questions waiting for an answer, by question id.
    fields: HashMap<String, QuestionField>,
    /// Prompt count when a send started; the next prompt's turn animates in.
    send_from: Option<usize>,
    /// The turn whose prompt is entering from the composer.
    sent_turn: Option<String>,
    /// The expiry a redraw is already waiting for.
    expiry_timer: Option<i64>,
    /// The title field while the session is being renamed.
    renaming: Option<Entity<InputState>>,
}

impl ChatPane {
    pub fn new(
        key: String,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&store, |this, _, cx| {
            this.follow();
            cx.notify();
        })
        .detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        let composer =
            cx.new(|cx| Composer::new(key.clone(), store.clone(), ui.clone(), window, cx));
        cx.subscribe(&composer, |this, _, event, cx| match event {
            ComposerEvent::Sent => {
                this.send_from = Some(
                    this.store
                        .read(cx)
                        .state
                        .conversations
                        .get(&this.key)
                        .map_or(0, |c| prompt_count(&c.messages)),
                );
                this.scroll_to_end(cx);
            }
            ComposerEvent::SendFailed => {
                this.send_from = None;
                this.sent_turn = None;
                cx.notify();
            }
            // Only the New lane dialog's composer creates lanes.
            ComposerEvent::CreateLane(_) => {}
        })
        .detach();
        let scroll = ScrollHandle::new();
        scroll.scroll_to_bottom();
        Self {
            key,
            store,
            ui,
            composer,
            scroll,
            view: ViewState::default(),
            shown: PAGE,
            pinned: true,
            seen: 0,
            fields: HashMap::new(),
            send_from: None,
            sent_turn: None,
            expiry_timer: None,
            renaming: None,
        }
    }

    /// Keeps the end in view while pinned there.
    fn follow(&mut self) {
        if self.pinned {
            self.scroll.scroll_to_bottom();
        }
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroll.scroll_to_bottom();
        self.pinned = true;
        cx.notify();
    }

    /// Swaps the title for a field holding it.
    fn start_rename(&mut self, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        cx.subscribe(&input, |this, input, event, cx| {
            let current = this.renaming.as_ref() == Some(&input);
            if current && matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_rename(cx);
            }
        })
        .detach();
        input.update(cx, |input, cx| input.focus(window, cx));
        self.renaming = Some(input);
        cx.notify();
    }

    /// Renames to what the field holds; an empty name falls back.
    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.renaming.take() else {
            return;
        };
        let title = input.read(cx).value().to_string();
        let key = self.key.clone();
        self.store
            .update(cx, |store, cx| store.rename(key, &title, cx));
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            self.focus(window, cx);
            cx.notify();
        }
    }

    /// The session's name: click it to rename. Only a session a lane has
    /// adopted has a record to rename.
    fn render_title(
        &self,
        title: SharedString,
        renamable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(input) = &self.renaming {
            return div()
                .flex_1()
                .min_w_0()
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    this.cancel_rename(window, cx);
                    cx.stop_propagation();
                }))
                .child(
                    div().max_w(px(448.)).child(
                        Input::new(input)
                            .appearance(false)
                            .t_body_strong()
                            .px(px(0.)),
                    ),
                )
                .into_any_element();
        }
        let label = div()
            .min_w_0()
            .truncate()
            .t_body_strong()
            .child(title.clone());
        if !renamable {
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(label)
                .into_any_element();
        }
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .child(
                label
                    .id("session-title")
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .tooltip(|window, cx| {
                        crate::ds::Tooltip::new("Rename session").build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.start_rename(title.to_string(), window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_header(
        &self,
        focused: bool,
        can_close: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = &self.store.read(cx).state;
        let status = state.status(&self.key);
        let session = state.session(&self.key);
        let renamable = session.is_some_and(|s| s.id.is_some());
        let title: SharedString = session
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Session".into())
            .into();
        let ui = self.ui.clone();
        let split_store = self.store.clone();
        let split = move |direction: SplitDirection| {
            let (store, ui) = (split_store.clone(), ui.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                crate::ui::split_focused_pane(&store, &ui, direction, cx)
            }
        };
        let pane_id = self
            .ui
            .read(cx)
            .layout
            .session_pane(&self.key)
            .map(|p| p.id.clone());
        div()
            .id("pane-header")
            .on_drag(
                crate::ui::DraggedSession {
                    key: self.key.clone(),
                },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            )
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(38.))
            .px(px(12.))
            .border_b_1()
            .border_color(if focused { p.line_strong } else { p.line })
            .when(status == SessionStatus::Running, |el| {
                el.child(loop_mark("pane-running", 20., LoopState::Thinking, p.ink))
            })
            .child(self.render_title(title, renamable, cx))
            .when(status != SessionStatus::Idle, |el| {
                el.child(status_pill(status, p))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(
                        Button::new("split-vertical")
                            .ghost()
                            .small()
                            .icon("panel-left")
                            .tooltip("Split vertically (⌘D)")
                            .on_click(split(SplitDirection::Vertical)),
                    )
                    .child(
                        Button::new("split-horizontal")
                            .ghost()
                            .small()
                            .glyph(icon("panel-left", 16.).rotate(90.))
                            .tooltip("Split horizontally (⌘⇧D)")
                            .on_click(split(SplitDirection::Horizontal)),
                    )
                    .when(can_close, |el| {
                        let ui = self.ui.clone();
                        el.child(
                            Button::new("close-pane")
                                .ghost()
                                .small()
                                .icon("x")
                                .tooltip("Close pane")
                                .on_click(move |_, _, cx| {
                                    if let Some(pane_id) = &pane_id {
                                        ui.update(cx, |ui, cx| ui.close_pane(pane_id, cx));
                                    }
                                }),
                        )
                    }),
            )
    }
}

impl Render for ChatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields(window, cx);
        let p = *cx.palette();
        let ui = self.ui.read(cx);
        let focused = ui
            .layout
            .find(&ui.focused_pane)
            .is_some_and(|pane| pane.session_key.as_deref() == Some(self.key.as_str()));
        let can_close = ui.layout.leaves().len() > 1;
        let state = &self.store.read(cx).state;
        let conversation = state.conversations.get(&self.key);
        let open_error = state.open_errors.get(&self.key).cloned();
        let unguarded = state
            .unguarded_tools
            .get(&self.key)
            .copied()
            .unwrap_or(false);
        let warning = unguarded && !state.preferences.permission_warning_dismissed;
        let preview = state.previews.contains(&self.key);
        let status = state.status(&self.key);
        let session = state.session(&self.key).cloned();
        let turns = conversation.map(build_turns).unwrap_or_default();
        let notices = conversation.map(|c| c.notices.clone()).unwrap_or_default();
        let exited = conversation.and_then(|c| c.exited.clone());
        let questions = conversation
            .map(|c| c.questions.clone())
            .unwrap_or_default();
        let loose = conversation.map(loose_approvals).unwrap_or_default();
        let has_conversation = conversation.is_some();
        let args_text = conversation
            .map(|c| c.args_text.clone())
            .unwrap_or_default();
        let messages = conversation.map_or(0, |c| c.messages.len());
        if let Some(turn) = self.send_from.and_then(|before| sent_turn(&turns, before)) {
            self.sent_turn = Some(turn.key.clone());
            self.send_from = None;
        }
        let behind = !self.pinned && messages > self.seen;
        if self.pinned {
            self.seen = messages;
        }

        let this = cx.entity().downgrade();
        let on_toggle: OnToggle = {
            let this = this.clone();
            Arc::new(move |id, open, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.view.toggled.insert(id, open);
                    cx.notify();
                });
            })
        };
        let on_unclamp: OnUnclamp = {
            let this = this.clone();
            Arc::new(move |id, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.view.unclamped.insert(id);
                    cx.notify();
                });
            })
        };
        let on_copy: OnCopy = {
            let this = this.clone();
            Arc::new(move |key, text, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                let _ = this.update(cx, |this, cx| {
                    this.view.copied = Some(key);
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(1500))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            this.view.copied = None;
                            cx.notify();
                        });
                    })
                    .detach();
                });
            })
        };
        let (card_store, card_key) = (self.store.clone(), self.key.clone());
        let approval = |a: &Approval| approval_card(a, false, &card_key, &card_store, &p);
        let ctx = Ctx {
            p: &p,
            view: &self.view,
            args_text: &args_text,
            on_toggle,
            on_unclamp,
            on_copy,
            approval: &approval,
            sent_turn: self.sent_turn.as_deref(),
        };
        let hidden = turns.len().saturating_sub(self.shown);

        let mut transcript = div()
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(24.))
            .w_full()
            .max_w(px(768.));
        if !has_conversation {
            transcript = transcript.child(match &open_error {
                Some(error) => {
                    let store = self.store.clone();
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .text_color(p.error)
                                .child(icon("circle-alert", 16.))
                                .child(div().t_body_strong().child("Could not start the agent")),
                        )
                        .child(div().text_color(p.ink_muted).child(error.clone()))
                        .child(
                            Button::new("retry-open")
                                .small()
                                .icon("refresh-cw")
                                .label("Try again")
                                .disabled(session.is_none())
                                .on_click(move |_, _, cx| {
                                    if let Some(session) = &session {
                                        store.update(cx, |store, cx| {
                                            store.open(session, cx).detach()
                                        });
                                    }
                                }),
                        )
                        .into_any_element()
                }
                None => div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .text_color(p.ink_muted)
                    .child(loop_mark("starting", 20., LoopState::Thinking, p.ink_muted))
                    .child("Starting the agent…")
                    .into_any_element(),
            });
        }
        if hidden > 0 {
            let this = this.clone();
            transcript = transcript.child(
                div().flex().justify_center().child(
                    Button::new("earlier")
                        .ghost()
                        .small()
                        .label(format!(
                            "Show {} earlier turns ({hidden} hidden)",
                            PAGE.min(hidden)
                        ))
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.shown += PAGE;
                                cx.notify();
                            });
                        }),
                ),
            );
        }
        transcript = transcript.children(turns[hidden..].iter().map(|t| turn_view(t, &ctx)));

        let scroller = div()
            .id(SharedString::from(format!("transcript-{}", self.key)))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll(cx.listener(|this, _: &ScrollEvent, _, cx| {
                this.follow_after_scroll();
                cx.notify();
            }))
            .px(px(16.))
            .py(px(24.))
            .child(transcript);

        let jump = behind.then(|| {
            let this = this.clone();
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(8.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .rounded(px(4.))
                        .bg(p.surface_raised)
                        .shadow(p.shadow_pop())
                        .child(
                            Button::new("jump")
                                .small()
                                .icon("chevron-down")
                                .label("Jump to latest")
                                .kbd(vec!["⌘", "↓"])
                                .on_click(move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| this.scroll_to_end(cx));
                                }),
                        ),
                )
        });

        let warning_bar = warning.then(|| {
            let store = self.store.clone();
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .border_b_1()
                .border_color(p.line)
                .bg(p.waiting_soft)
                .px(px(16.))
                .py(px(8.))
                .text_color(p.waiting)
                .child(icon("circle-alert", 16.).flex_none())
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .child("No permission extension installed: bash, edit and write run without asking.")
                        .child(
                            div()
                                .id("permission-link")
                                .underline()
                                .cursor_pointer()
                                .t_code()
                                .child("pi install npm:pi-permission-system")
                                .on_click(|_, _, cx| cx.open_url(PERMISSION_INSTALL_URL)),
                        ),
                )
                .child(
                    Button::new("dismiss-permission")
                        .ghost()
                        .small()
                        .icon("x")
                        .on_click(move |_, _, cx| {
                            store.update(cx, |store, cx| store.dismiss_permission_warning(cx))
                        }),
                )
        });

        let bottom = div()
            .px(px(16.))
            .pb(px(16.))
            .pt(px(8.))
            .children(questions.iter().map(|q| {
                question_card(
                    q,
                    self.fields.get(&q.id),
                    &self.key,
                    &self.store,
                    &p,
                    window,
                    cx,
                )
            }))
            .children(loose.iter().map(|a| {
                div()
                    .mx_auto()
                    .mb(px(12.))
                    .w_full()
                    .max_w(px(768.))
                    .child(approval_card(a, true, &self.key, &self.store, &p))
            }))
            .when(status == SessionStatus::Running, |el| {
                el.child(
                    div()
                        .mx_auto()
                        .flex()
                        .w_full()
                        .max_w(px(768.))
                        .items_center()
                        .gap(px(8.))
                        .pb(px(4.))
                        .text_color(p.running)
                        .child(loop_mark(
                            "composer-running",
                            16.,
                            LoopState::Thinking,
                            p.running,
                        ))
                        .child("Running"),
                )
            })
            .child(self.composer.clone());

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(p.bg)
            .text_color(p.ink)
            .t_body()
            .child(self.render_header(focused, can_close, &p, cx))
            .children(warning_bar)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(scroller)
                    .children(jump),
            )
            .children(notices.iter().map(|notice| {
                let (bg, ink) = match notice.level {
                    NoticeLevel::Error => (Some(p.error_soft), p.error),
                    NoticeLevel::Warning => (Some(p.waiting_soft), p.waiting),
                    NoticeLevel::Info => (None, p.ink_muted),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(p.line)
                    .px(px(16.))
                    .py(px(4.))
                    .when_some(bg, |el, bg| el.bg(bg))
                    .text_color(ink)
                    .when(notice.level != NoticeLevel::Info, |el| {
                        el.child(icon("circle-alert", 14.))
                    })
                    .child(notice.text.clone())
            }))
            .children(preview.then(|| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(p.line)
                    .bg(p.waiting_soft)
                    .px(px(16.))
                    .py(px(4.))
                    .text_color(p.waiting)
                    .child(icon("history", 14.))
                    .child(
                        "Previewing this session. Send a message to resume it and stop the lane's current session.",
                    )
            }))
            .children(exited.map(|_| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(p.line)
                    .bg(p.error_soft)
                    .px(px(16.))
                    .py(px(4.))
                    .text_color(p.error)
                    .child(icon("circle-alert", 14.))
                    .child("The agent process stopped. Send a message to restart it.")
            }))
            .child(bottom)
    }
}

impl ChatPane {
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// Keeps a text field for each waiting question that asks for text, and
    /// redraws when the soonest card expires.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation) = self.store.read(cx).state.conversations.get(&self.key) else {
            self.fields.clear();
            return;
        };
        let questions = conversation.questions.clone();
        let expiries: Vec<i64> = questions
            .iter()
            .filter_map(|q| q.expires_at)
            .chain(conversation.approvals.iter().filter_map(|a| a.expires_at))
            .collect();
        self.fields
            .retain(|id, _| questions.iter().any(|q| &q.id == id));
        for question in &questions {
            if self.fields.contains_key(&question.id) {
                continue;
            }
            let Some(field) = QuestionField::new(question, window, cx) else {
                continue;
            };
            if let QuestionField::Line(state) = &field {
                let (store, key, id) = (self.store.clone(), self.key.clone(), question.id.clone());
                cx.subscribe(state, move |_, state, event, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        let value = state.read(cx).value().to_string();
                        let (key, id) = (key.clone(), id.clone());
                        store.update(cx, |store, cx| {
                            store.answer(id, Answer::Value { value }, Some(key), cx)
                        });
                    }
                })
                .detach();
            }
            self.fields.insert(question.id.clone(), field);
        }
        let now = now_ms();
        if let Some(next) = expiries.into_iter().filter(|at| *at > now).min()
            && self.expiry_timer != Some(next)
        {
            self.expiry_timer = Some(next);
            let wait = std::time::Duration::from_millis((next - now) as u64);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
    }

    fn follow_after_scroll(&mut self) {
        // Offsets are negative going down.
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset();
        self.pinned = (max.y + offset.y).abs() < px(24.);
    }
}
