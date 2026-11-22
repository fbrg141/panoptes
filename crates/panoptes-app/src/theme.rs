//! The design system's tokens, read from `packages/design-system/tokens/tokens.json`
//! (the one place they live), as GPUI colours. The palette follows the
//! system's light or dark appearance.

use crate::zoom::px;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use serde_json::Value;

const TOKENS: &str = include_str!("../assets/tokens.json");

pub const FONT_SANS: &str = "IBM Plex Sans";
pub const FONT_MONO: &str = "IBM Plex Mono";
pub const FONT_DISPLAY: &str = "Gloock";

/// Every colour token, resolved for one appearance.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    // The brand and red tokens are kept with the rest of the set, unused by
    // the app's screens today.
    #[allow(dead_code)]
    pub brand_pupil: Hsla,
    #[allow(dead_code)]
    pub brand_bone: Hsla,
    #[allow(dead_code)]
    pub brand_cobalt: Hsla,
    #[allow(dead_code)]
    pub brand_red: Hsla,
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub surface_sunken: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub ink: Hsla,
    pub ink_muted: Hsla,
    pub ink_faint: Hsla,
    pub primary: Hsla,
    pub primary_hover: Hsla,
    pub on_primary: Hsla,
    #[allow(dead_code)]
    pub red: Hsla,
    #[allow(dead_code)]
    pub on_red: Hsla,
    pub focus: Hsla,
    pub link: Hsla,
    pub on_focus: Hsla,
    pub selection: Hsla,
    pub running: Hsla,
    #[allow(dead_code)]
    pub running_soft: Hsla,
    pub waiting: Hsla,
    pub waiting_soft: Hsla,
    pub error: Hsla,
    pub error_soft: Hsla,
    pub done: Hsla,
    #[allow(dead_code)]
    pub done_soft: Hsla,
    pub diff_add: Hsla,
    pub diff_remove: Hsla,
    pub scrim: Hsla,
    /// The first colour of `shadow-pop`, the drop shadow under floating panels.
    pub shadow: Hsla,
}

impl Global for Palette {}

impl Palette {
    pub fn load(dark: bool) -> Self {
        let tokens: Value = serde_json::from_str(TOKENS).expect("tokens.json parses");
        let list = tokens["color"]["tokens"].as_array().expect("colour tokens");
        let theme = if dark { "dark" } else { "light" };
        let raw = |name: &str| -> String {
            let token = list
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("token {name}"));
            match &token["value"] {
                Value::String(s) => s.clone(),
                Value::Object(by_theme) => by_theme[theme].as_str().unwrap().to_string(),
                other => panic!("token {name} has value {other}"),
            }
        };
        let color = |name: &str| -> Hsla {
            let mut value = raw(name);
            // Aliases name another token: `{brand-cobalt}`.
            while let Some(alias) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
                value = raw(alias);
            }
            hex(&value)
        };
        let shadow_pop = tokens["shadow"]["tokens"]
            .as_array()
            .and_then(|list| list.iter().find(|t| t["name"] == "shadow-pop"))
            .and_then(|t| t["value"][theme].as_str())
            .unwrap_or("0 6px 16px #00000080");
        let shadow = shadow_pop
            .split_whitespace()
            .find(|part| part.starts_with('#'))
            .map(|h| hex(h.trim_end_matches(',')))
            .unwrap_or(hsla(0., 0., 0., 0.5));
        Self {
            dark,
            brand_pupil: color("brand-pupil"),
            brand_bone: color("brand-bone"),
            brand_cobalt: color("brand-cobalt"),
            brand_red: color("brand-red"),
            bg: color("bg"),
            surface: color("surface"),
            surface_raised: color("surface-raised"),
            surface_sunken: color("surface-sunken"),
            line: color("line"),
            line_strong: color("line-strong"),
            ink: color("ink"),
            ink_muted: color("ink-muted"),
            ink_faint: color("ink-faint"),
            primary: color("primary"),
            primary_hover: color("primary-hover"),
            on_primary: color("on-primary"),
            red: color("red"),
            on_red: color("on-red"),
            focus: color("focus"),
            link: color("link"),
            on_focus: color("on-focus"),
            selection: color("selection"),
            running: color("running"),
            running_soft: color("running-soft"),
            waiting: color("waiting"),
            waiting_soft: color("waiting-soft"),
            error: color("error"),
            error_soft: color("error-soft"),
            done: color("done"),
            done_soft: color("done-soft"),
            diff_add: color("diff-add"),
            diff_remove: color("diff-remove"),
            scrim: color("scrim"),
            shadow,
        }
    }

    /// The drop shadow under menus, popovers and the palette (`shadow-pop`).
    pub fn shadow_pop(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.shadow,
            offset: point(px(0.), px(6.)),
            blur_radius: px(16.),
            spread_radius: px(0.),
            inset: false,
        }]
    }
}

