//! The design system's components: Button, Kbd, Tooltip, StatusPill, the loop
//! mark, tabs, menus, dialogs, fields and overlays, drawn from the tokens.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_harness::SessionStatus;

use crate::assets::loop_geometry;
use crate::theme::{FONT_DISPLAY, FONT_MONO, FONT_SANS, Palette, PaletteExt as _};
use crate::zoom::px;

pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
pub type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

// Sizes from the tokens.
pub const CONTROL_SM: f32 = 24.;
pub const CONTROL: f32 = 28.;
pub const CONTROL_LG: f32 = 32.;
pub const RADIUS_XS: f32 = 2.;
pub const RADIUS_SM: f32 = 4.;
pub const RADIUS_MD: f32 = 6.;
pub const RADIUS_LG: f32 = 8.;

/// The type scale, as style shorthands.
pub trait Type: Styled + Sized {
    fn type_style(self, family: &'static str, size: f32, line: f32, weight: FontWeight) -> Self {
        self.font_family(family)
            .text_size(px(size))
            .line_height(px(line))
            .font_weight(weight)
    }
    fn t_wordmark(self) -> Self {
        self.type_style(FONT_DISPLAY, 20., 24., FontWeight::NORMAL)
    }
    fn t_title(self) -> Self {
        self.type_style(FONT_SANS, 18., 24., FontWeight::SEMIBOLD)
    }
    fn t_heading(self) -> Self {
        self.type_style(FONT_SANS, 14., 20., FontWeight::SEMIBOLD)
    }
    fn t_body(self) -> Self {
        self.type_style(FONT_SANS, 13., 20., FontWeight::NORMAL)
    }
    fn t_body_strong(self) -> Self {
        self.type_style(FONT_SANS, 13., 20., FontWeight::MEDIUM)
    }
    fn t_label(self) -> Self {
        self.type_style(FONT_SANS, 12., 16., FontWeight::MEDIUM)
    }
    fn t_small(self) -> Self {
        self.type_style(FONT_SANS, 12., 16., FontWeight::NORMAL)
    }
    /// Caption: uppercase is the caller's, as GPUI has no text transform.
    fn t_caption(self) -> Self {
        self.type_style(FONT_SANS, 11., 14., FontWeight::MEDIUM)
    }
    fn t_code(self) -> Self {
        self.type_style(FONT_MONO, 12.5, 19., FontWeight::NORMAL)
    }
    fn t_code_strong(self) -> Self {
        self.type_style(FONT_MONO, 12.5, 19., FontWeight::MEDIUM)
    }
    fn t_code_sm(self) -> Self {
        self.type_style(FONT_MONO, 11.5, 16., FontWeight::NORMAL)
    }
}

impl<T: Styled> Type for T {}

/// A Lucide icon at a 1.5px stroke, in the current text colour.
pub fn icon(name: &str, size: f32) -> Icon {
    Icon(
        svg()
            .path(SharedString::from(format!("icons/{name}.svg")))
            .size(px(size))
            .flex_none(),
    )
}

/// An SVG that takes the surrounding text colour unless given its own, as
/// `currentColor` does on the web. GPUI's own `Svg` paints only in a colour
/// set on itself.
pub struct Icon(Svg);

