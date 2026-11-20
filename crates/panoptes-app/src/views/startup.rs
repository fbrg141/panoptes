//! What the window shows before the first listing: nothing for a brief load, a
//! loader for a slow one, and a failure with a retry, never a spinner forever.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{Button, LoopState, Type as _, icon, loop_mark};
use crate::store::{AppStore, Startup};
use crate::theme::Palette;
use crate::zoom::px;

/// Brief loads stay quiet for this long.
pub const LOADER_DELAY_MS: u64 = 150;

fn loading(p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(12.))
        .text_color(p.ink_muted)
        .child(loop_mark("loading", 20., LoopState::Thinking, p.ink_muted))
        .child("Loading projects and sessions\u{2026}")
}

pub fn startup_gate(
    startup: &Startup,
    show_loader: bool,
    store: &Entity<AppStore>,
    p: &Palette,
) -> Div {
    let loader = matches!(startup, Startup::Loading) && show_loader;
    let main = match startup {
        Startup::Error(error) => {
            let store = store.clone();
            div()
                .max_w(px(448.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(16.))
                .text_center()
                .child(icon("circle-alert", 24.).text_color(p.error))
                .child(
                    div()
                        .child(div().t_title().child("Could not load Panoptes"))
                        .child(
                            div()
                                .mt(px(8.))
                                .text_color(p.ink_muted)
                                .child(error.clone()),
                        ),
                )
                .child(
                    Button::new("startup-retry")
                        .icon("refresh-cw")
                        .label("Try again")
                        .on_click(move |_, _, cx| store.update(cx, |store, cx| store.init(cx))),
                )
        }
        _ => div().when(loader, |el| el.child(loading(p))),
    };
    div()
        .flex()
        .size_full()
        .bg(p.bg)
        .text_color(p.ink)
        .child(
            div()
                .w(px(320.))
                .flex_none()
                .flex()
                .flex_col()
                .border_r_1()
                .border_color(p.line)
                .bg(p.surface)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(38.))
                        .px(px(12.))
                        .border_b_1()
                        .border_color(p.line)
                        .child(loop_mark("brand", 20., LoopState::Idle, p.ink))
                        .child(div().t_wordmark().text_color(p.ink).child("Panoptes")),
                )
                .child(div().p(px(16.)).when(loader, |el| el.child(loading(p)))),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px(px(24.))
                .child(main),
        )
}
