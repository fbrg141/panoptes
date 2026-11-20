//! One prompt and the agent's reply: its work as quiet rows, then its
//! answer. Port of `Turn.tsx`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_harness::{
    Approval, Block, Message, SessionStatus, ToolImage, ToolKind, ToolOutput, ToolStatus,
};

use crate::ds::{Button, LoopState, Type as _, icon, loop_mark, status_pill};
use crate::model::diff::{DiffLine, LineKind, diff_stat, edit_diff};
use crate::model::format::local_time;
use crate::model::turns::{
    Step, ToolStep, Turn, answer_text, edits_of, format_duration, group_label, output_text,
    summarize_work, tool_images,
};
use crate::model::usage::turn_usage_parts;
use crate::theme::{FONT_MONO, Palette};
use crate::zoom::px;

/// Lines of output shown before "Show all".
const CLAMP: usize = 12;
/// Prompt lines shown before "Show all".
const PROMPT_CLAMP: usize = 20;

/// What a pane keeps about how its transcript is shown.
#[derive(Default)]
pub struct ViewState {
    /// Disclosures the user opened or closed, by id; the rest follow their default.
    pub toggled: HashMap<String, bool>,
    /// Clamped blocks the user asked to see in full.
    pub unclamped: HashSet<String>,
    /// The turn whose answer was just copied.
    pub copied: Option<String>,
}

pub type OnToggle = Arc<dyn Fn(String, bool, &mut App)>;
pub type OnUnclamp = Arc<dyn Fn(String, &mut App)>;
pub type OnCopy = Arc<dyn Fn(String, String, &mut App)>;

/// What rendering a turn needs, and how its rows talk back to the pane.
pub struct Ctx<'a> {
    pub p: &'a Palette,
    pub view: &'a ViewState,
    pub args_text: &'a HashMap<String, String>,
    pub on_toggle: OnToggle,
    pub on_unclamp: OnUnclamp,
    pub on_copy: OnCopy,
    /// Draws an approval card under its tool row.
    pub approval: &'a dyn Fn(&Approval) -> AnyElement,
}

impl Ctx<'_> {
    fn is_open(&self, id: &str, default: bool) -> bool {
        self.view.toggled.get(id).copied().unwrap_or(default)
    }
}

pub fn turn_view(turn: &Turn, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    let summary = if turn.live {
        None
    } else {
        summarize_work(&turn.work)
    };
    let group = SharedString::from(format!("turn-{}", turn.key));
    let mut reply = div().flex().flex_col().gap(px(4.)).pt(px(4.));
    reply = match summary {
        Some((title, detail)) => {
            let images = tool_images(&turn.work);
            let id = format!("{}:folded", turn.key);
            let open = ctx.is_open(&id, false);
            let closed = (!images.is_empty()).then(|| tool_images_row(&images, p));
            reply.child(disclosure(
                &id,
                "history",
                &title,
                false,
                Some(detail),
                Vec::new(),
                open,
                ctx,
                || {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .border_l_1()
                        .border_color(p.line)
                        .pl(px(12.))
                        .children(turn.work.iter().map(|s| step_view(s, false, ctx)))
                        .into_any_element()
                },
                closed,
            ))
        }
        None => reply.children(turn.work.iter().map(|s| step_view(s, turn.live, ctx))),
    };
    reply = reply.children(turn.answer.iter().map(|s| step_view(s, turn.live, ctx)));
    if turn.stopped {
        reply = reply.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(p.ink_muted)
                .child(icon("square", 14.))
                .child("Stopped"),
        );
    }
    div()
        .group(group.clone())
        .flex()
        .flex_col()
        .gap(px(4.))
        .children(turn.prompt.as_ref().map(|m| prompt_view(&turn.key, m, ctx)))
        .child(reply)
        .children((!turn.live).then(|| footer(turn, group, ctx)).flatten())
        .into_any_element()
}

