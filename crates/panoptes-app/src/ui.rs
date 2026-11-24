//! Window state that is not the backend's: the one overlay shown at a time and
//! the pane layout.

use gpui_kit::*;

use panoptes_core::service::Session;

use crate::model::diagnostics::DiagnosticsFilter;
use crate::model::panes::{DropZone, PaneNode, SplitDirection, single_pane};
use crate::model::sidebar::lane_session_to_stop;
use crate::store::AppStore;
use crate::views::turn::ZoomedImage;

/// The one overlay the window shows at a time. A second one replaces the
/// first instead of stacking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    Search,
    Shortcuts,
    PaneSession {
        pane_id: String,
    },
    Settings,
    Cleanup {
        project_id: i64,
    },
    NewLane {
        project_id: i64,
        prompt: Option<String>,
    },
    ResumeSession {
        lane_id: i64,
        session_id: Option<i64>,
    },
}

pub struct Ui {
    pub overlay: Option<Overlay>,
    /// An image clicked in the transcript, shown over the window until it is
    /// closed. Not an [`Overlay`]: those are mutually exclusive, and looking
    /// at a picture must not close the overlay you came from.
    pub image: Option<ZoomedImage>,
    pub layout: PaneNode,
    pub focused_pane: String,
    /// The diagnostics pane and its initial project/lane scope.
    pub diagnostics: Option<DiagnosticsFilter>,
}

impl Default for Ui {
    fn default() -> Self {
        let layout = single_pane(None);
        let focused_pane = layout.leaves()[0].id.clone();
        Self {
            overlay: None,
            image: None,
            layout,
            focused_pane,
            diagnostics: None,
        }
    }
}

impl Ui {
    pub fn open(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        self.overlay = Some(overlay);
        cx.notify();
    }

    pub fn show_image(&mut self, image: ZoomedImage, cx: &mut Context<Self>) {
        self.image = Some(image);
        cx.notify();
    }

    pub fn close_image(&mut self, cx: &mut Context<Self>) {
        self.image = None;
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        cx.notify();
    }

    pub fn open_diagnostics(&mut self, filter: DiagnosticsFilter, cx: &mut Context<Self>) {
        self.overlay = None;
        self.diagnostics = Some(filter);
        cx.notify();
    }

    pub fn close_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics.take().is_some() {
            cx.notify();
        }
    }

    /// Records the scope the pane is showing, so the pane's own pickers and a
    /// later request for that scope agree on where it is.
    pub fn set_diagnostics_filter(&mut self, filter: DiagnosticsFilter, cx: &mut Context<Self>) {
        if self.diagnostics != Some(filter) {
            self.diagnostics = Some(filter);
            cx.notify();
        }
    }

    pub fn focus_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        if self.layout.find(pane_id).is_some() && self.focused_pane != pane_id {
            self.focused_pane = pane_id.to_string();
            cx.notify();
        }
    }

    pub fn split_focused(
        &mut self,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (layout, new_pane) = self.layout.split(&self.focused_pane, direction);
        let new_pane = new_pane?;
        self.layout = layout;
        self.focused_pane = new_pane.clone();
        self.overlay = Some(Overlay::PaneSession {
            pane_id: new_pane.clone(),
        });
        cx.notify();
        Some(new_pane)
    }

    pub fn show_session(
        &mut self,
        session_key: &str,
        pane_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let pane_id = pane_id.unwrap_or(&self.focused_pane).to_string();
        let placed = self.layout.place(&pane_id, session_key);
        self.layout = placed.layout;
        self.focused_pane = placed.focused;
        cx.notify();
    }

    pub fn close_pane(&mut self, pane_id: &str, cx: &mut Context<Self>) {
        self.layout = self.layout.close(pane_id);
        if self.layout.find(&self.focused_pane).is_none() {
            self.focused_pane = self
                .layout
                .leaves()
                .first()
                .map(|p| p.id.clone())
                .unwrap_or_else(|| pane_id.to_string());
        }
        cx.notify();
    }

    pub fn drop_session(
        &mut self,
        session_key: &str,
        pane_id: &str,
        zone: DropZone,
        cx: &mut Context<Self>,
    ) {
        let placed = self.layout.drop_session(session_key, pane_id, zone);
        self.layout = placed.layout;
        self.focused_pane = placed.focused;
        cx.notify();
    }
}

/// Shows a session and opens it. An earlier lane session opens as a read-only
/// preview; sending its next message stops the live session and resumes it.
pub fn request_open(
    store: &Entity<AppStore>,
    ui: &Entity<Ui>,
    session: &Session,
    pane_id: Option<&str>,
    cx: &mut App,
) {
    let preview = lane_session_to_stop(session, &store.read(cx).state.sessions).is_some();
    ui.update(cx, |ui, cx| ui.show_session(&session.key, pane_id, cx));
    store.update(cx, |store, cx| {
        if preview {
            store.preview(session, cx).detach();
        } else {
            store.open(session, cx).detach();
        }
    });
}

/// A session being dragged from the sidebar or a pane header onto a pane.
/// The drag draws nothing under the pointer; the pane shows where it lands.
#[derive(Clone)]
pub struct DraggedSession {
    pub key: String,
}

impl Render for DraggedSession {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Splits the focused pane; the new empty pane takes focus, so no session
/// is active until one is picked for it.
pub fn split_focused_pane(
    store: &Entity<AppStore>,
    ui: &Entity<Ui>,
    direction: SplitDirection,
    cx: &mut App,
) {
    let split = ui.update(cx, |ui, cx| ui.split_focused(direction, cx));
    if split.is_some() {
        store.update(cx, |store, cx| {
            store.state.active_key = None;
            cx.notify();
        });
    }
}
