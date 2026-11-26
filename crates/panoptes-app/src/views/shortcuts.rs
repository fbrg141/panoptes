//! The window's keyboard shortcuts and the ⌘/ cheatsheet that lists them, laid
//! out in two columns with a filter.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{self, CONTROL, RADIUS_SM, Type as _, icon, kbd};
use crate::model::shortcuts::{filter_shortcuts, shortcut_groups};
use crate::theme::PaletteExt as _;
use crate::ui::Ui;
use crate::zoom::px;

actions!(
    panoptes,
    [
        Search,
        NewLane,
        NewLaneSession,
        AddProject,
        ShowShortcuts,
        SplitVertical,
        SplitHorizontal,
        ToggleReview,
        JumpToLatest,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit,
        CloseWindow,
        HideApp,
        HideOthers,
        ShowAll,
    ]
);

pub const CONTEXT: &str = "Workspace";

/// Window shortcuts use ⌘ because this is a macOS app; pane splits also
/// accept Ctrl, which a text field's own Ctrl+D binding still wins over.
/// The app-menu keys are bound without a context, so they fire wherever the
/// focus is and the menu finds them for its key equivalents. Their handlers
/// are global too, so they work whatever holds the focus; only Close needs
/// the window, and the workspace handles it.
pub fn init(cx: &mut App) {
    cx.bind_keys(bindings());
    // Quit runs the on_app_quit cleanup in main.
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
}

/// Every binding that `init` registers. A menu item's shortcut is looked up
/// from this list, so the two must stay in step; the test below enforces it.
fn bindings() -> Vec<KeyBinding> {
    let at = Some(CONTEXT);
    vec![
        KeyBinding::new("cmd-p", Search, at),
        KeyBinding::new("cmd-t", NewLane, at),
        KeyBinding::new("cmd-shift-n", NewLaneSession, at),
        KeyBinding::new("cmd-n", AddProject, at),
        KeyBinding::new("cmd-/", ShowShortcuts, at),
        KeyBinding::new("cmd-?", ShowShortcuts, at),
        KeyBinding::new("cmd-d", SplitVertical, at),
        KeyBinding::new("ctrl-d", SplitVertical, at),
        KeyBinding::new("cmd-shift-d", SplitHorizontal, at),
        KeyBinding::new("ctrl-shift-d", SplitHorizontal, at),
        KeyBinding::new("ctrl-shift-r", ToggleReview, at),
        // A text field's own ⌘↓ (to the end of its text) wins over this.
        KeyBinding::new("cmd-down", JumpToLatest, at),
        // ⌘= is what macOS reports for the plus key, shifted or not.
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-_", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        // The app menu's own keys.
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
    ]
}

/// The menu bar. The first menu is the application menu, whose title macOS
/// replaces with the app's name. Key equivalents come from the bindings
/// above, so every action here needs a binding there.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Panoptes").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Panoptes", HideApp),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Panoptes", Quit),
        ]),
        Menu::new("Window").items([MenuItem::action("Close", CloseWindow)]),
        Menu::new("Help").items([MenuItem::action("Keyboard Shortcuts", ShowShortcuts)]),
    ]
}

pub struct Shortcuts {
    ui: Entity<Ui>,
    filter: Entity<InputState>,
}

impl Shortcuts {
    pub fn new(ui: Entity<Ui>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter shortcuts"));
        cx.subscribe(&filter, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        filter.update(cx, |f, cx| f.focus(window, cx));
        Self { ui, filter }
    }
}

impl Render for Shortcuts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let query = self.filter.read(cx).value().to_string();
        let focused = self.filter.read(cx).focus_handle(cx).is_focused(window);
        let groups = filter_shortcuts(shortcut_groups(), &query);
        let close = {
            let ui = self.ui.clone();
            move |_: &mut Window, cx: &mut App| {
                ui.update(cx, |ui, cx| ui.close(cx));
                true
            }
        };
        let header_close: ds::CloseHandler = {
            let ui = self.ui.clone();
            std::rc::Rc::new(move |_, cx| ui.update(cx, |ui, cx| ui.close(cx)))
        };
        let section = |group: &crate::model::shortcuts::ShortcutGroup| {
            div()
                .child(
                    div()
                        .pb(px(4.))
                        .t_label()
                        .text_color(p.ink_muted)
                        .child(group.label),
                )
                .children(group.entries.iter().enumerate().map(|(i, entry)| {
                    div()
                        .id(SharedString::from(format!("{}-{i}", group.label)))
                        .mx(px(-8.))
                        .px(px(8.))
                        .h(px(CONTROL))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .rounded(px(RADIUS_SM))
                        .hover(move |s| s.bg(p.selection))
                        .child(div().flex_1().min_w_0().truncate().child(entry.label))
                        .child(kbd(entry.keys, &p))
                }))
        };
        // Two columns, filled row by row as the CSS grid does.
        let body = if groups.is_empty() {
            div()
                .py(px(32.))
                .text_center()
                .text_color(p.ink_muted)
                .child(format!(
                    "No shortcut matches \u{201c}{}\u{201d}",
                    query.trim()
                ))
        } else {
            let (mut left, mut right) = (Vec::new(), Vec::new());
            for (i, group) in groups.iter().enumerate() {
                if i % 2 == 0 {
                    left.push(section(group));
                } else {
                    right.push(section(group));
                }
            }
            div()
                .flex()
                .gap(px(32.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .children(left),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .children(right),
                )
        };
        let panel = ds::overlay_panel(680., window, &p)
            .child(ds::overlay_header(
                div().t_heading().child("Keyboard shortcuts"),
                Some(header_close),
            ))
            .child(
                div()
                    .mx(px(16.))
                    .h(px(CONTROL))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(RADIUS_SM))
                    .border_1()
                    .border_color(if focused { p.line_strong } else { p.line })
                    .bg(p.surface_sunken)
                    .px(px(8.))
                    .text_color(p.ink_muted)
                    .child(icon("search", 14.).flex_none())
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.filter)
                                .appearance(false)
                                .t_body()
                                .px(px(0.)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("shortcuts-body")
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pt(px(12.))
                    .pb(px(16.))
                    .child(body),
            );
        ds::modal_layer(window, close, panel, &p)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShowAll, bindings, menus};
    use gpui_kit::{Action as _, MenuItem};
    use std::collections::HashSet;

    /// Every action item in `items`, submenus included, as (label, action).
    fn menu_actions(items: Vec<MenuItem>, into: &mut Vec<(String, String)>) {
        for item in items {
            match item {
                MenuItem::Action { name, action, .. } => {
                    into.push((name.to_string(), action.name().to_string()));
                }
                MenuItem::Submenu(menu) => menu_actions(menu.items, into),
                _ => {}
            }
        }
    }

    /// A menu item's key equivalent is looked up from the bindings, so a
    /// menu action without a binding shows up dead, with no shortcut. The
    /// one exception is Show All, which macOS never gives a shortcut.
    #[test]
    fn every_menu_action_is_bound() {
        let bound: HashSet<&'static str> = bindings()
            .iter()
            .map(|binding| binding.action().name())
            .collect();
        let mut actions = Vec::new();
        for menu in menus() {
            menu_actions(menu.items, &mut actions);
        }
        actions.retain(|(_, action)| action != ShowAll.name());
        assert!(
            actions.iter().any(|(name, _)| name == "Quit Panoptes"),
            "the application menu is missing its Quit item"
        );
        for (item, action) in &actions {
            assert!(
                bound.contains(action.as_str()),
                "the menu item {item} has no binding for {action}"
            );
        }
    }
}