impl Icon {
    /// Turned clockwise, as `rotate-90` turns `panel-left` into a top panel.
    pub fn rotate(self, degrees: f32) -> Self {
        Icon(
            self.0
                .with_transformation(Transformation::rotate(radians(degrees.to_radians()))),
        )
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl IntoElement for Icon {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Icon {
    type RequestLayoutState = <Svg as Element>::RequestLayoutState;
    type PrepaintState = <Svg as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.0)
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        Element::source_location(&self.0)
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.0.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.0
            .prepaint(id, inspector_id, bounds, layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.0.style().text.color.is_none() {
            self.0.style().text.color = Some(window.text_style().color);
        }
        self.0
            .paint(id, inspector_id, bounds, layout, prepaint, window, cx)
    }
}

// --- Kbd -------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum KbdTone {
    #[default]
    Normal,
    OnPrimary,
}

pub fn kbd(keys: &[&str], tone: KbdTone, p: &Palette) -> Div {
    let (bg, border, ink) = match tone {
        KbdTone::Normal => (p.surface_raised, p.line_strong, p.ink_muted),
        KbdTone::OnPrimary => (transparent_black(), p.on_primary, p.on_primary),
    };
    h_flex_gap(2.).children(keys.iter().map(|k| {
        div()
            .min_w(px(18.))
            .h(px(18.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .border_1()
            .border_color(border)
            .rounded(px(RADIUS_XS))
            .bg(bg)
            .text_color(ink)
            .t_code_sm()
            .child(k.to_string())
    }))
}

pub fn h_flex_gap(gap: f32) -> Div {
    div().flex().flex_row().items_center().gap(px(gap))
}

pub fn v_flex_gap(gap: f32) -> Div {
    div().flex().flex_col().gap(px(gap))
}

// --- Button ----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Size {
    Sm,
    #[default]
    Md,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<&'static str>,
    variant: Variant,
    size: Size,
    kbd: Option<Vec<&'static str>>,
    disabled: bool,
    active: bool,
    tooltip: Option<SharedString>,
    on_click: Option<ClickHandler>,
    full_width: bool,
    /// Drawn in place of an icon, such as Stop's filled square.
    glyph: Option<AnyElement>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            variant: Variant::Secondary,
            size: Size::Md,
            kbd: None,
            disabled: false,
            active: false,
            tooltip: None,
            on_click: None,
            full_width: false,
            glyph: None,
        }
    }
    pub fn glyph(mut self, glyph: impl IntoElement) -> Self {
        self.glyph = Some(glyph.into_any_element());
        self
    }
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }
    pub fn ghost(self) -> Self {
        self.variant(Variant::Ghost)
    }
    pub fn danger(self) -> Self {
        self.variant(Variant::Danger)
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn small(self) -> Self {
        self.size(Size::Sm)
    }
    pub fn kbd(mut self, keys: Vec<&'static str>) -> Self {
        self.kbd = Some(keys);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Shows the hover fill while a menu it opened is open.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = *cx.palette();
        let icon_only = self.label.is_none() && (self.icon.is_some() || self.glyph.is_some());
        let (height, pad, gap, font) = match self.size {
            Size::Sm => (CONTROL_SM, 8., 4., 12.),
            Size::Md => (CONTROL, 12., 8., 13.),
        };
        let (bg, ink, border, hover_bg, hover_ink, hover_border) = match self.variant {
            Variant::Primary => (
                p.primary,
                p.on_primary,
                transparent_black(),
                p.primary_hover,
                p.on_primary,
                transparent_black(),
            ),
            Variant::Secondary => (
                transparent_black(),
                p.ink,
                p.line_strong,
                p.selection,
                p.ink,
                p.line_strong,
            ),
            Variant::Ghost => (
                transparent_black(),
                p.ink_muted,
                transparent_black(),
                p.surface_raised,
                p.ink,
                transparent_black(),
            ),
            Variant::Danger => (
                p.error_soft,
                p.error,
                transparent_black(),
                p.error_soft,
                p.error,
                p.error,
            ),
        };
        let (bg, ink, border) = if self.disabled {
            (transparent_black(), p.ink_faint, p.line)
        } else if self.active {
            (hover_bg, hover_ink, hover_border)
        } else {
            (bg, ink, border)
        };
        let mut el = div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(gap))
            .h(px(height))
            .border_1()
            .border_color(border)
            .rounded(px(RADIUS_SM))
            .bg(bg)
            .text_color(ink)
            .font_family(FONT_SANS)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(font))
            .line_height(px(20.))
            .whitespace_nowrap();
        el = if icon_only {
            el.w(px(height))
        } else {
            el.px(px(pad))
        };
        if self.full_width {
            el = el.w_full();
        }
        if !self.disabled {
            el = el.cursor_pointer().hover(move |s| {
                s.bg(hover_bg)
                    .text_color(hover_ink)
                    .border_color(hover_border)
            });
            if let Some(handler) = self.on_click {
                el = el.on_click(move |event, window, cx| handler(event, window, cx));
            }
        }
        if let Some(tip) = self.tooltip {
            el = el.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
        }
        el.children(self.icon.map(|name| icon(name, 16.)))
            .children(self.glyph)
            .children(self.label.map(|l| div().child(l)))
            .children(self.kbd.map(|keys| {
                kbd(
                    &keys,
                    if self.variant == Variant::Primary {
                        KbdTone::OnPrimary
                    } else {
                        KbdTone::Normal
                    },
                    &p,
                )
            }))
    }
}

