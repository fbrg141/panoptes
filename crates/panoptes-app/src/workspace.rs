//! The window's one view: the lane list and the open session.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::service::{
    Listing, OpenedSession, ServiceError, Session, SessionEvent, SessionUpdate,
};
use panoptes_core::store::LaneState;
use panoptes_harness::{HarnessEvent, PromptMode, Role, SessionStatus};
use tokio::sync::mpsc;

use crate::backend::Backend;
use crate::model::conversation::Conversation;
use crate::model::format::now_ms;

pub struct Workspace {
    backend: Arc<Backend>,
    listing: Option<Listing>,
    statuses: HashMap<String, SessionStatus>,
    current: Option<Current>,
    composer: Entity<TextareaState>,
    transcript_scroll: ScrollHandle,
    error: Option<String>,
}

struct Current {
    key: String,
    title: String,
    conversation: Conversation,
}

impl Workspace {
    pub fn new(
        backend: Arc<Backend>,
        mut events: mpsc::UnboundedReceiver<SessionEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message the agent")
                .auto_grow(1, 8)
        });
        cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.send(window, cx);
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_session_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let error = (!backend.harness_errors.is_empty()).then(|| backend.harness_errors.join("\n"));
        let mut workspace = Self {
            backend,
            listing: None,
            statuses: HashMap::new(),
            current: None,
            composer,
            transcript_scroll: ScrollHandle::new(),
            error,
        };
        workspace.refresh(cx);
        workspace
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let listing = self
            .backend
            .call(|service| async move { service.list().await });
        cx.spawn(async move |this, cx| {
            let listing = listing.await;
            this.update(cx, |this, cx| {
                match listing {
                    Ok(listing) => this.listing = Some(listing),
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn open(&mut self, session: Session, cx: &mut Context<Self>) {
        let title = session.title.clone();
        let opened = self.backend.call(move |service| async move {
            let locator = session
                .harness_ref
                .map(|reference| reference.locator)
                .unwrap_or_default();
            service
                .open(&session.harness, &session.key, locator, session.cwd)
                .await
        });
        self.show_opened(title, opened, cx);
    }

    fn start_session(&mut self, lane_id: i64, title: String, cx: &mut Context<Self>) {
        let opened = self
            .backend
            .call(move |service| async move { service.create_with(lane_id, "pi", None).await });
        self.show_opened(title, opened, cx);
    }

    fn show_opened(
        &mut self,
        title: String,
        opened: impl Future<Output = anyhow::Result<Result<OpenedSession, ServiceError>>> + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let opened = flatten(opened.await);
            this.update(cx, |this, cx| {
                match opened {
                    Ok(opened) => {
                        let mut conversation = Conversation::new(opened.history);
                        for event in &opened.replay {
                            conversation.apply(event, now_ms());
                        }
                        this.statuses.insert(opened.key.clone(), opened.status);
                        this.current = Some(Current {
                            key: opened.key,
                            title,
                            conversation,
                        });
                        this.error = None;
                        this.transcript_scroll.scroll_to_bottom();
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
        })
        .detach();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = &self.current else {
            return;
        };
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let key = current.key.clone();
        let mode = match self.statuses.get(&key) {
            Some(SessionStatus::Running) => PromptMode::FollowUp,
            _ => PromptMode::Start,
        };
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        let sent = self
            .backend
            .call(move |service| async move { service.prompt(&key, text, Vec::new(), mode).await });
        cx.spawn(async move |this, cx| {
            if let Err(error) = flatten(sent.await) {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match &event.event {
            SessionUpdate::Status(changed) => {
                self.statuses.insert(event.key.clone(), changed.status);
            }
            SessionUpdate::Harness(harness_event) => {
                let is_current = self
                    .current
                    .as_ref()
                    .is_some_and(|current| current.key == event.key);
                match harness_event {
                    // The history is authoritative once a run settles.
                    HarnessEvent::RunSettled { .. } if is_current => {
                        if let Some(current) = &mut self.current {
                            current.conversation.apply(harness_event, now_ms());
                        }
                        self.reload_history(event.key.clone(), cx)
                    }
                    HarnessEvent::TitleChanged { .. } => self.refresh(cx),
                    _ if is_current => {
                        if let Some(current) = &mut self.current {
                            current.conversation.apply(harness_event, now_ms());
                        }
                        self.transcript_scroll.scroll_to_bottom();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn reload_history(&mut self, key: String, cx: &mut Context<Self>) {
        let for_key = key.clone();
        let history = self
            .backend
            .call(move |service| async move { service.history(&for_key).await });
        cx.spawn(async move |this, cx| {
            let history = flatten(history.await);
            this.update(cx, |this, cx| {
                match history {
                    Ok(history) => {
                        if let Some(current) = &mut this.current
                            && current.key == key
                        {
                            current.conversation.messages = history;
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
                cx.notify();
            })
        })
        .detach();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut list = v_flex().gap_1().p_2();
        if let Some(listing) = &self.listing {
            let mut projects: Vec<_> = listing.projects.iter().collect();
            projects.sort_by(|a, b| a.settings.name.cmp(&b.settings.name));
            for project in projects {
                list = list.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(project.settings.name.clone()),
                );
                for lane in listing
                    .lanes
                    .iter()
                    .filter(|lane| lane.project_id == project.id && lane.state == LaneState::Active)
                {
                    let sessions: Vec<Session> = listing
                        .navigation
                        .iter()
                        .find(|navigation| navigation.lane_id == lane.id)
                        .map(|navigation| navigation.sessions.clone())
                        .unwrap_or_default();
                    let lane_id = lane.id;
                    let lane_name = lane.name.clone();
                    let mut row = h_flex()
                        .justify_between()
                        .child(div().text_sm().child(lane.name.clone()));
                    if sessions.is_empty() {
                        row = row.child(
                            Button::new(SharedString::from(format!("start-{lane_id}")))
                                .ghost()
                                .label("Start")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.start_session(lane_id, lane_name.clone(), cx)
                                })),
                        );
                    }
                    list = list.child(row);
                    for session in sessions {
                        list = list.child(self.render_session_row(session, &theme, cx));
                    }
                }
            }
        }
        div()
            .id("sidebar")
            .w(px(280.))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(list)
    }

    fn render_session_row(
        &self,
        session: Session,
        theme: &gpui_kit::component::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .current
            .as_ref()
            .is_some_and(|current| current.key == session.key);
        let status = self
            .statuses
            .get(&session.key)
            .and_then(|status| status_word(*status));
        let key = session.key.clone();
        h_flex()
            .id(SharedString::from(key))
            .pl_3()
            .pr_2()
            .py_1()
            .rounded_md()
            .justify_between()
            .text_sm()
            .when(selected, |row| row.bg(theme.accent))
            .hover(|row| row.bg(theme.accent))
            .child(div().truncate().child(session.title.clone()))
            .children(status.map(|word| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(word)
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.open(session.clone(), cx)))
    }

    fn render_session(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(current) = &self.current else {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Choose a session");
        };
        let status = self
            .statuses
            .get(&current.key)
            .and_then(|status| status_word(*status));
        let entries = current.conversation.messages.iter().map(|entry| {
            let user = entry.role == Role::User;
            div()
                .px_4()
                .py_2()
                .when(user, |row| row.bg(theme.muted).rounded_md())
                .when(entry.role == Role::Other, |row| {
                    row.text_color(theme.muted_foreground)
                })
                .child(crate::model::turns::plain_text(entry))
        });
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(current.title.clone()),
                    )
                    .children(
                        status.map(|word| div().text_color(theme.muted_foreground).child(word)),
                    ),
            )
            .child(
                div()
                    .id("transcript")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.transcript_scroll)
                    .child(v_flex().gap_2().p_4().children(entries)),
            )
            .child(div().p_3().child(Textarea::new(&self.composer)))
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .children(self.error.clone().map(|error| {
                div()
                    .px_4()
                    .py_2()
                    .bg(theme.danger)
                    .text_color(theme.danger_foreground)
                    .child(error)
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_session(cx)),
            )
    }
}

/// The status words the UI shows; a session that has not run shows none.
fn status_word(status: SessionStatus) -> Option<&'static str> {
    match status {
        SessionStatus::Idle => None,
        SessionStatus::Running => Some("Running"),
        SessionStatus::Waiting => Some("Waiting"),
        SessionStatus::Failed => Some("Failed"),
        SessionStatus::Done => Some("Done"),
    }
}

fn flatten<T>(result: anyhow::Result<Result<T, ServiceError>>) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.message),
        Err(error) => Err(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `gpui_kit::*` brings GPUI's own `test` attribute into scope.
    #[::core::prelude::v1::test]
    fn idle_sessions_show_no_status_word() {
        assert_eq!(status_word(SessionStatus::Idle), None);
        assert_eq!(status_word(SessionStatus::Running), Some("Running"));
    }
}