fn prompt_view(turn_key: &str, message: &Message, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    let text = message
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text { text } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let images: Vec<(&str, Option<&str>)> = message
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Image { mime_type, data } => Some((mime_type.as_str(), data.as_deref())),
            _ => None,
        })
        .collect();
    let shown: Vec<(&str, &str)> = images
        .iter()
        .filter_map(|(m, d)| d.map(|d| (*m, d)))
        .collect();
    let hidden = images.len() - shown.len();
    let lines: Vec<&str> = text.split('\n').collect();
    let id = format!("{turn_key}:prompt");
    let clamped = !ctx.view.unclamped.contains(&id) && lines.len() > PROMPT_CLAMP;
    let body = if clamped {
        lines[..PROMPT_CLAMP].join("\n")
    } else {
        text.clone()
    };
    div()
        .max_w(relative(0.85))
        .self_end()
        .rounded(px(6.))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_raised)
        .px(px(12.))
        .py(px(8.))
        .t_body()
        .child(selectable_plain(&id, body))
        .when(clamped, |el| el.child(show_all(&id, lines.len(), ctx)))
        .when(!shown.is_empty(), |el| {
            el.child(
                div()
                    .mt(px(8.))
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .children(shown.iter().map(|(m, d)| thumbnail(m, d, p))),
            )
        })
        .when(hidden > 0, |el| {
            el.child(
                div()
                    .mt(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .t_caption()
                    .text_color(p.ink_muted)
                    .child(icon("file", 14.))
                    .child(if hidden == 1 {
                        "1 IMAGE".to_string()
                    } else {
                        format!("{hidden} IMAGES")
                    }),
            )
        })
        .into_any_element()
}

/// Plain text that keeps its line breaks and can be selected and copied.
fn selectable_plain(id: &str, text: String) -> AnyElement {
    // Markdown would reflow a prompt; a fenced-free paragraph per line keeps
    // it as typed. Hard breaks keep every newline.
    let escaped = text
        .split('\n')
        .map(escape_markdown)
        .collect::<Vec<_>>()
        .join("  \n");
    TextView::markdown(SharedString::from(format!("{id}:text")), escaped)
        .selectable(true)
        .into_any_element()
}

fn escape_markdown(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if "\\`*_{}[]()#+-.!|<>~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    if out.is_empty() { "\u{a0}".into() } else { out }
}

/// Assistant text as markdown, on the design system's type scale.
pub fn markdown(id: &str, text: &str, muted: bool, p: &Palette) -> AnyElement {
    let mut code = StyleRefinement::default()
        .bg(p.surface_sunken)
        .border_1()
        .border_color(p.line)
        .rounded(px(6.))
        .p(px(12.));
    code.text.font_family = Some(FONT_MONO.into());
    code.text.font_size = Some(px(12.5).into());
    code.text.line_height = Some(px(19.).into());
    let style = TextViewStyle::default()
        .paragraph_gap(rems(0.5))
        .heading_font_size(|_, _| px(14.))
        .code_block(code)
        .inline_code(HighlightStyle {
            background_color: Some(p.surface_raised),
            ..Default::default()
        });
    div()
        .w_full()
        .t_body()
        .when(muted, |el| el.text_color(p.ink_muted))
        .child(
            TextView::markdown(SharedString::from(id.to_string()), text.to_string())
                .style(style)
                .selectable(true)
                .on_link_click(|url, _, _, cx| cx.open_url(url)),
        )
        .into_any_element()
}

