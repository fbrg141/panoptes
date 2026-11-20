//! The design system's command palette: a search field over grouped rows,
//! arrows to move, Enter to open. The resume, session and search palettes
//! are all this one control. Port of `CommandPalette` in the design system.

use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{CONTROL, CONTROL_LG, KbdTone, RADIUS_MD, RADIUS_SM, Type as _, icon, kbd};
use crate::theme::{FONT_MONO, Palette, PaletteExt as _};

const ASK: &str = "__ask";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub icon: Option<&'static str>,
    pub kbd: Vec<&'static str>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteGroup {
    pub label: String,
    pub items: Vec<PaletteItem>,
}

/// The groups with only the rows whose label or hint holds the query, and
/// no empty groups.
pub fn filter_palette(groups: &[PaletteGroup], query: &str) -> Vec<PaletteGroup> {
    let needle = query.trim().to_lowercase();
    groups
        .iter()
        .map(|group| PaletteGroup {
            label: group.label.clone(),
            items: group
                .items
                .iter()
                .filter(|item| {
                    needle.is_empty()
                        || format!("{} {}", item.label, item.hint.as_deref().unwrap_or(""))
                            .to_lowercase()
                            .contains(&needle)
                })
                .cloned()
                .collect(),
        })
        .filter(|group| !group.items.is_empty())
        .collect()
}

pub enum PaletteEvent {
    Selected(String),
    Ask(String),
    QueryChanged(String),
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

pub struct CommandPalette {
    search: Entity<InputState>,
    groups: Vec<PaletteGroup>,
    picked: usize,
    scroll: ScrollHandle,
    asks: bool,
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
        cx.subscribe(&search, |this, search, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.picked = 0;
                let query = search.read(cx).value().to_string();
                cx.emit(PaletteEvent::QueryChanged(query));
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
            asks: false,
        }
    }

    /// Offer "Ask the agent" with the query when one is typed.
    pub fn asks(mut self) -> Self {
        self.asks = true;
        self
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

    /// What is shown: the filtered groups, then the ask row in its own group.
    fn visible(&self, cx: &App) -> Vec<PaletteGroup> {
        let query = self.query(cx);
        let mut groups = filter_palette(&self.groups, &query);
        if self.asks && !query.trim().is_empty() {
            groups.push(PaletteGroup {
                label: "Agent".into(),
                items: vec![PaletteItem {
                    id: ASK.into(),
                    label: "Ask the agent".into(),
                    hint: Some(format!("\u{201c}{}\u{201d}", query.trim())),
                    icon: Some("agent"),
                    kbd: Vec::new(),
                }],
            });
        }
        groups
    }

    fn pick(&mut self, id: String, cx: &mut Context<Self>) {
        if id == ASK {
            let query = self.query(cx).trim().to_string();
            cx.emit(PaletteEvent::Ask(query));
        } else {
            cx.emit(PaletteEvent::Selected(id));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, label: &str, hint: Option<&str>) -> PaletteItem {
        PaletteItem {
            id: id.into(),
            label: label.into(),
            hint: hint.map(Into::into),
            ..Default::default()
        }
    }

    #[::core::prelude::v1::test]
    fn filtering_matches_label_or_hint_and_drops_empty_groups() {
        let groups = vec![
            PaletteGroup {
                label: "A".into(),
                items: vec![
                    item("1", "Fix login", None),
                    item("2", "Docs", Some("12 messages")),
                ],
            },
            PaletteGroup {
                label: "B".into(),
                items: vec![item("3", "Other", None)],
            },
        ];
        let found = filter_palette(&groups, " LOGIN ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].items[0].id, "1");
        assert_eq!(filter_palette(&groups, "messages")[0].items[0].id, "2");
        assert_eq!(filter_palette(&groups, "").len(), 2);
    }
}