/// `#RRGGBB` or `#RRGGBBAA`.
pub fn hex(value: &str) -> Hsla {
    let digits = value.trim_start_matches('#');
    let n = u32::from_str_radix(digits, 16).unwrap_or(0);
    match digits.len() {
        6 => rgb(n).into(),
        8 => rgba(n).into(),
        _ => hsla(0., 0., 0., 1.),
    }
}

pub trait PaletteExt {
    fn palette(&self) -> &Palette;
}

impl PaletteExt for App {
    fn palette(&self) -> &Palette {
        self.global::<Palette>()
    }
}

/// Loads the fonts, then the palette for the system appearance, and maps
/// it onto the component library's theme so its inputs, scrollbars and
/// text views match.
pub fn init(cx: &mut App) {
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![
        include_bytes!("../assets/fonts/ibm-plex-sans-latin-400-normal.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/fonts/ibm-plex-sans-latin-500-normal.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/fonts/ibm-plex-sans-latin-600-normal.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/fonts/ibm-plex-mono-latin-400-normal.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/fonts/ibm-plex-mono-latin-500-normal.ttf")
            .as_slice()
            .into(),
        include_bytes!("../assets/fonts/gloock-latin-400-normal.ttf")
            .as_slice()
            .into(),
    ];
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("Could not load the design system's fonts: {error:#}");
    }
    follow(cx.window_appearance(), cx);
}

/// Applies the palette for a system appearance. `PANOPTES_THEME=light` or
/// `dark` overrides it, for screenshots on systems that report none.
pub fn follow(appearance: WindowAppearance, cx: &mut App) {
    let dark = match std::env::var("PANOPTES_THEME").as_deref() {
        Ok("dark") => true,
        Ok("light") => false,
        _ => !matches!(
            appearance,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    };
    if cx.try_global::<Palette>().is_some_and(|p| p.dark == dark) {
        return;
    }
    apply(dark, cx);
    cx.refresh_windows();
}

/// Switches the palette, and the component theme with it.
pub fn apply(dark: bool, cx: &mut App) {
    let palette = Palette::load(dark);
    cx.set_global(palette);
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        let p = palette;
        let c = &mut theme.colors;
        c.background = p.bg;
        c.foreground = p.ink;
        c.border = p.line;
        c.input = p.line_strong;
        c.ring = p.focus;
        c.caret = p.ink;
        c.selection = p.selection;
        c.muted = p.surface_raised;
        c.muted_foreground = p.ink_muted;
        c.accent = p.surface_raised;
        c.accent_foreground = p.ink;
        c.popover = p.surface_raised;
        c.popover_foreground = p.ink;
        c.primary = p.primary;
        c.primary_hover = p.primary_hover;
        c.primary_active = p.primary_hover;
        c.primary_foreground = p.on_primary;
        c.secondary = p.surface_raised;
        c.secondary_hover = p.selection;
        c.secondary_active = p.selection;
        c.secondary_foreground = p.ink;
        c.danger = p.error;
        c.danger_foreground = p.bg;
        c.danger_hover = p.error;
        c.danger_active = p.error;
        c.link = p.link;
        c.link_hover = p.link;
        c.link_active = p.link;
        c.list = p.surface;
        c.list_hover = p.surface_raised;
        c.list_active = p.selection;
        c.list_active_border = p.focus;
        c.sidebar = p.surface;
        c.sidebar_foreground = p.ink;
        c.sidebar_border = p.line;
        c.sidebar_accent = p.surface_raised;
        c.sidebar_accent_foreground = p.ink;
        c.scrollbar = gpui_kit::transparent_black();
        c.scrollbar_thumb = p.line_strong.opacity(0.5);
        c.scrollbar_thumb_hover = p.line_strong;
        c.success = p.done;
        c.success_foreground = p.bg;
        c.warning = p.waiting;
        c.warning_foreground = p.bg;
        c.info = p.running;
        c.info_foreground = p.bg;
        theme.font_family = FONT_SANS.into();
        theme.font_size = px(13.);
        theme.mono_font_family = FONT_MONO.into();
        theme.mono_font_size = px(12.5);
        theme.radius = px(4.);
        theme.radius_lg = px(6.);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn tokens_resolve_aliases_in_both_themes() {
        let dark = Palette::load(true);
        let light = Palette::load(false);
        assert_eq!(dark.bg, hex("#070F10"));
        assert_eq!(light.bg, hex("#F4EEE4"));
        // `primary` is an alias of `brand-cobalt`, the same in both themes.
        assert_eq!(dark.primary, hex("#1F57B2"));
        assert_eq!(light.primary, dark.primary);
        // `link` aliases `focus`, which differs per theme.
        assert_eq!(dark.link, hex("#66A1FF"));
        assert_eq!(light.link, hex("#1F57B2"));
        assert_eq!(dark.scrim, hex("#020607B3"));
    }
}
