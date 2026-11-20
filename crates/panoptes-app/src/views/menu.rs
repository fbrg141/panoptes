//! The row menus' popup: the design system's Menu, anchored where it was
//! opened and dismissed by a click anywhere else.

use std::rc::Rc;

use gpui_kit::*;

use crate::ds::{menu_item, menu_separator, panel};
use crate::model::sidebar::MenuEntry;
use crate::theme::Palette;
use crate::zoom::px;

/// An open menu: who it belongs to, what it offers and where.
pub struct OpenMenu<Owner> {
    pub owner: Owner,
    pub items: Vec<MenuEntry>,
    pub position: Point<Pixels>,
}

pub fn popup_menu(
    items: &[MenuEntry],
    position: Point<Pixels>,
    p: &Palette,
    on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = Rc::new(on_select);
    let rows = items.iter().enumerate().map(|(index, entry)| match entry {
        MenuEntry::Separator => menu_separator(p).into_any_element(),
        MenuEntry::Item {
            id,
            label,
            icon,
            danger,
            disabled,
        } => {
            let on_select = on_select.clone();
            let id = *id;
            let mut row = menu_item(
                SharedString::from(format!("menu-{index}-{id}")),
                *icon,
                label.clone(),
                false,
                *danger,
                *disabled,
                p,
            );
            if !disabled {
                row = row.on_click(move |_, window, cx| on_select(id, window, cx));
            }
            row.into_any_element()
        }
    });
    deferred(
        anchored()
            .position(position)
            .snap_to_window_with_margin(px(8.))
            .child(
                panel(p)
                    .id("row-menu")
                    .occlude()
                    .min_w(px(220.))
                    .p(px(4.))
                    .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx))
                    .children(rows),
            ),
    )
    .with_priority(1)
}