fn step_view(step: &Step, live: bool, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    match step {
        Step::Text { key, text } => div()
            .max_w(relative(0.85))
            .self_start()
            .rounded(px(6.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface_raised)
            .px(px(12.))
            .py(px(8.))
            .child(markdown(key, text, false, p))
            .into_any_element(),
        Step::Thinking {
            key,
            text,
            duration_ms,
            live,
        } => thinking(key, text, *duration_ms, *live, ctx),
        Step::Tool(step) => tool_call(step, live, ctx),
        Step::Group { key, steps } => {
            let searching = steps.iter().any(|s| s.call.kind == ToolKind::Search);
            let subject = steps
                .iter()
                .map(|s| {
                    s.call
                        .subject
                        .clone()
                        .unwrap_or_else(|| s.call.name.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            let running = live && steps.iter().any(|s| s.call.status != ToolStatus::Done);
            let open = ctx.is_open(key, false);
            disclosure(
                key,
                if searching { "search" } else { "file" },
                &group_label(steps),
                false,
                Some(subject),
                if running {
                    vec![status_pill(SessionStatus::Running, p).into_any_element()]
                } else {
                    Vec::new()
                },
                open,
                ctx,
                || {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .children(steps.iter().map(|s| tool_call(s, live, ctx)))
                        .into_any_element()
                },
                None,
            )
        }
        Step::Note { text, .. } => div()
            .border_l_2()
            .border_color(p.line_strong)
            .py(px(4.))
            .pl(px(12.))
            .text_color(p.ink_muted)
            .child(text.clone())
            .into_any_element(),
        Step::Image {
            mime_type, data, ..
        } => match data {
            Some(data) => div()
                .py(px(4.))
                .child(thumbnail(mime_type, data, p))
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .gap(px(4.))
                .t_caption()
                .text_color(p.ink_muted)
                .child(icon("file", 14.))
                .child(format!("IMAGE ({})", mime_type.to_uppercase()))
                .into_any_element(),
        },
        Step::Error { text, .. } => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .py(px(4.))
            .text_color(p.error)
            .child(
                div()
                    .mt(px(3.))
                    .flex_none()
                    .child(icon("circle-alert", 14.)),
            )
            .child(div().child(text.clone()))
            .into_any_element(),
    }
}

fn thinking(key: &str, text: &str, duration_ms: Option<i64>, live: bool, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    if live {
        let last = text
            .trim()
            .split('\n')
            .next_back()
            .unwrap_or("")
            .to_string();
        return div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(4.))
            .text_color(p.ink_muted)
            .child(loop_mark(
                SharedString::from(format!("{key}:mark")),
                16.,
                LoopState::Thinking,
                p.ink_muted,
            ))
            .child(div().flex_none().text_color(p.ink).child("Thinking"))
            .child(div().min_w_0().truncate().child(last))
            .into_any_element();
    }
    let title = match duration_ms {
        None => "Thought".to_string(),
        Some(ms) => format!("Thought for {}", format_duration(ms)),
    };
    let open = ctx.is_open(key, false);
    disclosure(
        key,
        "eye",
        &title,
        false,
        None,
        Vec::new(),
        open,
        ctx,
        || {
            div()
                .pl(px(24.))
                .child(markdown(&format!("{key}:md"), text, true, p))
                .into_any_element()
        },
        None,
    )
}

fn tool_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "file",
        ToolKind::Edit => "file-diff",
        ToolKind::Search => "search",
        ToolKind::Execute => "terminal",
        ToolKind::Other => "command",
    }
}

fn tool_call(step: &ToolStep, live: bool, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    let call = &step.call;
    let status = call.status;
    let running = status == ToolStatus::Running || (live && status == ToolStatus::Pending);
    let diffs: Vec<Vec<DiffLine>> = edits_of(call).iter().map(edit_diff).collect();
    let stat = (!diffs.is_empty()).then(|| diff_stat(&diffs));
    let mut right: Vec<AnyElement> = Vec::new();
    if let Some((added, removed)) = stat {
        right.push(
            div()
                .flex()
                .gap(px(4.))
                .t_code_sm()
                .child(div().text_color(p.done).child(format!("+{added}")))
                .child(div().text_color(p.error).child(format!("−{removed}")))
                .into_any_element(),
        );
    }
    if let Some(ms) = step.duration_ms {
        right.push(
            div()
                .t_code_sm()
                .text_color(p.ink_muted)
                .child(format_duration(ms))
                .into_any_element(),
        );
    }
    if step.approval.is_some() {
        right.push(status_pill(SessionStatus::Waiting, p).into_any_element());
    } else if running {
        right.push(status_pill(SessionStatus::Running, p).into_any_element());
    } else if status == ToolStatus::Failed {
        right.push(status_pill(SessionStatus::Failed, p).into_any_element());
    }
    // Running calls show their output as it comes, failures stay in view,
    // and edits show their diff; the rest open on request.
    let default_open = running || status == ToolStatus::Failed || call.kind == ToolKind::Edit;
    let open = ctx.is_open(&step.key, default_open);
    let row = disclosure(
        &step.key,
        tool_icon(call.kind),
        &call.name,
        true,
        call.subject.clone(),
        right,
        open,
        ctx,
        || {
            div()
                .pl(px(24.))
                .child(tool_body(step, running, &diffs, ctx))
                .into_any_element()
        },
        None,
    );
    if step.approval.is_none() && call.images.is_empty() {
        return row;
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(row)
        // Images stay in view, like an approval: what the agent saw is the point.
        .when(!call.images.is_empty(), |el| {
            el.child(tool_images_row(&call.images, p))
        })
        .children(
            step.approval
                .as_ref()
                .map(|a| div().pl(px(24.)).child((ctx.approval)(a))),
        )
        .into_any_element()
}

fn tool_body(step: &ToolStep, running: bool, diffs: &[Vec<DiffLine>], ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    let call = &step.call;
    let output = output_text(call);
    if call.kind == ToolKind::Edit && !diffs.is_empty() {
        let edits = edits_of(call);
        return div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(diffs.iter().enumerate().map(|(i, lines)| {
                let id = format!("{}:diff:{i}", step.key);
                clamp(&id, lines.len(), Clamp::Head, CLAMP * 3, ctx, |range| {
                    diff_view(&edits[i].path, lines, range, p)
                })
            }))
            .when(
                call.status == ToolStatus::Failed && !output.is_empty(),
                |el| el.child(output_view(&format!("{}:out", step.key), output, ctx)),
            )
            .into_any_element();
    }
    if call.kind == ToolKind::Execute {
        let mut lines: Vec<String> = if output.is_empty() {
            Vec::new()
        } else {
            output
                .strip_suffix('\n')
                .unwrap_or(output)
                .split('\n')
                .map(String::from)
                .collect()
        };
        if let Some(ToolOutput::Shell {
            exit_code: Some(code),
            ..
        }) = &call.output
            && *code != 0
        {
            lines.push(format!("exit {code}"));
        }
        let command = call.subject.clone().unwrap_or_else(|| call.name.clone());
        let id = format!("{}:term", step.key);
        return clamp(&id, lines.len(), Clamp::Tail, CLAMP, ctx, |range| {
            terminal(&command, &lines[range], running, p)
        });
    }
    let args = if call.kind == ToolKind::Other {
        if call.input.is_null() {
            ctx.args_text.get(&call.id).cloned().unwrap_or_default()
        } else {
            serde_json::to_string_pretty(&call.input).unwrap_or_default()
        }
    } else {
        String::new()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .when(!args.is_empty(), |el| {
            el.child(
                div()
                    .rounded(px(6.))
                    .bg(p.surface_sunken)
                    .p(px(12.))
                    .t_code()
                    .child(args),
            )
        })
        .when(!output.is_empty(), |el| {
            el.child(output_view(&format!("{}:out", step.key), output, ctx))
        })
        .into_any_element()
}

fn output_view(id: &str, text: &str, ctx: &Ctx) -> AnyElement {
    let p = ctx.p;
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect();
    clamp(id, lines.len(), Clamp::Head, CLAMP, ctx, |range| {
        div()
            .rounded(px(6.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface_sunken)
            .p(px(12.))
            .t_code()
            .child(lines[range].join("\n"))
            .into_any_element()
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clamp {
    Head,
    Tail,
}

/// Shows the first or last `limit` lines, with a button for the rest.
fn clamp(
    id: &str,
    count: usize,
    from: Clamp,
    limit: usize,
    ctx: &Ctx,
    render: impl FnOnce(std::ops::Range<usize>) -> AnyElement,
) -> AnyElement {
    let clamped = !ctx.view.unclamped.contains(id) && count > limit + 2;
    let range = match (clamped, from) {
        (false, _) => 0..count,
        (true, Clamp::Head) => 0..limit,
        (true, Clamp::Tail) => count - limit..count,
    };
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(4.))
        .when(clamped && from == Clamp::Tail, |el| {
            el.child(show_all(id, count, ctx))
        })
        .child(div().w_full().child(render(range)))
        .when(clamped && from == Clamp::Head, |el| {
            el.child(show_all(id, count, ctx))
        })
        .into_any_element()
}

fn show_all(id: &str, count: usize, ctx: &Ctx) -> AnyElement {
    let on_unclamp = ctx.on_unclamp.clone();
    let id = id.to_string();
    Button::new(SharedString::from(format!("{id}:all")))
        .ghost()
        .small()
        .label(format!("Show all {count} lines"))
        .on_click(move |_, _, cx| on_unclamp(id.clone(), cx))
        .into_any_element()
}

/// A quiet one-line row that opens to show more.
#[allow(clippy::too_many_arguments)]
fn disclosure(
    id: &str,
    icon_name: &'static str,
    title: &str,
    mono: bool,
    subject: Option<String>,
    right: Vec<AnyElement>,
    open: bool,
    ctx: &Ctx,
    body: impl FnOnce() -> AnyElement,
    closed: Option<AnyElement>,
) -> AnyElement {
    let p = ctx.p;
    let on_toggle = ctx.on_toggle.clone();
    let toggle_id = id.to_string();
    let raised = p.surface_raised;
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .id(SharedString::from(format!("{id}:row")))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .w_full()
                .min_w_0()
                .px(px(4.))
                .rounded(px(4.))
                .t_body()
                .text_color(p.ink_muted)
                .hover(move |s| s.bg(raised))
                .on_click(move |_, _, cx| on_toggle(toggle_id.clone(), !open, cx))
                .child(
                    icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        14.,
                    )
                    .text_color(p.ink_faint),
                )
                .child(icon(icon_name, 14.))
                .child(
                    div()
                        .flex_none()
                        .when(mono, |el| el.t_code_strong().text_color(p.ink))
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .t_code_sm()
                        .child(subject.unwrap_or_default()),
                )
                .children(right),
        )
        .map(|el| {
            if open {
                el.child(body())
            } else {
                el.children(closed)
            }
        })
        .into_any_element()
}

fn tool_images_row(images: &[ToolImage], p: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .gap(px(8.))
        .py(px(4.))
        .pl(px(24.))
        .children(images.iter().map(|i| thumbnail(&i.mime_type, &i.data, p)))
        .into_any_element()
}

thread_local! {
    /// Decoded images by a hash of their data, so a frame never decodes twice.
    static IMAGES: RefCell<HashMap<u64, Option<Arc<Image>>>> = RefCell::new(HashMap::new());
}

/// An image from the conversation, decoded once.
pub(crate) fn decoded(mime_type: &str, data: &str) -> Option<Arc<Image>> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let key = hasher.finish();
    IMAGES.with(|cache| {
        cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let format = ImageFormat::from_mime_type(mime_type)?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .ok()?;
                Some(Arc::new(Image::from_bytes(format, bytes)))
            })
            .clone()
    })
}

fn thumbnail(mime_type: &str, data: &str, p: &Palette) -> AnyElement {
    let frame = div()
        .rounded(px(6.))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_sunken)
        .overflow_hidden();
    match decoded(mime_type, data) {
        Some(image) => frame
            .child(
                img(image)
                    .max_h(px(160.))
                    .max_w(px(240.))
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
        None => frame
            .p(px(8.))
            .t_caption()
            .text_color(p.ink_muted)
            .child(format!("IMAGE ({})", mime_type.to_uppercase()))
            .into_any_element(),
    }
}

/// The design system's Terminal: the command, then its output.
fn terminal(command: &str, lines: &[String], live: bool, p: &Palette) -> AnyElement {
    let _ = live;
    div()
        .rounded(px(6.))
        .border_1()
        .border_color(p.line)
        .bg(p.surface_sunken)
        .overflow_hidden()
        .text_color(p.ink)
        .child(
            div()
                .p(px(12.))
                .t_code()
                .child(
                    div()
                        .flex()
                        .child(div().text_color(p.ink_muted).child("~"))
                        .child(div().text_color(p.running).child(" ❯ "))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(command.to_string()),
                        ),
                )
                .children(
                    lines
                        .iter()
                        .map(|l| div().child(if l.is_empty() { " ".into() } else { l.clone() })),
                ),
        )
        .into_any_element()
}

