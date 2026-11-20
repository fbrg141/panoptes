//! A lane-owned history picker; it never scans the harness's shared cwd.
//! With a session id it only confirms resuming that record, since doing so
//! stops the lane's live session. Port of `ResumeSessionDialog.tsx`.

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::service::Session;

use crate::ds::{self, Button, Type as _};
use crate::model::sidebar::lane_resume_sessions;
use crate::store::AppStore;
use crate::theme::PaletteExt as _;
use crate::ui::Ui;
use crate::views::palette::{CommandPalette, PaletteEvent, PaletteGroup, PaletteItem};

pub struct ResumeSession {
    lane_id: i64,
    session_id: Option<i64>,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    palette: Option<Entity<CommandPalette>>,
    busy: bool,
}

fn groups(sessions: &[Session], lane_id: i64) -> Vec<PaletteGroup> {
    let own: Vec<Session> = sessions
        .iter()
        .filter(|s| s.lane_id == Some(lane_id))
        .cloned()
        .collect();
    vec![PaletteGroup {
        label: "Earlier sessions".into(),
        items: lane_resume_sessions(&own)
            .into_iter()
            .filter_map(|s| {
                Some(PaletteItem {
                    id: s.id?.to_string(),
                    label: s.title,
                    hint: Some(format!("{} messages", s.message_count)),
                    icon: Some("history"),
                    kbd: Vec::new(),
                })
            })
            .collect(),
    }]
}

impl ResumeSession {
    pub fn new(
        lane_id: i64,
        session_id: Option<i64>,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette = session_id.is_none().then(|| {
            let list = groups(&store.read(cx).state.sessions, lane_id);
            let palette =
                cx.new(|cx| CommandPalette::new("Resume an earlier session", list, window, cx));
            cx.subscribe(&palette, |this, _, event, cx| {
                if let PaletteEvent::Selected(id) = event
                    && let Ok(id) = id.parse()
                {
                    this.resume(id, cx);
                }
            })
            .detach();
            palette
        });
        cx.observe(&store, move |this, store, cx| {
            if let Some(palette) = &this.palette {
                let list = groups(&store.read(cx).state.sessions, this.lane_id);
                palette.update(cx, |palette, cx| palette.set_groups(list, cx));
            }
            cx.notify();
        })
        .detach();
        Self {
            lane_id,
            session_id,
            store,
            ui,
            palette,
            busy: false,
        }
    }

    fn resume(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let lane_id = self.lane_id;
        let task = self
            .store
            .update(cx, |store, cx| store.resume_lane_session(lane_id, id, cx));
        cx.spawn(async move |this, cx| {
            let resumed = task.await == Some(true);
            let _ = this.update(cx, |this, cx| {
                if resumed {
                    this.ui.update(cx, |ui, cx| ui.close(cx));
                } else {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn close(&self, cx: &mut App) -> bool {
        if self.busy {
            return false;
        }
        self.ui.update(cx, |ui, cx| ui.close(cx));
        true
    }
}

impl Render for ResumeSession {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let this = cx.entity().downgrade();
        let close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.close(cx)).unwrap_or(false)
            }
        };
        if let Some(palette) = &self.palette {
            return ds::modal_layer(window, close, palette.clone(), &p).into_any_element();
        }
        let session_id = self.session_id.unwrap_or_default();
        let title = self
            .store
            .read(cx)
            .state
            .sessions
            .iter()
            .find(|s| s.id == Some(session_id))
            .map_or("this session".to_string(), |s| s.title.clone());
        let busy = self.busy;
        let (cancel, resume) = (this.clone(), this.clone());
        let actions = div()
            .flex()
            .gap(px(8.))
            .child(
                Button::new("resume-cancel")
                    .label("Cancel")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = cancel.update(cx, |this, cx| this.close(cx));
                    }),
            )
            .child(
                Button::new("resume-confirm")
                    .label("Resume")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = resume.update(cx, |this, cx| this.resume(session_id, cx));
                    }),
            )
            .into_any_element();
        ds::dialog(
            format!("Resume {title}?"),
            440.,
            div().t_body().child(
                "Only one session runs in a lane. Resuming this one stops the current session; \
                 its conversation stays in the lane's history.",
            ),
            Some(actions),
            move |window, cx| {
                close(window, cx);
            },
            &p,
        )
        .into_any_element()
    }
}
