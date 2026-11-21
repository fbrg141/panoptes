//! The design system's command palette: a search field over grouped rows,
//! arrows to move, Enter to open. The resume, session and search palettes are
//! all this one control.

use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{CONTROL, CONTROL_LG, KbdTone, RADIUS_MD, RADIUS_SM, Type as _, icon, kbd};
use crate::model::palette::filter_palette;
pub use crate::model::palette::{PaletteGroup, PaletteItem};
use crate::theme::{FONT_MONO, Palette, PaletteExt as _};
use crate::zoom::px;

pub enum PaletteEvent {
    Selected(String),
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

pub struct CommandPalette {
    search: Entity<InputState>,
    groups: Vec<PaletteGroup>,
    picked: usize,
    scroll: ScrollHandle,
}

impl CommandPalette {
    pub fn new(
        placeholder: impl Into<SharedString>,
        groups: Vec<PaletteGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.picked = 0;
                cx.notify();
            }
        })
        .detach();
        search.update(cx, |s, cx| s.focus(window, cx));
        Self {
            search,
            groups,
            picked: 0,
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_groups(&mut self, groups: Vec<PaletteGroup>, cx: &mut Context<Self>) {
        if self.groups != groups {
            self.groups = groups;
            cx.notify();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    /// What is shown: the groups filtered by the query.
    fn visible(&self, cx: &App) -> Vec<PaletteGroup> {
        filter_palette(&self.groups, &self.query(cx))
    }

    fn pick(&mut self, id: String, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Selected(id));
    }

    fn step(&mut self, by: isize, rows: usize, cx: &mut Context<Self>) {
        if rows > 0 {
            self.picked = (self.picked as isize + by).rem_euclid(rows as isize) as usize;
            cx.notify();
        }
    }
}

fn hint(keys: &[&str], text: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(kbd(keys, KbdTone::Normal, p))
        .child(text.to_string())
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let groups = self.visible(cx);
        let ids: Vec<String> = groups
            .iter()
            .flat_map(|g| g.items.iter().map(|i| i.id.clone()))
            .collect();
        let count = ids.len();
        let picked = self.picked.min(count.saturating_sub(1));
        let this = cx.entity().downgrade();

        let mut children = Vec::new();
        let mut index = 0usize;
        let mut picked_child = None;
        for group in &groups {
            children.push(
                div()
                    .pt(px(8.))
                    .px(px(8.))
                    .pb(px(4.))
                    .text_size(px(11.))
                    .line_height(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink_muted)
                    .child(group.label.to_uppercase())
                    .into_any_element(),
            );
            for item in &group.items {
                let at = index;
                index += 1;
                let on = at == picked;
                if on {
                    picked_child = Some(children.len());
                }
                let (hover, choose, id) = (this.clone(), this.clone(), item.id.clone());
                children.push(
                    div()
                        .id(SharedString::from(format!("cmd-{at}")))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.))
                        .h(px(CONTROL_LG))
                        .px(px(8.))
                        .rounded(px(RADIUS_SM))
                        .t_body()
                        .when(on, |el| el.bg(p.selection))
                        .on_mouse_move(move |_, _, cx| {
                            let _ = hover.update(cx, |this, cx| {
                                if this.picked != at {
                                    this.picked = at;
                                    cx.notify();
                                }
                            });
                        })
                        .on_click(move |_, _, cx| {
                            let id = id.clone();
                            let _ = choose.update(cx, |this, cx| this.pick(id, cx));
                        })
                        .child(match item.icon {
                            Some(name) => icon(name, 16.)
                                .flex_none()
                                .text_color(if on { p.ink } else { p.ink_muted })
                                .into_any_element(),
                            None => div().w(px(16.)).flex_none().into_any_element(),
                        })
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(8.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(div().flex_none().child(item.label.clone()))
                                .children(item.hint.clone().map(|hint| {
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(FONT_MONO)
                                        .text_size(px(11.5))
                                        .line_height(px(20.))
                                        .text_color(p.ink_muted)
                                        .child(hint)
                                })),
                        )
                        .when(!item.kbd.is_empty(), |el| {
                            el.child(kbd(&item.kbd, KbdTone::Normal, &p))
                        })
                        .into_any_element(),
                );
            }
        }
        if count == 0 {
            children.push(
                div()
                    .p(px(16.))
                    .t_body()
                    .text_color(p.ink_muted)
                    .child("No matches")
                    .into_any_element(),
            );
        }
        if let Some(child) = picked_child {
            self.scroll.scroll_to_item(child);
        }

        let (up, down, enter) = (this.clone(), this.clone(), this.clone());
        div()
            .id("command-palette")
            .w(px(560.))
            .max_w_full()
            .flex()
            .flex_col()
            .rounded(px(RADIUS_MD))
            .bg(p.surface_raised)
            .text_color(p.ink)
            .shadow(p.shadow_pop())
            .overflow_hidden()
            .capture_action(move |_: &MoveUp, _, cx| {
                cx.stop_propagation();
                let _ = up.update(cx, |this, cx| this.step(-1, count, cx));
            })
            .capture_action(move |_: &MoveDown, _, cx| {
                cx.stop_propagation();
                let _ = down.update(cx, |this, cx| this.step(1, count, cx));
            })
            .capture_action(move |_: &Enter, _, cx| {
                cx.stop_propagation();
                if let Some(id) = ids.get(picked).cloned() {
                    let _ = enter.update(cx, |this, cx| this.pick(id, cx));
                }
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .h(px(44.))
                    .px(px(16.))
                    .border_b_1()
                    .border_color(p.line)
                    .text_color(p.ink_muted)
                    .child(icon("search", 16.).flex_none())
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .appearance(false)
                                .text_size(px(14.))
                                .line_height(px(20.))
                                .px(px(0.)),
                        ),
                    )
                    .child(kbd(&["Esc"], KbdTone::Normal, &p)),
            )
            .child(
                div()
                    .id("command-list")
                    .flex()
                    .flex_col()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(4.))
                    .children(children),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(16.))
                    .h(px(CONTROL))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(p.line)
                    .t_small()
                    .text_color(p.ink_muted)
                    .child(hint(&["↑", "↓"], "to move", &p))
                    .child(hint(&["↵"], "to open", &p)),
            )
    }
}