// --- Tooltip ---------------------------------------------------------------

pub struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }
    pub fn build(self, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| self).into()
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        h_flex_gap(8.)
            .px(px(8.))
            .py(px(4.))
            .rounded(px(RADIUS_SM))
            .bg(p.ink)
            .text_color(p.bg)
            .t_label()
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

// --- Status ----------------------------------------------------------------

/// The logo's pupil: a disc with its glint cut out, the sign for running.
pub fn pupil(size: f32) -> Svg {
    svg().path("icons/pupil.svg").size(px(size)).flex_none()
}

pub fn status_color(status: SessionStatus, p: &Palette) -> Hsla {
    match status {
        SessionStatus::Running => p.running,
        SessionStatus::Waiting => p.waiting,
        SessionStatus::Failed => p.error,
        SessionStatus::Done => p.done,
        SessionStatus::Idle => p.ink_muted,
    }
}

pub fn status_word(status: SessionStatus) -> Option<&'static str> {
    match status {
        SessionStatus::Idle => None,
        SessionStatus::Running => Some("Running"),
        SessionStatus::Waiting => Some("Waiting"),
        SessionStatus::Failed => Some("Failed"),
        SessionStatus::Done => Some("Done"),
    }
}

/// The small glyph for a status: alert, check, pupil or a ring.
pub fn status_glyph(status: SessionStatus, p: &Palette) -> AnyElement {
    let color = status_color(status, p);
    match status {
        SessionStatus::Failed => icon("circle-alert", 14.)
            .text_color(color)
            .into_any_element(),
        SessionStatus::Done => icon("check", 14.).text_color(color).into_any_element(),
        SessionStatus::Running => pupil(9.).text_color(color).into_any_element(),
        SessionStatus::Waiting => div()
            .size(px(8.))
            .rounded_full()
            .border_2()
            .border_color(color)
            .flex_none()
            .into_any_element(),
        SessionStatus::Idle => div()
            .size(px(8.))
            .rounded_full()
            .border(px(1.5))
            .border_color(color)
            .flex_none()
            .into_any_element(),
    }
}

pub fn status_pill(status: SessionStatus, p: &Palette) -> Div {
    let (bg, ink) = match status {
        SessionStatus::Running => (p.running_soft, p.running),
        SessionStatus::Waiting => (p.waiting_soft, p.waiting),
        SessionStatus::Failed => (p.error_soft, p.error),
        SessionStatus::Done => (p.done_soft, p.done),
        SessionStatus::Idle => (p.surface_raised, p.ink_muted),
    };
    let mut pill = h_flex_gap(6.)
        .flex_none()
        .h(px(22.))
        .px(px(8.))
        .rounded(px(RADIUS_SM))
        .bg(bg)
        .text_color(ink)
        .t_label()
        .whitespace_nowrap()
        .child(status_glyph(status, p))
        .child(status_word(status).unwrap_or("Idle"));
    if status == SessionStatus::Idle {
        pill = pill.border_1().border_color(p.line);
    }
    pill
}

// --- LoopMark --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoopState {
    Idle,
    Thinking,
}

/// The logo, one ink. `width` is the rendered width in px. Thinking moves
/// the pupils left and right and blinks, as the `glance` motion does.
pub fn loop_mark(
    id: impl Into<ElementId>,
    width: f32,
    state: LoopState,
    color: Hsla,
) -> AnyElement {
    let (size, geometry) = loop_geometry(width);
    let height = (width as f64 * geometry.aspect).round() as f32;
    let body = svg()
        .path(SharedString::from(format!("loop/{size}/body.svg")))
        .absolute()
        .size_full()
        .text_color(color);
    let pupils = svg()
        .path(SharedString::from(format!("loop/{size}/pupils.svg")))
        .absolute()
        .size_full()
        .text_color(color);
    let pupils = match state {
        LoopState::Idle => pupils.into_any_element(),
        LoopState::Thinking => {
            // The glance keyframes in view-box units, scaled to this width.
            let (left, right) = match size {
                "L" => (-16., 5.),
                "M" => (-15., 4.5),
                _ => (-12., 1.5),
            };
            let scale = width / 226.;
            pupils
                .with_animation(
                    "glance",
                    Animation::new(Duration::from_millis(2400)).repeat(),
                    move |svg, t| {
                        let (x, y) = glance(t, left, right);
                        let blink = blink(t);
                        svg.with_transformation(
                            Transformation::translate(point(px(x * scale), px(y * scale)))
                                .with_scaling(size2(1., blink)),
                        )
                    },
                )
                .into_any_element()
        }
    };
    div()
        .id(id.into())
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .child(body)
        .child(pupils)
        .into_any_element()
}

