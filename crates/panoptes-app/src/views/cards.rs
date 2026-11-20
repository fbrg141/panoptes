//! The cards that wait on the user: a tool call to allow or reject, and a
//! question from the harness. Port of `ApprovalCard.tsx` and
//! `QuestionCard.tsx`.

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_harness::{Answer, Approval, Question, QuestionKind, SessionStatus};

use crate::ds::{Button, RADIUS_MD, RADIUS_SM, Type as _, input_frame, status_pill};
use crate::model::format::now_ms;
use crate::store::AppStore;
use crate::theme::Palette;

/// Whether a card's offer has run out.
pub fn expired(expires_at: Option<i64>) -> bool {
    expires_at.is_some_and(|at| at <= now_ms())
}

fn frame(p: &Palette) -> Div {
    div()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_raised)
        .shadow(p.shadow_pop())
}

fn answer(store: &Entity<AppStore>, key: &str, id: &str, answer: Answer, cx: &mut App) {
    let (id, key) = (id.to_string(), key.to_string());
    store.update(cx, |store, cx| store.answer(id, answer, Some(key), cx));
}

/// Allow or reject a tool call, shown at the call it is about. `pill` is off
/// where the call's own row already says Waiting.
pub fn approval_card(
    approval: &Approval,
    pill: bool,
    key: &str,
    store: &Entity<AppStore>,
    p: &Palette,
) -> AnyElement {
    if expired(approval.expires_at) {
        return div().into_any_element();
    }
    let decide = |allow: bool| {
        let (store, key, id) = (store.clone(), key.to_string(), approval.id.clone());
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            answer(&store, &key, &id, Answer::Decision { allow }, cx)
        }
    };
    frame(p)
        .p(px(12.))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .when(pill, |el| el.child(status_pill(SessionStatus::Waiting, p)))
                .child(div().flex_1().min_w_0().child(approval.title.clone())),
        )
        .children(
            approval
                .message
                .clone()
                .map(|message| div().mt(px(8.)).text_color(p.ink_muted).child(message)),
        )
        .child(
            div()
                .mt(px(12.))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .child(
                    Button::new(SharedString::from(format!("reject-{}", approval.id)))
                        .label("Reject")
                        .on_click(decide(false)),
                )
                .child(
                    Button::new(SharedString::from(format!("allow-{}", approval.id)))
                        .primary()
                        .label("Allow")
                        .on_click(decide(true)),
                ),
        )
        .into_any_element()
}

/// The text field a question asks for, kept between frames.
#[derive(Clone)]
pub enum QuestionField {
    Line(Entity<InputState>),
    Text(Entity<TextareaState>),
}

impl QuestionField {
    pub fn new(question: &Question, window: &mut Window, cx: &mut App) -> Option<Self> {
        match &question.kind {
            QuestionKind::Input { placeholder } => {
                let placeholder = placeholder.clone().unwrap_or_default();
                Some(Self::Line(cx.new(|cx| {
                    InputState::new(window, cx).placeholder(placeholder)
                })))
            }
            QuestionKind::Editor { prefill } => {
                let prefill = prefill.clone().unwrap_or_default();
                Some(Self::Text(cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx).auto_grow(5, 16);
                    state.set_value(prefill, window, cx);
                    state
                })))
            }
            _ => None,
        }
    }

    pub fn value(&self, cx: &App) -> String {
        match self {
            Self::Line(state) => state.read(cx).value().to_string(),
            Self::Text(state) => state.read(cx).value().to_string(),
        }
    }

    fn focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            Self::Line(state) => state.read(cx).focus_handle(cx).is_focused(window),
            Self::Text(state) => state.read(cx).focus_handle(cx).is_focused(window),
        }
    }
}

/// A question from the harness, shown until answered or until it expires.
pub fn question_card(
    question: &Question,
    field: Option<&QuestionField>,
    key: &str,
    store: &Entity<AppStore>,
    p: &Palette,
    window: &Window,
    cx: &App,
) -> AnyElement {
    if expired(question.expires_at) {
        return div().into_any_element();
    }
    let send = |answer_with: Answer| {
        let (store, key, id) = (store.clone(), key.to_string(), question.id.clone());
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            answer(&store, &key, &id, answer_with.clone(), cx)
        }
    };
    let id = |name: &str| SharedString::from(format!("{name}-{}", question.id));
    let field_element = field.map(|field| {
        let focused = field.focused(window, cx);
        match field {
            QuestionField::Line(state) => input_frame(focused, false, p)
                .mt(px(12.))
                .child(Input::new(state).appearance(false).t_body().px(px(0.)))
                .into_any_element(),
            QuestionField::Text(state) => div()
                .mt(px(12.))
                .rounded(px(RADIUS_SM))
                .border_1()
                .border_color(if focused { p.focus } else { p.line_strong })
                .bg(p.surface_sunken)
                .p(px(8.))
                .child(Textarea::new(state).appearance(false).t_code())
                .into_any_element(),
        }
    });
    let mut actions = div()
        .mt(px(12.))
        .pt(px(12.))
        .border_t_1()
        .border_color(p.line)
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .child(
            Button::new(id("dismiss"))
                .ghost()
                .label("Dismiss")
                .on_click(send(Answer::Cancelled)),
        );
    match &question.kind {
        QuestionKind::Select { options } => {
            for (i, option) in options.iter().enumerate() {
                actions = actions.child(
                    Button::new(id(&format!("option-{i}")))
                        .label(option.clone())
                        .on_click(send(Answer::Value {
                            value: option.clone(),
                        })),
                );
            }
        }
        QuestionKind::Confirm => {
            actions = actions
                .child(
                    Button::new(id("decline"))
                        .label("Decline")
                        .on_click(send(Answer::Confirmed { confirmed: false })),
                )
                .child(
                    Button::new(id("approve"))
                        .primary()
                        .label("Approve")
                        .on_click(send(Answer::Confirmed { confirmed: true })),
                );
        }
        QuestionKind::Input { .. } | QuestionKind::Editor { .. } => {
            let (store, key, qid) = (store.clone(), key.to_string(), question.id.clone());
            let field = field.cloned();
            actions = actions.child(
                Button::new(id("submit"))
                    .primary()
                    .label("Submit")
                    .on_click(move |_, _, cx| {
                        let value = field.as_ref().map(|f| f.value(cx)).unwrap_or_default();
                        answer(&store, &key, &qid, Answer::Value { value }, cx)
                    }),
            );
        }
    }
    frame(p)
        .mx_auto()
        .mb(px(12.))
        .w_full()
        .max_w(px(768.))
        .p(px(16.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(status_pill(SessionStatus::Waiting, p))
                .child(div().t_heading().child(question.title.clone())),
        )
        .children(
            question
                .message
                .clone()
                .map(|message| div().mt(px(8.)).text_color(p.ink_muted).child(message)),
        )
        .children(field_element)
        .child(actions)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn a_card_without_a_deadline_never_expires() {
        assert!(!expired(None));
        assert!(expired(Some(now_ms() - 1)));
        assert!(!expired(Some(now_ms() + 60_000)));
    }
}