/// The design system's Diff: a file header, then numbered lines.
fn diff_view(
    path: &str,
    lines: &[DiffLine],
    range: std::ops::Range<usize>,
    p: &Palette,
) -> AnyElement {
    let added = lines.iter().filter(|l| l.kind == LineKind::Add).count();
    let removed = lines.iter().filter(|l| l.kind == LineKind::Remove).count();
    div()
        .rounded(px(6.))
        .border_1()
        .border_color(p.line)
        .bg(p.bg)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(28.))
                .px(px(12.))
                .bg(p.surface)
                .border_b_1()
                .border_color(p.line)
                .t_code_sm()
                .text_color(p.ink_muted)
                .child(icon("file-diff", 14.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(p.ink)
                        .child(path.to_string()),
                )
                .child(div().text_color(p.done).child(format!("+{added}")))
                .child(div().text_color(p.error).child(format!("−{removed}"))),
        )
        .children(lines[range].iter().map(|line| {
            match line.kind {
                LineKind::Hunk => div()
                    .px(px(12.))
                    .py(px(2.))
                    .bg(p.surface)
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .child(line.text.clone())
                    .into_any_element(),
                kind => {
                    let (bg, sign) = match kind {
                        LineKind::Add => (Some(p.diff_add), "+"),
                        LineKind::Remove => (Some(p.diff_remove), "−"),
                        _ => (None, " "),
                    };
                    div()
                        .flex()
                        .t_code()
                        .text_color(p.ink)
                        .when_some(bg, |el, bg| el.bg(bg))
                        .child(
                            div()
                                .w(px(16.))
                                .flex_none()
                                .text_center()
                                .text_color(if bg.is_some() { p.ink } else { p.ink_muted })
                                .child(sign),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pr(px(12.))
                                .child(line.text.clone()),
                        )
                        .into_any_element()
                }
            }
        }))
        .into_any_element()
}

