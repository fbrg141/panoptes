//! The palettes that jump somewhere: ⌘P search over projects, lanes, sessions
//! and a few actions, and the session picker that fills an empty or split pane.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds;
use crate::model::palette::{
    PaletteGroup, PaletteTarget, palette_groups, palette_target, session_picker_groups,
};
use crate::store::AppStore;
use crate::theme::PaletteExt as _;
use crate::ui::{Overlay, Ui, request_open};
use crate::views::palette::{CommandPalette, PaletteEvent};

/// Which palette this is; a pane picker fills its pane.
#[derive(Clone, PartialEq)]
pub enum JumpKind {
    Search,
    PaneSession(String),
}

pub struct Jump {
    kind: JumpKind,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    palette: Entity<CommandPalette>,
}

fn groups(kind: &JumpKind, store: &AppStore) -> Vec<PaletteGroup> {
    let state = &store.state;
    match kind {
        JumpKind::Search => palette_groups(
            &state.sessions,
            &state.projects,
            &state.lanes,
            state.active_key.as_deref(),
        ),
        JumpKind::PaneSession(_) => session_picker_groups(&state.sessions, &state.projects),
    }
}

impl Jump {
    pub fn new(
        kind: JumpKind,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match kind {
            JumpKind::Search => "Jump to a session, lane or project",
            JumpKind::PaneSession(_) => "Select a session for this pane",
        };
        let list = groups(&kind, store.read(cx));
        let palette = cx.new(|cx| CommandPalette::new(placeholder, list, window, cx));
        cx.subscribe(&palette, |this, _, event, cx| {
            let PaletteEvent::Selected(id) = event;
            this.select(id, cx);
        })
        .detach();
        cx.observe(&store, |this, store, cx| {
            let list = groups(&this.kind, store.read(cx));
            this.palette
                .update(cx, |palette, cx| palette.set_groups(list, cx));
        })
        .detach();
        Self {
            kind,
            store,
            ui,
            palette,
        }
    }

    fn show(&self, overlay: Overlay, cx: &mut App) {
        self.ui.update(cx, |ui, cx| ui.open(overlay, cx));
    }

    fn first_project(&self, cx: &App) -> i64 {
        self.store
            .read(cx)
            .state
            .projects
            .first()
            .map_or(0, |p| p.id)
    }

    /// The lane of the session on screen.
    fn active_lane(&self, cx: &App) -> Option<i64> {
        let state = &self.store.read(cx).state;
        state
            .sessions
            .iter()
            .find(|s| Some(&s.key) == state.active_key.as_ref())
            .and_then(|s| s.lane_id)
    }

    fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ui.update(cx, |ui, cx| ui.close(cx));
        let (store, ui) = (self.store.clone(), self.ui.clone());
        if let JumpKind::PaneSession(pane) = &self.kind {
            if id == "new" {
                let project_id = self.first_project(cx);
                self.show(
                    Overlay::NewLane {
                        project_id,
                        prompt: None,
                    },
                    cx,
                );
                return;
            }
            let Some(key) = id.strip_prefix("session:") else {
                return;
            };
            let session = store
                .read(cx)
                .state
                .sessions
                .iter()
                .find(|s| s.key == key)
                .cloned();
            if let Some(session) = session {
                request_open(&store, &ui, &session, Some(pane), cx);
            }
            return;
        }
        let state = &store.read(cx).state;
        let Some(target) = palette_target(id, &state.sessions, &state.projects, &state.lanes)
        else {
            return;
        };
        match target {
            PaletteTarget::Session(session) => request_open(&store, &ui, &session, None, cx),
            PaletteTarget::Lane(lane) => {
                let session = state
                    .sessions
                    .iter()
                    .find(|s| s.lane_id == Some(lane.id))
                    .cloned();
                match session {
                    Some(session) => request_open(&store, &ui, &session, None, cx),
                    None => store.update(cx, |store, cx| {
                        store.create(lane.id, Default::default(), cx).detach()
                    }),
                }
            }
            PaletteTarget::Project(project) => self.show(
                Overlay::NewLane {
                    project_id: project.id,
                    prompt: None,
                },
                cx,
            ),
            PaletteTarget::NewSession => {
                let project_id = self.first_project(cx);
                self.show(
                    Overlay::NewLane {
                        project_id,
                        prompt: None,
                    },
                    cx,
                );
            }
            PaletteTarget::NewLaneSession => {
                if let Some(lane_id) = self.active_lane(cx) {
                    store.update(cx, |store, cx| store.new_lane_session(lane_id, cx).detach());
                }
            }
            PaletteTarget::ResumeSession => {
                if let Some(lane_id) = self.active_lane(cx) {
                    self.show(
                        Overlay::ResumeSession {
                            lane_id,
                            session_id: None,
                        },
                        cx,
                    );
                }
            }
            PaletteTarget::Review => {
                crate::ui::toggle_review(&store, &ui, self.active_lane(cx), cx);
            }
            PaletteTarget::NewProject => store.update(cx, |store, cx| store.pick_project(cx)),
            PaletteTarget::Shortcuts => self.show(Overlay::Shortcuts, cx),
            PaletteTarget::ShowLogs => crate::views::diagnostics::open(self.store.clone(), cx),
        }
    }
}

impl Render for Jump {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let ui = self.ui.clone();
        ds::modal_layer(
            window,
            move |_, cx| {
                ui.update(cx, |ui, cx| ui.close(cx));
                true
            },
            self.palette.clone(),
            &p,
        )
    }
}
