//! A button that opens a list to pick one value from, driven from the keyboard:
//! the list opens on the current value, typing narrows it, arrows move, Enter
//! picks and Escape backs out. The composer's model menu and the new lane and
//! settings forms' menus are all this one control.

use std::rc::Rc;

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{CONTROL, CONTROL_LG, CONTROL_SM, RADIUS_SM, Type as _, icon, kbd, panel};
use crate::model::picker::{PickerGroup, filter_picker, initial_pick, pickable};
use crate::theme::Palette;
use crate::zoom::px;

/// What the picker keeps between frames: whether it is open, the search
/// typed and the highlighted row.
pub struct PickerState {
    open: bool,
    search: Entity<InputState>,
    picked: usize,
    scroll: ScrollHandle,
    /// Where focus goes after a pick or Escape.
    return_focus: Option<FocusHandle>,
}

pub enum PickerEvent {
    Opened,
    Selected(String),
}

impl EventEmitter<PickerEvent> for PickerState {}

impl PickerState {
    pub fn new(label: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = format!("Search {}s", label.to_lowercase());
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.picked = 0;
                cx.notify();
            }
        })
        .detach();
        Self {
            open: false,
            search,
            picked: 0,
            scroll: ScrollHandle::new(),
            return_focus: None,
        }
    }

    pub fn return_focus(mut self, focus: FocusHandle) -> Self {
        self.return_focus = Some(focus);
        self
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens the list from the keyboard, with the groups that decide where
    /// the highlight starts. Focus returns to `return_focus` on a pick or
    /// Escape, as when the trigger is clicked.
    pub fn open(&mut self, groups: &[PickerGroup], window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(groups, window, cx);
    }

    fn toggle(&mut self, groups: &[PickerGroup], window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return self.dismiss(false, window, cx);
        }
        cx.emit(PickerEvent::Opened);
        self.open = true;
        self.picked = initial_pick(groups);
        self.scroll.scroll_to_item(self.picked);
        self.search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn dismiss(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.search.update(cx, |s, cx| s.set_value("", window, cx));
        if refocus && let Some(focus) = &self.return_focus {
            window.focus(focus, cx);
        }
        cx.notify();
    }

    fn choose(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PickerEvent::Selected(id));
        self.dismiss(true, window, cx);
    }

    fn step(&mut self, by: isize, rows: usize, cx: &mut Context<Self>) {
        if rows == 0 {
            return;
        }
        self.picked = (self.picked as isize + by).rem_euclid(rows as isize) as usize;
        cx.notify();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerVariant {
    /// A bordered chip, in forms.
    Chip,
    /// A borderless toolbar button.
    Ghost,
}

/// The picker as drawn this frame, with its groups and trigger.
#[derive(IntoElement)]
pub struct Picker {
    id: ElementId,
    state: Entity<PickerState>,
    groups: Rc<Vec<PickerGroup>>,
    value: AnyElement,
    title: Option<SharedString>,
    leading: Option<&'static str>,
    variant: PickerVariant,
    up: bool,
    width: f32,
    empty_text: SharedString,
    searchable: bool,
    disabled: bool,
    p: Palette,
}

impl Picker {
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<PickerState>,
        groups: Vec<PickerGroup>,
        value: impl IntoElement,
        p: &Palette,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            groups: Rc::new(groups),
            value: value.into_any_element(),
            title: None,
            leading: None,
            variant: PickerVariant::Chip,
            up: false,
            width: 320.,
            empty_text: "Nothing to pick yet".into(),
            searchable: true,
            disabled: false,
            p: *p,
        }
    }
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn icon(mut self, name: &'static str) -> Self {
        self.leading = Some(name);
        self
    }
    pub fn ghost(mut self) -> Self {
        self.variant = PickerVariant::Ghost;
        self
    }
    pub fn up(mut self) -> Self {
        self.up = true;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty_text = text.into();
        self
    }
    /// Without a search field typing still narrows the list.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Picker {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = self.p;
        let state = self.state.read(cx);
        let open = state.open;
        let ghost = self.variant == PickerVariant::Ghost;
        let groups = self.groups.clone();
        let popup = open.then(|| self.popup(cx).into_any_element());
        let toggle_state = self.state.clone();
        let trigger = div()
            .id(self.id.clone())
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(CONTROL_SM))
            .max_w_full()
            .min_w_0()
            .px(px(8.))
            .rounded(px(RADIUS_SM))
            .text_color(if self.disabled {
                p.ink_faint
            } else {
                p.ink_muted
            })
            .when(ghost, |el| el.t_body())
            .when(!ghost, |el| {
                el.t_small()
                    .border_1()
                    .border_color(if open { p.line_strong } else { p.line })
                    .bg(p.surface_raised)
            })
            .when(open, |el| el.bg(p.selection).text_color(p.ink))
            .when(!self.disabled, |el| {
                el.cursor_pointer()
                    .hover(move |s| {
                        let s = s.bg(p.selection).text_color(p.ink);
                        if ghost {
                            s
                        } else {
                            s.border_color(p.line_strong)
                        }
                    })
                    .on_click(move |_, window, cx| {
                        toggle_state.update(cx, |s, cx| s.toggle(&groups, window, cx))
                    })
            })
            .when_some(self.title.clone(), |el, title| {
                el.tooltip(move |window, cx| {
                    crate::ds::Tooltip::new(title.clone()).build(window, cx)
                })
            })
            .children(self.leading.map(|name| icon(name, 14.).flex_none()))
            .child(div().min_w_0().truncate().child(self.value))
            .child(icon("chevron-down", 12.).flex_none().opacity(0.7));
        div().relative().min_w_0().child(trigger).children(popup)
    }
}