fn footer(turn: &Turn, group: SharedString, ctx: &Ctx) -> Option<AnyElement> {
    let p = ctx.p;
    let text = answer_text(turn);
    let mut parts: Vec<String> = Vec::new();
    if let Some(start) = turn.started_at {
        parts.push(local_time(start));
        if let Some(end) = turn.ended_at {
            parts.push(format_duration(end - start));
        }
    }
    if let Some(usage) = &turn.usage {
        parts.extend(turn_usage_parts(usage));
    }
    if parts.is_empty() && text.is_empty() {
        return None;
    }
    let copied = ctx.view.copied.as_deref() == Some(turn.key.as_str());
    let on_copy = ctx.on_copy.clone();
    let key = turn.key.clone();
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .h(px(24.))
            .opacity(0.)
            .group_hover(group, |s| s.opacity(1.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .child(parts.join(" · ")),
            )
            .when(!text.is_empty(), |el| {
                el.child(
                    Button::new(SharedString::from(format!("{key}:copy")))
                        .ghost()
                        .small()
                        .icon(if copied { "check" } else { "copy" })
                        .label(if copied { "Copied" } else { "Copy" })
                        .on_click(move |_, _, cx| on_copy(key.clone(), text.clone(), cx)),
                )
            })
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn prompts_keep_their_text_when_escaped() {
        assert_eq!(escape_markdown("a *b* #1"), "a \\*b\\* \\#1");
        assert_eq!(escape_markdown(""), "\u{a0}");
    }
}
