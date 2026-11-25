//! The window: banners, the sidebar and the tiled session panes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::base::{h_resizable, resizable_panel};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::service::SessionEvent;
use tokio::sync::mpsc;

use crate::backend::Backend;
use crate::ds::{Button, LoopState, Type as _, icon, loop_mark};
use crate::model::diagnostics::{DiagnosticsFilter, ScopeChange, scope_change};
use crate::model::panes::{DropZone, PaneNode, SplitDirection, drop_zone_at};
use crate::model::review;
use crate::store::AppStore;
use crate::store::Startup;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::{DraggedSession, Overlay, Ui, split_focused_pane};
use crate::views::chat::ChatPane;
use crate::views::cleanup::Cleanup;
use crate::views::diagnostics::DiagnosticsPane;
use crate::views::image_zoom::ImageZoom;
use crate::views::jump::{Jump, JumpKind};
use crate::views::lane_dialogs::LaneDialogs;
use crate::views::new_lane::NewLane;
use crate::views::resume::ResumeSession;
use crate::views::review::ReviewPane;
use crate::views::settings::Settings;
use crate::views::shortcuts::{
    self as keys, AddProject, CloseWindow, HideApp, HideOthers, JumpToLatest,
    NewLane as NewLaneAction, NewLaneSession, Quit, Search, Shortcuts, ShowAll, ShowShortcuts,
    SplitHorizontal, SplitVertical, ToggleReview, ZoomIn, ZoomOut, ZoomReset,
};
use crate::views::sidebar::{Sidebar, SidebarEvent};
use crate::views::startup;
use crate::zoom::{ZoomCommand, px};

pub struct Workspace {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    sidebar: Entity<Sidebar>,
    dialogs: Entity<LaneDialogs>,
    review: Entity<ReviewPane>,
    /// The overlay shown, and the view drawing it.
    overlay: Option<(Overlay, AnyView)>,
    /// The image shown, and its persistent view. Keeping the entity preserves
    /// focus and the scroll position of tall screenshots across redraws.
    image_zoom: Option<(crate::views::turn::ZoomedImage, AnyView)>,
    /// The diagnostics pane and the scope it shows. Kept in step with the
    /// scope `Ui` asks for, the pane's own pickers included.
    diagnostics: Option<(DiagnosticsFilter, Entity<DiagnosticsPane>)>,
    /// Holds the keyboard when no text field does, so window shortcuts work.
    focus: FocusHandle,
    /// Whether a slow first load has gone on long enough to show a loader.
    show_loader: bool,
    /// Where a dragged session would land: a pane and a zone of it.
    drop_target: Option<(String, DropZone)>,
    panes: HashMap<String, Entity<ChatPane>>,
    /// Open conversations when last seen, to close the panes of ones that went.
    open_keys: HashSet<String>,
    active_key: Option<String>,
    review_was_open: bool,
}