impl Picker {
    fn popup(&self, cx: &mut App) -> impl IntoElement {
        let p = self.p;
        let state = self.state.read(cx);
        let query = state.search.read(cx).value().to_string();
        let visible = filter_picker(&self.groups, &query);
        let rows: Vec<String> = pickable(&visible).iter().map(|i| i.id.clone()).collect();
        let picked = state.picked.min(rows.len().saturating_sub(1));
        let search = state.search.clone();
        let scroll = state.scroll.clone();

        let mut children = Vec::new();
        let mut index = 0usize;
        let mut row_child = Vec::new();
        for group in &visible {
            if let Some(label) = &group.label {
                children.push(
                    div()
                        .px(px(8.))
                        .pt(px(if children.is_empty() { 4. } else { 8. }))
                        .pb(px(4.))
                        .t_label()
                        .text_color(p.ink_muted)
                        .child(label.clone())
                        .into_any_element(),
                );
            }
            for item in &group.items {
                if item.disabled {
                    children.push(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .h(px(CONTROL))
                            .px(px(8.))
                            .text_color(p.ink_faint)
                            .child(div().w(px(14.)).flex_none())
                            .child(div().truncate().child(item.label.clone()))
                            .into_any_element(),
                    );
                    continue;
                }
                let at = index;
                index += 1;
                row_child.push(children.len());
                let on = at == picked;
                let (hover, choose) = (self.state.clone(), self.state.clone());
                let id = item.id.clone();
                children.push(
                    div()
                        .id(SharedString::from(format!("pick-{at}")))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(CONTROL))
                        .flex_none()
                        .px(px(8.))
                        .rounded(px(RADIUS_SM))
                        .when(on, |el| el.bg(p.selection))
                        .on_mouse_move(move |_, _, cx| {
                            hover.update(cx, |s, cx| {
                                if s.picked != at {
                                    s.picked = at;
                                    cx.notify();
                                }
                            })
                        })
                        .on_click(move |_, window, cx| {
                            let id = id.clone();
                            choose.update(cx, |s, cx| s.choose(id, window, cx))
                        })
                        .child(if item.current {
                            icon("check", 14.)
                                .flex_none()
                                .text_color(p.ink)
                                .into_any_element()
                        } else {
                            div().w(px(14.)).flex_none().into_any_element()
                        })
                        .child(
                            div()
                                .truncate()
                                .text_color(p.ink)
                                .when(item.detail.is_some(), |el| {
                                    el.flex_none().max_w(relative(0.62))
                                })
                                .when(item.detail.is_none(), |el| el.flex_1())
                                .child(item.label.clone()),
                        )
                        .children(item.detail.clone().map(|detail| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_right()
                                .t_code_sm()
                                .text_color(p.ink_muted)
                                .child(detail)
                        }))
                        .into_any_element(),
                );
            }
        }
        if rows.is_empty() {
            let text = if query.trim().is_empty() {
                self.empty_text.to_string()
            } else {
                format!("No match for \u{201c}{}\u{201d}", query.trim())
            };
            children.push(
                div()
                    .px(px(8.))
                    .py(px(12.))
                    .text_center()
                    .text_color(p.ink_muted)
                    .child(text)
                    .into_any_element(),
            );
        }
        if let Some(child) = row_child.get(picked) {
            scroll.scroll_to_item(*child);
        }

        let count = rows.len();
        let (up_state, down_state, enter_state, esc_state, tab_state) = (
            self.state.clone(),
            self.state.clone(),
            self.state.clone(),
            self.state.clone(),
            self.state.clone(),
        );
        let dismiss_state = self.state.clone();
        let enter_rows = rows.clone();
        let panel = panel(&p)
            .id("picker-panel")
            .occlude()
            .w(px(self.width))
            .flex()
            .flex_col()
            .overflow_hidden()
            .t_body()
            .on_mouse_down_out(move |_, window, cx| {
                dismiss_state.update(cx, |s, cx| s.dismiss(false, window, cx))
            })
            .capture_action(move |_: &MoveUp, _, cx| {
                cx.stop_propagation();
                up_state.update(cx, |s, cx| s.step(-1, count, cx))
            })
            .capture_action(move |_: &MoveDown, _, cx| {
                cx.stop_propagation();
                down_state.update(cx, |s, cx| s.step(1, count, cx))
            })
            .capture_action(move |_: &Enter, window, cx| {
                cx.stop_propagation();
                if let Some(id) = enter_rows.get(picked).cloned() {
                    enter_state.update(cx, |s, cx| s.choose(id, window, cx))
                }
            })
            .capture_action(move |_: &Escape, window, cx| {
                cx.stop_propagation();
                esc_state.update(cx, |s, cx| s.dismiss(true, window, cx))
            })
            .capture_action(move |_: &IndentInline, window, cx| {
                cx.stop_propagation();
                tab_state.update(cx, |s, cx| s.dismiss(false, window, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .when(self.searchable, |el| el.h(px(CONTROL_LG)).px(px(12.)))
                    .when(!self.searchable, |el| el.h(px(0.)).overflow_hidden())
                    .when(self.searchable, |el| el.border_b_1())
                    .border_color(p.line)
                    .text_color(p.ink_muted)
                    .child(icon("search", 14.).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&search).appearance(false).t_body()),
                    ),
            )
            .child(
                div()
                    .id("picker-list")
                    .flex()
                    .flex_col()
                    .max_h(px(288.))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .p(px(4.))
                    .children(children),
            )
            .when(self.searchable, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(16.))
                        .h(px(CONTROL))
                        .px(px(12.))
                        .border_t_1()
                        .border_color(p.line)
                        .t_small()
                        .text_color(p.ink_muted)
                        .child(hint(&["↑", "↓"], "move", &p))
                        .child(hint(&["↵"], "choose", &p))
                        .child(div().flex_1())
                        .child(hint(&["Esc"], "back", &p)),
                )
            });
        deferred(
            div()
                .absolute()
                .left_0()
                .when(self.up, |el| el.bottom_full().mb(px(6.)))
                .when(!self.up, |el| el.top_full().mt(px(6.)))
                .child(panel),
        )
        .with_priority(2)
    }
}

fn hint(keys: &[&str], label: &'static str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(kbd(keys, p))
        .child(label)
}