fn size2(w: f32, h: f32) -> gpui_kit::Size<f32> {
    gpui_kit::Size {
        width: w,
        height: h,
    }
}

/// `px-glance`: rest, look left, look right, rest.
fn glance(t: f32, left: f32, right: f32) -> (f32, f32) {
    let ease = |x: f32| 1. - (1. - x).powi(3);
    let lerp = |a: f32, b: f32, x: f32| a + (b - a) * ease(x.clamp(0., 1.));
    match t {
        t if t < 0.10 => (0., 0.),
        t if t < 0.22 => {
            let x = (t - 0.10) / 0.12;
            (lerp(0., left, x), lerp(0., 1., x))
        }
        t if t < 0.44 => (left, 1.),
        t if t < 0.56 => {
            let x = (t - 0.44) / 0.12;
            (lerp(left, right, x), lerp(1., 2., x))
        }
        t if t < 0.76 => (right, 2.),
        t if t < 0.88 => {
            let x = (t - 0.76) / 0.12;
            (lerp(right, 0., x), lerp(2., 0., x))
        }
        _ => (0., 0.),
    }
}

/// `px-blink`: a quick squash halfway through.
fn blink(t: f32) -> f32 {
    if (0.46..0.54).contains(&t) {
        let x = (t - 0.46) / 0.08;
        1. - 0.9 * (1. - (2. * x - 1.).abs())
    } else {
        1.
    }
}

// --- Tabs ------------------------------------------------------------------

pub struct Tab {
    pub id: &'static str,
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub count: Option<usize>,
}

pub fn tabs(
    tabs: Vec<Tab>,
    value: &str,
    p: &Palette,
    on_change: impl Fn(&'static str, &mut Window, &mut App) + 'static,
) -> Div {
    let on_change = Rc::new(on_change);
    h_flex_gap(4.)
        .border_b_1()
        .border_color(p.line)
        .children(tabs.into_iter().map(|tab| {
            let on = tab.id == value;
            let id = tab.id;
            let on_change = on_change.clone();
            let ink = if on { p.ink } else { p.ink_muted };
            let hover_ink = p.ink;
            div()
                .id(SharedString::from(format!("tab-{id}")))
                .relative()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(CONTROL_LG))
                .px(px(12.))
                .t_label()
                .text_color(ink)
                .cursor_pointer()
                .hover(move |s| s.text_color(hover_ink))
                .on_click(move |_, window, cx| on_change(id, window, cx))
                .children(tab.icon.map(|name| icon(name, 16.)))
                .child(tab.label)
                .children(tab.count.map(|n| {
                    div()
                        .t_code_sm()
                        .text_color(p.ink_muted)
                        .child(n.to_string())
                }))
                .when(on, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom(px(-1.))
                            .h(px(2.))
                            .bg(p.focus),
                    )
                })
        }))
}

// --- Menu ------------------------------------------------------------------

/// A floating panel: menus, popovers and the palette.
pub fn panel(p: &Palette) -> Div {
    div()
        .bg(p.surface_raised)
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(p.line)
        .shadow(p.shadow_pop())
        .text_color(p.ink)
}

pub fn menu_separator(p: &Palette) -> Div {
    div().h(px(1.)).my(px(4.)).mx(px(8.)).bg(p.line)
}

/// One menu row: an icon (or its gap), the label and an optional shortcut.
pub fn menu_item(
    id: impl Into<ElementId>,
    icon_name: Option<&'static str>,
    label: impl Into<SharedString>,
    active: bool,
    danger: bool,
    disabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let ink = if disabled {
        p.ink_faint
    } else if danger {
        p.error
    } else {
        p.ink
    };
    let icon_ink = if danger {
        p.error
    } else if active {
        p.ink
    } else {
        p.ink_muted
    };
    let selection = p.selection;
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(CONTROL))
        .px(px(8.))
        .rounded(px(RADIUS_SM))
        .t_body()
        .text_color(ink)
        .when(active, |el| el.bg(selection))
        .when(!disabled, |el| el.hover(move |s| s.bg(selection)))
        .child(match icon_name {
            Some(name) => icon(name, 16.).text_color(icon_ink).into_any_element(),
            None => div().w(px(16.)).flex_none().into_any_element(),
        })
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
}