impl Workspace {
    pub fn new(
        backend: Arc<Backend>,
        mut events: mpsc::UnboundedReceiver<SessionEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = cx.new(|cx| AppStore::new(backend, cx));
        let ui = cx.new(|_| Ui::default());
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), ui.clone(), cx));
        let dialogs = cx.new(|_| LaneDialogs::new(store.clone()));
        let review = cx.new(|cx| ReviewPane::new(store.clone(), ui.clone(), cx));
        cx.observe(&dialogs, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&sidebar, |this, _, event, cx| {
            this.on_sidebar(event.clone(), cx)
        })
        .detach();
        cx.observe(&store, |this, _, cx| this.sync_panes(cx))
            .detach();
        cx.observe_in(&ui, window, |this, _, window, cx| {
            this.sync_overlay(window, cx);
            this.sync_image_zoom(window, cx);
            let open = this.ui.read(cx).review_open;
            if open != this.review_was_open {
                if open {
                    this.review
                        .update(cx, |review, cx| review.focus(window, cx));
                } else {
                    this.focus_active(window, cx);
                }
                this.review_was_open = open;
            }
            this.sync_diagnostics(window, cx);
            cx.notify();
        })
        .detach();

        let events_store = store.downgrade();
        cx.spawn(async move |_, cx| {
            while let Some(event) = events.recv().await {
                if events_store
                    .update(cx, |store, cx| store.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(startup::LOADER_DELAY_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.show_loader = true;
                cx.notify();
            });
        })
        .detach();
        let window_handle = window.window_handle();
        let this = cx.entity().downgrade();
        cx.on_system_notification_response(move |response, cx| {
            let key = response.tag.to_string();
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.open_notification(&key, window, cx));
            });
        });
        Self {
            store,
            ui,
            sidebar,
            dialogs,
            review,
            overlay: None,
            image_zoom: None,
            diagnostics: None,
            focus,
            show_loader: false,
            drop_target: None,
            panes: HashMap::new(),
            open_keys: HashSet::new(),
            active_key: None,
            review_was_open: false,
        }
    }

    /// Builds the view for the overlay the window shows, once per opening.
    fn sync_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.ui.read(cx).overlay.clone();
        if self.overlay.as_ref().map(|(o, _)| o) == wanted.as_ref() {
            return;
        }
        let (store, ui) = (self.store.clone(), self.ui.clone());
        self.overlay = wanted.map(|overlay| {
            let view: AnyView = match &overlay {
                Overlay::NewLane { project_id, prompt } => {
                    let (id, prompt) = (*project_id, prompt.clone());
                    cx.new(|cx| NewLane::new(id, prompt, store, ui, window, cx))
                        .into()
                }
                Overlay::ResumeSession {
                    lane_id,
                    session_id,
                } => {
                    let (lane, session) = (*lane_id, *session_id);
                    cx.new(|cx| ResumeSession::new(lane, session, store, ui, window, cx))
                        .into()
                }
                Overlay::Cleanup { project_id } => {
                    let id = *project_id;
                    cx.new(|cx| Cleanup::new(id, store, ui, window, cx)).into()
                }
                Overlay::Search => cx
                    .new(|cx| Jump::new(JumpKind::Search, store, ui, window, cx))
                    .into(),
                Overlay::PaneSession { pane_id } => {
                    let kind = JumpKind::PaneSession(pane_id.clone());
                    cx.new(|cx| Jump::new(kind, store, ui, window, cx)).into()
                }
                Overlay::Shortcuts => cx.new(|cx| Shortcuts::new(ui, window, cx)).into(),
                Overlay::Settings => cx.new(|cx| Settings::new(store, ui, window, cx)).into(),
            };
            (overlay, view)
        });
        if self.overlay.is_none() && self.image_zoom.is_none() {
            self.focus_active(window, cx);
        }
    }

    /// Builds the lightbox once per image, rather than on every workspace draw.
    fn sync_image_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.ui.read(cx).image.clone();
        if self.image_zoom.as_ref().map(|(image, _)| image) == wanted.as_ref() {
            return;
        }
        let ui = self.ui.clone();
        self.image_zoom = wanted.map(|image| {
            let view = cx
                .new(|cx| ImageZoom::new(ui, image.clone(), window, cx))
                .into();
            (image, view)
        });
    }

    /// Builds the diagnostics pane when opened, moves it to a newly requested
    /// scope, and drops its refresh clock on close.
    fn sync_diagnostics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.ui.read(cx).diagnostics;
        let open = self.diagnostics.as_ref().map(|(filter, _)| *filter);
        match scope_change(open, wanted) {
            ScopeChange::Nothing => {}
            ScopeChange::Open(filter) => {
                let (store, ui) = (self.store.clone(), self.ui.clone());
                let view = cx.new(|cx| DiagnosticsPane::new(store, ui, filter, window, cx));
                self.diagnostics = Some((filter, view));
            }
            // A request while the pane is open must move it, even though the
            // pane may already have left the scope it was opened with.
            ScopeChange::Rescope(filter) => {
                if let Some((scope, view)) = self.diagnostics.as_mut() {
                    view.update(cx, |pane, cx| pane.set_filter(filter, cx));
                    *scope = filter;
                }
            }
            ScopeChange::Close => {
                self.diagnostics = None;
                self.focus_active(window, cx);
            }
        }
    }

    /// Places a dropped session: in the pane, or in a split beside it.
    fn drop_session(&mut self, key: String, pane: &str, cx: &mut Context<Self>) {
        let Some((target, zone)) = self.drop_target.take() else {
            return;
        };
        if target != pane {
            return;
        }
        let visible = self.ui.read(cx).layout.session_pane(&key).is_some();
        self.ui
            .update(cx, |ui, cx| ui.drop_session(&key, pane, zone, cx));
        self.store.update(cx, |store, cx| {
            store.state.active_key = Some(key.clone());
            if !visible
                && let Some(session) = store.state.sessions.iter().find(|s| s.key == key).cloned()
            {
                store.open(&session, cx).detach();
            }
            cx.notify();
        });
    }

    /// Focus back on the focused pane's composer.
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = self.ui.read(cx);
        let key = ui
            .layout
            .find(&ui.focused_pane)
            .and_then(|leaf| leaf.session_key.clone());
        match key.and_then(|key| self.panes.get(&key)) {
            Some(pane) => pane.update(cx, |pane, cx| pane.focus(window, cx)),
            None => window.focus(&self.focus, cx),
        }
    }

    /// Opens the session named by a clicked system notification and brings
    /// the main window and its composer to the front.
    fn open_notification(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.dismiss_system_notification(key);
        cx.activate(true);
        window.activate_window();

        let session = self.store.read(cx).state.session(key).cloned();
        self.ui.update(cx, |ui, cx| {
            ui.close(cx);
            ui.close_image(cx);
            if session.is_some() {
                ui.show_session(key, None, cx);
            }
        });
        if let Some(session) = session {
            self.store.update(cx, |store, cx| {
                store.open(&session, cx).detach();
            });
            self.sync_panes(cx);
            let pane = self.pane(key, window, cx);
            pane.update(cx, |pane, cx| pane.focus(window, cx));
        } else {
            self.store.update(cx, |store, cx| {
                store.show_error("That session is no longer available.", cx)
            });
            self.focus_active(window, cx);
        }
    }

    /// Creating a session goes through the store's active key: show it in
    /// the focused pane. A conversation that was forgotten closes its pane.
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let state = &self.store.read(cx).state;
        let active = state.active_key.clone();
        let open: HashSet<String> = state.conversations.keys().cloned().collect();
        let next_lane = active.as_ref().and_then(|key| {
            state
                .sessions
                .iter()
                .find(|session| &session.key == key)
                .and_then(|session| session.lane_id)
        });
        let current_review = state
            .reviews
            .values()
            .find(|review| review.visible)
            .map(|review| review.lane_id);
        if active != self.active_key {
            self.active_key = active.clone();
            if let Some(key) = &active {
                self.ui.update(cx, |ui, cx| ui.show_session(key, None, cx));
            }
            match review::follow_lane(self.ui.read(cx).review_open, current_review, next_lane) {
                review::FollowLane::Leave => {}
                review::FollowLane::Open(lane_id) => self
                    .store
                    .update(cx, |store, cx| store.open_review(lane_id, cx)),
                review::FollowLane::Close => {
                    self.store.update(cx, |store, cx| store.close_review(cx))
                }
            }
        }
        let gone: Vec<String> = self.open_keys.difference(&open).cloned().collect();
        self.open_keys = open;
        for key in gone {
            let pane = self
                .ui
                .read(cx)
                .layout
                .session_pane(&key)
                .map(|p| p.id.clone());
            if let Some(pane) = pane {
                self.ui.update(cx, |ui, cx| ui.close_pane(&pane, cx));
            }
            self.panes.remove(&key);
        }
        cx.notify();
    }

    fn on_sidebar(&mut self, event: SidebarEvent, cx: &mut Context<Self>) {
        self.dialogs.update(cx, |dialogs, cx| match event {
            SidebarEvent::ArchiveLane(id) => {
                dialogs.archive(id, None, panoptes_core::lanes::BranchRecovery::None, cx)
            }
            SidebarEvent::Adopt(session) => dialogs.adopt(*session, None, cx),
            SidebarEvent::RemoveProject(id) => dialogs.remove_project(id, cx),
            SidebarEvent::DeleteLane(id) => dialogs.delete_lane(id, cx),
        });
    }

    fn pane(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<ChatPane> {
        if let Some(pane) = self.panes.get(key) {
            return pane.clone();
        }
        let (store, ui) = (self.store.clone(), self.ui.clone());
        let pane = cx.new(|cx| ChatPane::new(key.to_string(), store, ui, window, cx));
        self.panes.insert(key.to_string(), pane.clone());
        pane
    }

    fn render_node(
        &mut self,
        node: &PaneNode,
        multiple: bool,
        p: &Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            PaneNode::Split {
                direction,
                first,
                second,
                ..
            } => {
                let vertical = *direction == crate::model::panes::SplitDirection::Vertical;
                let first = self.render_node(first, multiple, p, window, cx);
                let second = self.render_node(second, multiple, p, window, cx);
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(vertical, |el| el.flex_row())
                    .when(!vertical, |el| el.flex_col())
                    .child(div().flex().flex_1().min_w_0().min_h_0().child(first))
                    .child(
                        div()
                            .flex_none()
                            .bg(p.line)
                            .when(vertical, |el| el.w(px(1.)))
                            .when(!vertical, |el| el.h(px(1.))),
                    )
                    .child(div().flex().flex_1().min_w_0().min_h_0().child(second))
                    .into_any_element()
            }
            PaneNode::Leaf(leaf) => {
                let pane_id = leaf.id.clone();
                let ui = self.ui.clone();
                let store = self.store.clone();
                let activate = move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
                    ui.update(cx, |ui, cx| ui.focus_pane(&pane_id, cx));
                    let key = ui
                        .read(cx)
                        .layout
                        .find(&pane_id)
                        .and_then(|p| p.session_key.clone());
                    store.update(cx, |store, cx| {
                        if store.state.active_key != key {
                            store.state.active_key = key;
                            cx.notify();
                        }
                    });
                };
                let content = match &leaf.session_key {
                    Some(key) => self.pane(key, window, cx).into_any_element(),
                    None => self.empty_pane(&leaf.id, multiple, p),
                };
                let zone = self
                    .drop_target
                    .as_ref()
                    .filter(|(pane, _)| pane == &leaf.id && cx.has_active_drag())
                    .map(|(_, zone)| *zone);
                let this = cx.entity().downgrade();
                let (moving, dropping) = (this.clone(), this);
                let (move_pane, drop_pane) = (leaf.id.clone(), leaf.id.clone());
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .capture_any_mouse_down(activate)
                    .on_drag_move::<DraggedSession>(move |event, _, cx| {
                        let bounds = event.bounds;
                        let at = event.event.position;
                        let target = bounds.contains(&at).then(|| {
                            let zone = drop_zone_at(
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                                f32::from(at.x - bounds.origin.x),
                                f32::from(at.y - bounds.origin.y),
                            );
                            (move_pane.clone(), zone)
                        });
                        let _ = moving.update(cx, |this, cx| {
                            let mine = this
                                .drop_target
                                .as_ref()
                                .is_some_and(|(pane, _)| pane == &move_pane);
                            if target.is_some() && this.drop_target != target {
                                this.drop_target = target;
                                cx.notify();
                            } else if target.is_none() && mine {
                                this.drop_target = None;
                                cx.notify();
                            }
                        });
                    })
                    .on_drop(move |dragged: &DraggedSession, _, cx| {
                        let key = dragged.key.clone();
                        let pane = drop_pane.clone();
                        let _ = dropping.update(cx, |this, cx| this.drop_session(key, &pane, cx));
                    })
                    .child(content)
                    .children(zone.map(|zone| drop_indicator(zone, leaf.session_key.is_none(), p)))
                    .into_any_element()
            }
        }
    }

    fn empty_pane(&self, pane_id: &str, can_close: bool, p: &Palette) -> AnyElement {
        let ui = self.ui.clone();
        let pick = pane_id.to_string();
        let close = pane_id.to_string();
        let close_ui = self.ui.clone();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .bg(p.bg)
            .when(can_close, |el| {
                el.child(
                    div().absolute().top(px(8.)).right(px(8.)).child(
                        Button::new("close-empty-pane")
                            .ghost()
                            .small()
                            .icon("x")
                            .tooltip("Close pane")
                            .on_click(move |_, _, cx| {
                                close_ui.update(cx, |ui, cx| ui.close_pane(&close, cx))
                            }),
                    ),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!("empty-{pane_id}")))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .text_center()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        ui.update(cx, |ui, cx| {
                            ui.open(
                                Overlay::PaneSession {
                                    pane_id: pick.clone(),
                                },
                                cx,
                            )
                        })
                    })
                    .child(loop_mark("empty-mark", 32., LoopState::Idle, p.ink_faint))
                    .child(
                        div()
                            .child(
                                div()
                                    .t_heading()
                                    .text_color(p.ink)
                                    .child("No session selected"),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .t_body()
                                    .text_color(p.ink_muted)
                                    .child("Choose a session from the sidebar, or press ⌘P."),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let state = &self.store.read(cx).state;
        if state.startup != Startup::Ready {
            return startup::startup_gate(&state.startup, self.show_loader, &self.store, &p)
                .into_any_element();
        }
        let error = state.error.clone();
        let backend = self.store.read(cx).backend();
        let warning = backend
            .harness_errors
            .first()
            .cloned()
            .or_else(|| backend.unsupported.clone());
        let first_project = state.projects.first().map_or(0, |p| p.id);
        let active_lane = state
            .sessions
            .iter()
            .find(|s| Some(&s.key) == state.active_key.as_ref())
            .and_then(|s| s.lane_id);
        let layout = self.ui.read(cx).layout.clone();
        let multiple = layout.leaves().len() > 1;
        let panes = self.render_node(&layout, multiple, &p, window, cx);
        // The lane review sits beside the panes, or takes their place in a
        // narrow window.
        let review_open = self.ui.read(cx).review_open;
        let review_narrow = window.viewport_size().width < px(1100.);
        let conversation = if review_open && review_narrow {
            self.review.clone().into_any_element()
        } else {
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(panes)
                .when(review_open, |el| {
                    el.child(div().w(px(1.)).bg(p.line)).child(
                        div()
                            .w(px(560.))
                            .flex_none()
                            .min_h_0()
                            .child(self.review.clone()),
                    )
                })
                .into_any_element()
        };
        let narrow = window.viewport_size().width < px(1280.);
        let content: AnyElement = match self.diagnostics.as_ref().map(|(_, view)| view.clone()) {
            // Diagnostics sits beside the conversation rather than taking its
            // place: a question the agent asked must not be behind a pane. A
            // narrow window gets a narrower panel, still resizable.
            Some(diagnostics) => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    h_resizable("conversation-diagnostics")
                        .child(resizable_panel().child(conversation))
                        .child(
                            resizable_panel()
                                .size(if narrow { px(520.) } else { px(760.) })
                                .flex_none()
                                .child(diagnostics),
                        ),
                )
                .into_any_element(),
            None => div()
                .flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(conversation)
                .into_any_element(),
        };
        let store = self.store.clone();
        fn open<A: Action>(
            ui: &Entity<Ui>,
            overlay: Overlay,
        ) -> impl Fn(&A, &mut Window, &mut App) + use<A> {
            let ui = ui.clone();
            move |_, _, cx| {
                // An open overlay keeps the keyboard.
                if ui.read(cx).overlay.is_none() {
                    ui.update(cx, |ui, cx| ui.open(overlay.clone(), cx))
                }
            }
        }
        fn split<A: Action>(
            store: &Entity<AppStore>,
            ui: &Entity<Ui>,
            direction: SplitDirection,
        ) -> impl Fn(&A, &mut Window, &mut App) + use<A> {
            let (store, ui) = (store.clone(), ui.clone());
            move |_, _, cx| {
                if ui.read(cx).overlay.is_none() {
                    split_focused_pane(&store, &ui, direction, cx);
                }
            }
        }
        let ui = &self.ui;
        let (lane_store, project_store) = (self.store.clone(), self.store.clone());
        let (lane_ui, project_ui) = (self.ui.clone(), self.ui.clone());
        let (zoom_in_store, zoom_out_store, zoom_reset_store) =
            (self.store.clone(), self.store.clone(), self.store.clone());
        let (review_store, review_ui) = (self.store.clone(), self.ui.clone());
        div()
            .key_context(keys::CONTEXT)
            .track_focus(&self.focus)
            .on_action(open::<Search>(ui, Overlay::Search))
            .on_action(open::<ShowShortcuts>(ui, Overlay::Shortcuts))
            .on_action(open::<NewLaneAction>(
                ui,
                Overlay::NewLane {
                    project_id: first_project,
                    prompt: None,
                },
            ))
            .on_action(move |_: &NewLaneSession, _, cx| {
                if let Some(lane_id) = active_lane
                    && lane_ui.read(cx).overlay.is_none()
                {
                    lane_store.update(cx, |store, cx| store.new_lane_session(lane_id, cx).detach());
                }
            })
            .on_action(move |_: &ToggleReview, _, cx| {
                if review_ui.read(cx).overlay.is_none() {
                    crate::ui::toggle_review(&review_store, &review_ui, active_lane, cx);
                }
            })
            .on_action(cx.listener(|this, _: &JumpToLatest, _, cx| {
                let ui = this.ui.read(cx);
                let key = ui
                    .layout
                    .find(&ui.focused_pane)
                    .and_then(|leaf| leaf.session_key.clone());
                if let Some(pane) = key.and_then(|key| this.panes.get(&key)) {
                    pane.update(cx, |pane, cx| pane.scroll_to_end(cx));
                }
            }))
            .on_action(move |_: &ZoomIn, window, cx| {
                zoom_by(&zoom_in_store, ZoomCommand::In, window, cx)
            })
            .on_action(move |_: &ZoomOut, window, cx| {
                zoom_by(&zoom_out_store, ZoomCommand::Out, window, cx)
            })
            .on_action(move |_: &ZoomReset, window, cx| {
                zoom_by(&zoom_reset_store, ZoomCommand::Reset, window, cx)
            })
            .on_action(move |_: &AddProject, _, cx| {
                if project_ui.read(cx).overlay.is_none() {
                    project_store.update(cx, |store, cx| store.pick_project(cx))
                }
            })
            .on_action(split::<SplitVertical>(
                &self.store,
                ui,
                SplitDirection::Vertical,
            ))
            .on_action(split::<SplitHorizontal>(
                &self.store,
                ui,
                SplitDirection::Horizontal,
            ))
            // The app-menu keys. Quit runs the on_app_quit cleanup in main;
            // closing the last window quits through the app's quit mode.
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &HideApp, _, cx| cx.hide())
            .on_action(|_: &HideOthers, _, cx| cx.hide_other_apps())
            .on_action(|_: &ShowAll, _, cx| cx.unhide_other_apps())
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(p.bg)
            .text_color(p.ink)
            .font_family(crate::theme::FONT_SANS)
            .t_body()
            .children(warning.map(|warning| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(p.line)
                    .bg(p.waiting_soft)
                    .px(px(16.))
                    .py(px(4.))
                    .text_color(p.waiting)
                    .child(icon("circle-alert", 14.))
                    .child(warning)
            }))
            .children(error.map(|error| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(p.line)
                    .bg(p.error_soft)
                    .px(px(16.))
                    .py(px(4.))
                    .text_color(p.error)
                    .child(icon("circle-alert", 14.))
                    .child(div().flex_1().child(error))
                    .child(
                        Button::new("dismiss-error")
                            .ghost()
                            .small()
                            .label("Dismiss")
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| store.dismiss_error(cx))
                            }),
                    )
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar.clone())
                    .child(content),
            )
            .child(self.dialogs.clone())
            .children(self.overlay.as_ref().map(|(_, view)| view.clone()))
            .children(self.image_zoom.as_ref().map(|(_, view)| view.clone()))
            .into_any_element()
    }
}

