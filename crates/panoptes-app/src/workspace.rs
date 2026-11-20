//! The window: banners, the sidebar and the tiled session panes. Port of
//! `App.tsx` and `SessionWorkspace.tsx`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::service::SessionEvent;
use tokio::sync::mpsc;

use crate::backend::Backend;
use crate::ds::{Button, LoopState, Type as _, icon, loop_mark};
use crate::model::panes::PaneNode;
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::{Overlay, Ui};
use crate::views::chat::ChatPane;
use crate::views::lane_dialogs::LaneDialogs;
use crate::views::sidebar::{Sidebar, SidebarEvent};

pub struct Workspace {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    sidebar: Entity<Sidebar>,
    dialogs: Entity<LaneDialogs>,
    panes: HashMap<String, Entity<ChatPane>>,
    /// Open conversations when last seen, to close the panes of ones that went.
    open_keys: HashSet<String>,
    active_key: Option<String>,
}

impl Workspace {
    pub fn new(
        backend: Arc<Backend>,
        mut events: mpsc::UnboundedReceiver<SessionEvent>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = cx.new(|cx| AppStore::new(backend, cx));
        let ui = cx.new(|_| Ui::default());
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), ui.clone(), cx));
        let dialogs = cx.new(|_| LaneDialogs::new(store.clone()));
        cx.observe(&dialogs, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&sidebar, |this, _, event, cx| {
            this.on_sidebar(event.clone(), cx)
        })
        .detach();
        cx.observe(&store, |this, _, cx| this.sync_panes(cx))
            .detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();

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

        Self {
            store,
            ui,
            sidebar,
            dialogs,
            panes: HashMap::new(),
            open_keys: HashSet::new(),
            active_key: None,
        }
    }

    /// Creating a session goes through the store's active key: show it in
    /// the focused pane. A conversation that was forgotten closes its pane.
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let state = &self.store.read(cx).state;
        let active = state.active_key.clone();
        let open: HashSet<String> = state.conversations.keys().cloned().collect();
        if active != self.active_key {
            self.active_key = active.clone();
            if let Some(key) = &active {
                self.ui.update(cx, |ui, cx| ui.show_session(key, None, cx));
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
            SidebarEvent::ArchiveLane(id) => dialogs.archive(id, None, false, cx),
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
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .capture_any_mouse_down(activate)
                    .child(content)
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
                    .gap(px(16.))
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
                    .child(loop_mark("empty-mark", 64., LoopState::Idle, p.ink))
                    .child(
                        div()
                            .child(
                                div()
                                    .t_title()
                                    .text_color(p.ink)
                                    .child("No session selected"),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .t_body()
                                    .text_color(p.ink_muted)
                                    .child("Click to choose a session for this pane."),
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
        let error = state.error.clone();
        let warning = self
            .store
            .read(cx)
            .backend()
            .harness_errors
            .first()
            .cloned();
        let layout = self.ui.read(cx).layout.clone();
        let multiple = layout.leaves().len() > 1;
        let panes = self.render_node(&layout, multiple, &p, window, cx);
        let store = self.store.clone();
        div()
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
                    .child(div().flex().flex_1().min_w_0().min_h_0().child(panes)),
            )
            .child(self.dialogs.clone())
    }
}