// --- Dialog ----------------------------------------------------------------

/// The scrim and dialog frame: title, close button, body and actions.
pub fn dialog(
    title: impl Into<SharedString>,
    width: f32,
    body: impl IntoElement,
    actions: Option<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    p: &Palette,
) -> Stateful<Div> {
    let on_close = Rc::new(on_close);
    let close_button = {
        let on_close = on_close.clone();
        Button::new("dialog-close")
            .ghost()
            .small()
            .icon("x")
            .tooltip("Close")
            .on_click(move |_, window, cx| on_close(window, cx))
    };
    let on_scrim = on_close.clone();
    div()
        .id("scrim")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.))
        .bg(p.scrim)
        .occlude()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| on_scrim(window, cx))
        .child(
            div()
                .id("dialog")
                .w(px(width))
                .max_w_full()
                .max_h_full()
                .flex()
                .flex_col()
                .bg(p.surface_raised)
                .rounded(px(RADIUS_LG))
                .shadow(p.shadow_pop())
                .border_1()
                .border_color(p.line)
                .text_color(p.ink)
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    h_flex_gap(8.)
                        .pt(px(12.))
                        .pr(px(12.))
                        .pl(px(16.))
                        .child(div().flex_1().t_heading().child(title.into()))
                        .child(close_button),
                )
                .child(
                    div()
                        .id("dialog-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .pt(px(8.))
                        .px(px(16.))
                        .pb(px(16.))
                        .t_body()
                        .text_color(p.ink_muted)
                        .child(body),
                )
                .children(actions.map(|actions| {
                    h_flex_gap(8.)
                        .justify_end()
                        .py(px(12.))
                        .px(px(16.))
                        .border_t_1()
                        .border_color(p.line)
                        .child(actions)
                })),
        )
}

// --- Checkbox --------------------------------------------------------------

pub fn checkbox(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    disabled: bool,
    p: &Palette,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (bg, border) = if disabled {
        (transparent_black(), p.line)
    } else if checked {
        (p.focus, p.focus)
    } else {
        (p.surface_raised, p.line_strong)
    };
    div()
        .id(id.into())
        .flex()
        .items_start()
        .gap(px(8.))
        .text_color(if disabled { p.ink_faint } else { p.ink })
        .when(!disabled, |el| {
            el.cursor_pointer()
                .on_click(move |_, window, cx| on_toggle(!checked, window, cx))
        })
        .child(
            div()
                .mt(px(2.))
                .size(px(16.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border(px(1.5))
                .border_color(border)
                .rounded(px(RADIUS_XS))
                .bg(bg)
                .when(checked, |el| {
                    el.child(icon("check", 14.).text_color(p.on_focus))
                }),
        )
        .child(
            v_flex_gap(0.)
                .child(div().t_body().child(label.into()))
                .children(hint.map(|h| div().t_small().text_color(p.ink_muted).child(h))),
        )
}

/// A form field's label above its control.
pub fn field_label(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().t_label().text_color(p.ink_muted).child(text.into())
}

/// The bordered box an input sits in.
pub fn input_frame(focused: bool, error: bool, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(CONTROL))
        .px(px(8.))
        .bg(p.surface_raised)
        .border_1()
        .border_color(if error {
            p.error
        } else if focused {
            p.focus
        } else {
            p.line_strong
        })
        .rounded(px(RADIUS_SM))
        .text_color(p.ink_muted)
}

/// A labelled single-line field with a hint or an error under it: the
/// design system's `Input` with `label`, `hint` and `error`.
pub fn text_field(
    label: impl Into<SharedString>,
    field: impl IntoElement,
    focused: bool,
    error: Option<SharedString>,
    hint: Option<SharedString>,
    p: &Palette,
) -> Div {
    let failed = error.is_some();
    let message = error.clone().or(hint);
    v_flex_gap(4.)
        .min_w_0()
        .child(field_label(label, p))
        .child(
            input_frame(focused, failed, p)
                .h(px(CONTROL))
                .child(div().flex_1().min_w_0().child(field)),
        )
        .children(message.map(|message| {
            h_flex_gap(4.)
                .t_small()
                .text_color(if failed { p.error } else { p.ink_muted })
                .when(failed, |el| el.child(icon("circle-alert", 14.)))
                .child(message)
        }))
}

// --- Overlay ---------------------------------------------------------------

/// The window's modal layer: a scrim, and the content near the top like a
/// command palette. A click on the scrim or Escape calls `on_close`, which says
/// whether it closed: a panel with an open menu lets Escape close the menu
/// instead.
pub fn modal_layer(
    window: &Window,
    on_close: impl Fn(&mut Window, &mut App) -> bool + 'static,
    content: impl IntoElement,
    p: &Palette,
) -> Stateful<Div> {
    let on_close = Rc::new(on_close);
    let (on_scrim, on_escape, on_key) = (on_close.clone(), on_close.clone(), on_close);
    let top = window.viewport_size().height * 0.12;
    div()
        .id("modal-layer")
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .px(px(24.))
        .pt(top)
        .bg(p.scrim)
        .occlude()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_scrim(window, cx);
        })
        .capture_action(move |_: &gpui_kit::component::input::Escape, window, cx| {
            if on_escape(window, cx) {
                cx.stop_propagation();
            }
        })
        // Escape with focus outside a text field, where it is no action.
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" && on_key(window, cx) {
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .id("modal-content")
                .max_w_full()
                .flex()
                .flex_col()
                .min_h_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(content),
        )
}