/// Where a dragged session will land, drawn over the pane.
fn drop_indicator(zone: DropZone, empty: bool, p: &Palette) -> Div {
    let label = if empty {
        "Drop session here".to_string()
    } else {
        match zone {
            DropZone::Center => "Drop session in this pane".to_string(),
            DropZone::Left => "Drop to split left".to_string(),
            DropZone::Right => "Drop to split right".to_string(),
            DropZone::Top => "Drop to split top".to_string(),
            DropZone::Bottom => "Drop to split bottom".to_string(),
        }
    };
    let inset = px(12.);
    let half = relative(0.5);
    let base = div().absolute();
    let placed = match zone {
        DropZone::Left => base.top(inset).bottom(inset).left(inset).right(half),
        DropZone::Right => base.top(inset).bottom(inset).left(half).right(inset),
        DropZone::Top => base.left(inset).right(inset).top(inset).bottom(half),
        DropZone::Bottom => base.left(inset).right(inset).top(half).bottom(inset),
        DropZone::Center => base.top(inset).bottom(inset).left(inset).right(inset),
    };
    placed
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(crate::ds::RADIUS_MD))
        .border_1()
        .border_color(p.focus)
        .bg(p.selection.opacity(0.8))
        .t_body_strong()
        .text_color(p.ink)
        .child(icon(
            if zone == DropZone::Center {
                "session"
            } else {
                "plus"
            },
            16.,
        ))
        .child(label)
}

/// Zooms the whole window and saves the font-size preference.
fn zoom_by(store: &Entity<AppStore>, command: ZoomCommand, window: &mut Window, cx: &mut App) {
    if let Some(percent) = crate::zoom::change(command, window, cx) {
        store.update(cx, |store, cx| store.set_font_size(percent, cx));
    }
}