/// A panel in the modal layer: the content decides its header and footer.
pub fn overlay_panel(width: f32, window: &Window, p: &Palette) -> Div {
    div()
        .w(px(width))
        .max_w_full()
        .max_h(window.viewport_size().height * 0.76)
        .flex()
        .flex_col()
        .rounded(px(RADIUS_LG))
        .bg(p.surface_raised)
        .text_color(p.ink)
        .shadow(p.shadow_pop())
}

/// A panel's top line: what it is, and where it acts.
pub fn overlay_header(content: impl IntoElement, on_close: Option<CloseHandler>) -> Div {
    h_flex_gap(8.)
        .h(px(44.))
        .flex_none()
        .px(px(16.))
        .child(h_flex_gap(6.).flex_1().min_w_0().child(content))
        .children(on_close.map(|on_close| {
            div().mr(px(-6.)).child(
                Button::new("overlay-close")
                    .ghost()
                    .small()
                    .icon("x")
                    .tooltip("Close (Esc)")
                    .on_click(move |_, window, cx| on_close(window, cx)),
            )
        }))
}

/// A panel's bottom line: options on the left, the one action on the right.
pub fn overlay_footer(p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .min_h(px(48.))
        .border_t_1()
        .border_color(p.line)
        .px(px(12.))
        .py(px(8.))
}

/// An on/off option in a panel's footer, styled as a chip beside the pickers.
pub fn option_toggle(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    title: Option<SharedString>,
    pressed: bool,
    disabled: bool,
    p: &Palette,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let p = *p;
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(CONTROL_SM))
        .px(px(8.))
        .rounded(px(RADIUS_SM))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_raised)
        .t_small()
        .text_color(if disabled { p.ink_faint } else { p.ink_muted })
        .when(!disabled, |el| {
            el.cursor_pointer()
                .hover(move |s| {
                    s.bg(p.selection)
                        .text_color(p.ink)
                        .border_color(p.line_strong)
                })
                .on_click(move |_, window, cx| on_change(!pressed, window, cx))
        })
        .when_some(title, |el, title| {
            el.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        })
        .child(
            div()
                .size(px(14.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_XS))
                .border_1()
                .when(pressed, |el| {
                    el.border_color(p.focus)
                        .bg(p.focus)
                        .text_color(p.on_focus)
                        .child(icon("check", 12.))
                })
                .when(!pressed, |el| el.border_color(p.line_strong)),
        )
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn the_glance_returns_to_rest() {
        assert_eq!(glance(0.0, -12., 1.5), (0., 0.));
        assert_eq!(glance(0.3, -12., 1.5), (-12., 1.));
        assert_eq!(glance(0.6, -12., 1.5), (1.5, 2.));
        assert_eq!(glance(0.95, -12., 1.5), (0., 0.));
        assert!(blink(0.5) < 0.2);
        assert_eq!(blink(0.1), 1.);
    }
}
