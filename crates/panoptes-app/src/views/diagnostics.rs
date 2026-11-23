//! A local-only observability pane for sessions and the event log.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{Local, TimeZone as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_harness::SessionStatus;
use serde_json::Value;

use crate::ds::{self, Button, Tab, Type as _, icon};
use crate::model::diagnostics::{
    Diagnostics, DiagnosticsFilter, FileChange, SessionDiagnostics, ToolSummary, analyze,
    diagnostic_event_indices, event_summary, lane_filter_groups, project_filter_groups,
};
use crate::model::format::{format_tokens, relative_time};
use crate::model::usage::format_cost;
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::Ui;
use crate::views::picker::{Picker, PickerEvent, PickerState};
use crate::zoom::px;

const REFRESH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Activity,
    Tools,
    Changes,
    Raw,
}

enum TreeRow {
    Project(String),
    Lane { title: String, branch: String },
    Session { key: String, depth: usize },
}

pub struct DiagnosticsPane {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    filter: DiagnosticsFilter,
    project_picker: Entity<PickerState>,
    lane_picker: Entity<PickerState>,
    events: Vec<Value>,
    analysis: Diagnostics,
    analysis_dirty: bool,
    errors: Vec<String>,
    selected_key: Option<String>,
    selected_event: Option<usize>,
    search: Entity<InputState>,
    tab: DetailTab,
    loading: bool,
    _clock: Task<()>,
}

fn timestamp(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| "Unknown time".into())
}

fn duration(ms: i64) -> String {
    if ms < 1_000 {
        format!("{} ms", ms.max(0))
    } else if ms < 60_000 {
        format!("{:.2} s", ms as f64 / 1_000.)
    } else {
        format!("{:.1} min", ms as f64 / 60_000.)
    }
}

fn session_matches(session: &SessionDiagnostics, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let metadata = format!(
        "{} {} {} {} {} {} {}",
        session.key,
        session.title,
        session.project.as_deref().unwrap_or_default(),
        session.lane.as_deref().unwrap_or_default(),
        session.branch.as_deref().unwrap_or_default(),
        session.models.join(" "),
        session.cwd.as_deref().unwrap_or_default(),
    )
    .to_lowercase();
    metadata.contains(query)
        || session.tools.values().any(|tool| {
            format!("{} {} {} {}", tool.id, tool.name, tool.kind, tool.status)
                .to_lowercase()
                .contains(query)
        })
        || session
            .changes
            .keys()
            .any(|path| path.to_lowercase().contains(query))
}

fn append_session_rows(
    rows: &mut Vec<TreeRow>,
    session: &SessionDiagnostics,
    members: &[&SessionDiagnostics],
    depth: usize,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(session.key.clone()) {
        return;
    }
    rows.push(TreeRow::Session {
        key: session.key.clone(),
        depth,
    });
    let mut children: Vec<_> = members
        .iter()
        .copied()
        .filter(|candidate| candidate.parent_key.as_deref() == Some(session.key.as_str()))
        .collect();
    children.sort_by(|a, b| a.title.cmp(&b.title));
    for child in children {
        append_session_rows(rows, child, members, depth + 1, visited);
    }
}

fn session_rows(members: &[&SessionDiagnostics], query: &str) -> Vec<TreeRow> {
    let visible: Vec<_> = members
        .iter()
        .copied()
        .filter(|session| session_matches(session, query))
        .collect();
    let keys: HashSet<_> = visible.iter().map(|session| session.key.as_str()).collect();
    let mut roots: Vec<_> = visible
        .iter()
        .copied()
        .filter(|session| {
            session
                .parent_key
                .as_deref()
                .is_none_or(|parent| !keys.contains(parent))
        })
        .collect();
    roots.sort_by(|a, b| a.title.cmp(&b.title));
    let mut rows = Vec::new();
    let mut visited = HashSet::new();
    for session in roots {
        append_session_rows(&mut rows, session, &visible, 0, &mut visited);
    }
    // A malformed parent cycle must not make a session disappear.
    for session in visible {
        if !visited.contains(&session.key) {
            append_session_rows(&mut rows, session, members, 0, &mut visited);
        }
    }
    rows
}

fn tree_rows(diagnostics: &Diagnostics, query: &str, filter: DiagnosticsFilter) -> Vec<TreeRow> {
    let mut project_ids: Vec<_> = diagnostics
        .sessions
        .iter()
        .filter(|session| filter.matches(session))
        .filter_map(|session| session.project_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    project_ids.sort_unstable();
    let mut rows = Vec::new();
    for project_id in project_ids {
        let members: Vec<_> = diagnostics
            .sessions
            .iter()
            .filter(|session| filter.matches(session) && session.project_id == Some(project_id))
            .collect();
        let mut project_rows = Vec::new();
        let mut lane_ids: Vec<_> = members
            .iter()
            .filter_map(|session| session.lane_id)
            .collect();
        lane_ids.sort_unstable();
        lane_ids.dedup();
        for lane_id in lane_ids {
            let lane_members: Vec<_> = members
                .iter()
                .copied()
                .filter(|session| session.lane_id == Some(lane_id))
                .collect();
            let lane_rows = session_rows(&lane_members, query);
            if !lane_rows.is_empty() {
                let first = lane_members[0];
                project_rows.push(TreeRow::Lane {
                    title: first
                        .lane
                        .clone()
                        .unwrap_or_else(|| format!("Lane {lane_id}")),
                    branch: first.branch.clone().unwrap_or_default(),
                });
                project_rows.extend(lane_rows);
            }
        }
        let direct: Vec<_> = members
            .iter()
            .copied()
            .filter(|session| session.lane_id.is_none())
            .collect();
        project_rows.extend(session_rows(&direct, query));
        if !project_rows.is_empty() {
            let title = members[0]
                .project
                .clone()
                .unwrap_or_else(|| format!("Project {project_id}"));
            rows.push(TreeRow::Project(title));
            rows.extend(project_rows);
        }
    }
    let other: Vec<_> = diagnostics
        .sessions
        .iter()
        .filter(|session| filter.matches(session) && session.project_id.is_none())
        .collect();
    let other_rows = session_rows(&other, query);
    if !other_rows.is_empty() {
        rows.push(TreeRow::Project("Other sessions".into()));
        rows.extend(other_rows);
    }
    rows
}

impl DiagnosticsPane {
    pub fn new(
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        filter: DiagnosticsFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let project_picker = cx.new(|cx| PickerState::new("Project", window, cx));
        cx.subscribe(&project_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(id) = event {
                this.filter = DiagnosticsFilter {
                    project_id: id.parse().ok(),
                    lane_id: None,
                };
                this.selected_key = None;
                this.selected_event = None;
                cx.notify();
            }
        })
        .detach();
        let lane_picker = cx.new(|cx| PickerState::new("Lane", window, cx));
        cx.subscribe(&lane_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(id) = event {
                let lane_id = id.parse().ok();
                this.filter.lane_id = lane_id;
                if let Some(lane_id) = lane_id {
                    this.filter.project_id = this
                        .store
                        .read(cx)
                        .state
                        .lanes
                        .iter()
                        .find(|lane| lane.id == lane_id)
                        .map(|lane| lane.project_id);
                }
                this.selected_key = None;
                this.selected_event = None;
                cx.notify();
            }
        })
        .detach();
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter sessions, tools, files or events")
        });
        cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.selected_event = None;
                cx.notify();
            }
        })
        .detach();
        cx.observe(&store, |this, _, cx| {
            this.analysis_dirty = true;
            cx.notify();
        })
        .detach();
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            store,
            ui,
            filter,
            project_picker,
            lane_picker,
            events: Vec::new(),
            analysis: Diagnostics::default(),
            analysis_dirty: true,
            errors: Vec::new(),
            selected_key: None,
            selected_event: None,
            search,
            tab: DetailTab::Activity,
            loading: false,
            _clock: clock,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let backend = self.store.read(cx).backend().clone();
        let read = backend.read_logs();
        cx.spawn(async move |this, cx| {
            let snapshot = read.await;
            let _ = this.update(cx, |this, cx| {
                this.events = snapshot.events;
                this.analysis_dirty = true;
                this.errors = snapshot.errors;
                this.loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn reanalyze(&mut self, cx: &mut Context<Self>) {
        if !self.analysis_dirty {
            return;
        }
        let state = &self.store.read(cx).state;
        self.analysis = analyze(&state.sessions, &state.projects, &state.lanes, &self.events);
        self.analysis_dirty = false;
    }

    fn select_session(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        self.selected_key = key;
        self.selected_event = None;
        self.tab = DetailTab::Activity;
        cx.notify();
    }

    fn filtered_events(&self, diagnostics: &Diagnostics, query: &str) -> Vec<usize> {
        let indices = diagnostic_event_indices(
            diagnostics,
            self.events.len(),
            self.filter,
            self.selected_key.as_deref(),
        );
        indices
            .into_iter()
            .rev()
            .filter(|index| {
                query.is_empty()
                    || self.events[*index]
                        .to_string()
                        .to_lowercase()
                        .contains(query)
            })
            .collect()
    }

    fn raw_panel(&self, event: Option<&Value>, p: &Palette) -> Div {
        let body = event
            .map(|event| serde_json::to_string_pretty(event).unwrap_or_default())
            .unwrap_or_else(|| "Select an event to inspect its complete JSON record.".into());
        let copy = event.map(|event| serde_json::to_string_pretty(event).unwrap_or_default());
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pb(px(8.))
                    .child(div().t_body_strong().child("Raw event"))
                    .child(
                        Button::new("copy-diagnostic")
                            .ghost()
                            .small()
                            .icon("copy")
                            .label("Copy JSON")
                            .disabled(copy.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(copy) = &copy {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .id("raw-diagnostic")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(12.))
                    .rounded(px(ds::RADIUS_MD))
                    .border_1()
                    .border_color(p.line)
                    .bg(p.surface_sunken)
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .child(body),
            )
    }

    fn activity(
        &self,
        diagnostics: &Diagnostics,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let indices = self.filtered_events(diagnostics, query);
        let mut list = div()
            .id("diagnostic-events")
            .h(px(360.))
            .flex_none()
            .overflow_y_scroll()
            .border_1()
            .border_color(p.line)
            .rounded(px(ds::RADIUS_MD));
        if indices.is_empty() {
            list = list.child(div().p(px(20.)).t_body().text_color(p.ink_muted).child(
                if self.events.is_empty() {
                    "No diagnostic events yet. Activity is recorded as sessions run."
                } else {
                    "No events match this filter."
                },
            ));
        }
        for index in &indices {
            let event = &self.events[*index];
            let index = *index;
            let event_name = event
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let failed = event_name == "error"
                || event_name == "sessionExited"
                || event.get("status").and_then(Value::as_str) == Some("failed")
                || event.get("failed").and_then(Value::as_bool) == Some(true);
            let selected = self.selected_event == Some(index);
            let this = cx.entity().downgrade();
            let at = event.get("at").and_then(Value::as_i64).unwrap_or(0);
            let key = event.get("key").and_then(Value::as_str);
            list = list.child(
                div()
                    .id(SharedString::from(format!("diagnostic-event-{index}")))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(p.line)
                    .cursor_pointer()
                    .when(selected, |row| row.bg(p.selection))
                    .hover(move |row| {
                        row.bg(if selected {
                            p.selection
                        } else {
                            p.surface_raised
                        })
                    })
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.selected_event = Some(index);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .t_code_sm()
                                    .text_color(p.ink_muted)
                                    .child(timestamp(at)),
                            )
                            .child(
                                div()
                                    .t_code_strong()
                                    .text_color(if failed { p.error } else { p.ink })
                                    .child(event_name),
                            )
                            .children(key.map(|key| {
                                div()
                                    .t_code_sm()
                                    .text_color(p.ink_faint)
                                    .child(key.to_string())
                            })),
                    )
                    .child(
                        div()
                            .t_small()
                            .text_color(p.ink_muted)
                            .child(event_summary(event)),
                    ),
            );
        }
        let selected = self.selected_event.and_then(|index| self.events.get(index));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .t_caption()
                    .text_color(p.ink_muted)
                    .child(format!("{} events · newest first", indices.len())),
            )
            .child(list)
            .child(self.raw_panel(selected, p))
    }

    fn tool_row(
        &self,
        session: &SessionDiagnostics,
        tool: &ToolSummary,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let color = match tool.status.as_str() {
            "done" => p.done,
            "failed" => p.error,
            "running" | "pending" => p.running,
            _ => p.ink_muted,
        };
        let tool_id = tool.id.clone();
        let event = session.event_indices.iter().rev().copied().find(|index| {
            self.events[*index].get("event").and_then(Value::as_str) == Some("toolCall")
                && self.events[*index].get("id").and_then(Value::as_str) == Some(tool.id.as_str())
        });
        let this = cx.entity().downgrade();
        div()
            .id(SharedString::from(format!(
                "tool-{}-{}",
                session.key, tool.id
            )))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(10.))
            .border_b_1()
            .border_color(p.line)
            .cursor_pointer()
            .hover(move |row| row.bg(p.surface_raised))
            .on_click(move |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.selected_event = event;
                    this.tab = DetailTab::Raw;
                    cx.notify();
                });
            })
            .child(
                icon(
                    if tool.status == "failed" {
                        "circle-alert"
                    } else {
                        "terminal"
                    },
                    16.,
                )
                .text_color(color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().t_body_strong().child(tool.name.clone()))
                    .child(
                        div()
                            .t_code_sm()
                            .text_color(p.ink_muted)
                            .child(format!("{} · {}", tool.kind, tool_id)),
                    ),
            )
            .child(
                div()
                    .t_small()
                    .text_color(p.ink_muted)
                    .child(format!("{} updates", tool.updates)),
            )
            .children(tool.duration_ms().map(|ms| {
                div()
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .child(duration(ms))
            }))
            .child(
                div()
                    .min_w(px(64.))
                    .t_label()
                    .text_color(color)
                    .child(tool.status.clone()),
            )
    }

    fn tools(
        &self,
        session: Option<&SessionDiagnostics>,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut body = div()
            .id("diagnostic-tools")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .border_1()
            .border_color(p.line)
            .rounded(px(ds::RADIUS_MD));
        let Some(session) = session else {
            return body.child(
                self.select_session_message("Select a session to inspect its tool calls.", p),
            );
        };
        let mut tools: Vec<_> = session
            .tools
            .values()
            .filter(|tool| {
                query.is_empty()
                    || format!("{} {} {} {}", tool.id, tool.name, tool.kind, tool.status)
                        .to_lowercase()
                        .contains(query)
            })
            .collect();
        tools.sort_by_key(|tool| std::cmp::Reverse(tool.ended_at.or(tool.started_at).unwrap_or(0)));
        if tools.is_empty() {
            body = body.child(self.select_session_message(
                "No tool calls match. New tool activity is recorded automatically.",
                p,
            ));
        }
        for tool in tools {
            body = body.child(self.tool_row(session, tool, p, cx));
        }
        body
    }

    fn change_row(
        &self,
        session: &SessionDiagnostics,
        change: &FileChange,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let event = session.event_indices.iter().rev().copied().find(|index| {
            self.events[*index].get("event").and_then(Value::as_str) == Some("toolCall")
                && self.events[*index].get("id").and_then(Value::as_str)
                    == Some(change.tool_call_id.as_str())
        });
        let this = cx.entity().downgrade();
        div()
            .id(SharedString::from(format!(
                "change-{}-{}",
                session.key, change.path
            )))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .py(px(10.))
            .border_b_1()
            .border_color(p.line)
            .cursor_pointer()
            .hover(move |row| row.bg(p.surface_raised))
            .on_click(move |_, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.selected_event = event;
                    this.tab = DetailTab::Raw;
                    cx.notify();
                });
            })
            .child(icon("file-diff", 16.).text_color(p.diff_add))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().t_code().child(change.path.clone()))
                    .child(div().t_small().text_color(p.ink_muted).child(format!(
                        "{} · call {}",
                        change.tool_name, change.tool_call_id
                    ))),
            )
            .child(
                div()
                    .t_code_sm()
                    .text_color(p.ink_muted)
                    .child(timestamp(change.at)),
            )
    }

    fn changes(
        &self,
        session: Option<&SessionDiagnostics>,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let mut body = div()
            .id("diagnostic-changes")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .border_1()
            .border_color(p.line)
            .rounded(px(ds::RADIUS_MD));
        let Some(session) = session else {
            return body.child(self.select_session_message(
                "Select a session to see exactly which files it changed.",
                p,
            ));
        };
        let mut changes: Vec<_> = session
            .changes
            .values()
            .filter(|change| query.is_empty() || change.path.to_lowercase().contains(query))
            .collect();
        changes.sort_by_key(|change| std::cmp::Reverse(change.at));
        if changes.is_empty() {
            body = body.child(self.select_session_message(
                "No edited paths match. Successful edit calls record their paths here.",
                p,
            ));
        }
        for change in changes {
            body = body.child(self.change_row(session, change, p, cx));
        }
        body
    }

    fn select_session_message(&self, message: &'static str, p: &Palette) -> Div {
        div()
            .p(px(20.))
            .t_body()
            .text_color(p.ink_muted)
            .child(message)
    }

    fn session_tree(
        &self,
        diagnostics: &Diagnostics,
        query: &str,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let event_count =
            diagnostic_event_indices(diagnostics, self.events.len(), self.filter, None).len();
        let mut tree = div()
            .id("diagnostic-sessions")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(px(8.));
        let all_this = cx.entity().downgrade();
        tree = tree.child(
            div()
                .id("all-diagnostic-events")
                .mx(px(8.))
                .px(px(8.))
                .py(px(7.))
                .rounded(px(ds::RADIUS_SM))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .when(self.selected_key.is_none(), |row| row.bg(p.selection))
                .on_click(move |_, _, cx| {
                    let _ = all_this.update(cx, |this, cx| this.select_session(None, cx));
                })
                .child(icon("history", 14.).text_color(p.ink_muted))
                .child(div().t_body_strong().child("All activity"))
                .child(
                    div()
                        .ml_auto()
                        .t_code_sm()
                        .text_color(p.ink_muted)
                        .child(event_count.to_string()),
                ),
        );
        for row in tree_rows(diagnostics, query, self.filter) {
            match row {
                TreeRow::Project(title) => {
                    tree = tree.child(
                        div()
                            .px(px(16.))
                            .pt(px(16.))
                            .pb(px(5.))
                            .t_caption()
                            .text_color(p.ink_muted)
                            .child(title),
                    );
                }
                TreeRow::Lane { title, branch } => {
                    tree = tree.child(
                        div()
                            .px(px(16.))
                            .py(px(5.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon("git-branch", 12.).text_color(p.ink_muted))
                            .child(div().t_label().child(title))
                            .child(div().t_code_sm().text_color(p.ink_faint).child(branch)),
                    );
                }
                TreeRow::Session { key, depth } => {
                    let Some(session) = diagnostics.get(&key) else {
                        continue;
                    };
                    let selected = self.selected_key.as_deref() == Some(key.as_str());
                    let this = cx.entity().downgrade();
                    let status = self
                        .store
                        .read(cx)
                        .state
                        .statuses
                        .get(&key)
                        .copied()
                        .unwrap_or(session.status);
                    let color = match status {
                        SessionStatus::Failed => p.error,
                        SessionStatus::Running => p.running,
                        SessionStatus::Waiting => p.waiting,
                        SessionStatus::Done => p.done,
                        SessionStatus::Idle => p.ink_faint,
                    };
                    let fork = session.parent_key.is_some();
                    tree = tree.child(
                        div()
                            .id(SharedString::from(format!("diagnostic-session-{key}")))
                            .ml(px(16. + depth as f32 * 16.))
                            .mr(px(8.))
                            .pl(px(if fork { 20. } else { 8. }))
                            .pr(px(8.))
                            .py(px(7.))
                            .rounded(px(ds::RADIUS_SM))
                            .relative()
                            .cursor_pointer()
                            .when(selected, |row| row.bg(p.selection))
                            .hover(move |row| {
                                row.bg(if selected {
                                    p.selection
                                } else {
                                    p.surface_raised
                                })
                            })
                            .on_click(move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.select_session(Some(key.clone()), cx)
                                });
                            })
                            .children(fork.then(|| {
                                div()
                                    .absolute()
                                    .left(px(5.))
                                    .top(px(0.))
                                    .bottom(px(0.))
                                    .w(px(1.))
                                    .bg(p.line_strong)
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(div().size(px(7.)).rounded_full().bg(color))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .t_body()
                                            .child(session.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .t_code_sm()
                                            .text_color(p.ink_muted)
                                            .child(format_tokens(session.total_tokens())),
                                    ),
                            )
                            .child(div().pl(px(14.)).t_small().text_color(p.ink_muted).child(
                                format!(
                                    "{} tools · {} changes · {}",
                                    session.tools.len(),
                                    session.changes.len(),
                                    relative_time(
                                        session.last_activity,
                                        crate::model::format::now_ms()
                                    )
                                ),
                            )),
                    );
                }
            }
        }
        tree
    }

    fn metric(label: &'static str, value: String, detail: String, p: &Palette) -> Div {
        div()
            .flex_1()
            .min_w(px(120.))
            .p(px(12.))
            .rounded(px(ds::RADIUS_MD))
            .border_1()
            .border_color(p.line)
            .bg(p.surface)
            .child(div().t_caption().text_color(p.ink_muted).child(label))
            .child(div().mt(px(3.)).t_title().child(value))
            .child(
                div()
                    .mt(px(2.))
                    .t_small()
                    .text_color(p.ink_muted)
                    .child(detail),
            )
    }

    fn detail_header(&self, session: Option<&SessionDiagnostics>, cx: &mut Context<Self>) -> Div {
        let p = *cx.palette();
        match session {
            Some(session) => {
                let state = &self.store.read(cx).state;
                let status = state
                    .statuses
                    .get(&session.key)
                    .copied()
                    .unwrap_or(session.status);
                let context = state
                    .settings
                    .get(&session.key)
                    .and_then(|settings| settings.context)
                    .map(|context| (context.tokens, Some(context.window)))
                    .unwrap_or((session.context_tokens, session.context_window));
                let context_value = match context {
                    (Some(tokens), Some(window)) => {
                        format!("{} / {}", format_tokens(tokens), format_tokens(window))
                    }
                    _ => "Unknown".into(),
                };
                let context_detail = match context {
                    (Some(tokens), Some(window)) if window > 0 => format!(
                        "{}% of the model window",
                        tokens.saturating_mul(100) / window
                    ),
                    _ => "Reported after a run settles".into(),
                };
                let input =
                    session.input_tokens + session.cache_read_tokens + session.cache_write_tokens;
                let token_detail = format!(
                    "{} in · {} out{}",
                    format_tokens(input),
                    format_tokens(session.output_tokens),
                    format_cost(session.cost)
                        .map(|cost| format!(" · {cost}"))
                        .unwrap_or_default()
                );
                div()
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().t_title().child(session.title.clone()))
                            .children(
                                (status != SessionStatus::Idle)
                                    .then(|| ds::status_pill(status, &p)),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .t_code_sm()
                            .text_color(p.ink_muted)
                            .child(session.key.clone()),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .t_small()
                            .text_color(p.ink_muted)
                            .child(format!(
                                "{}{}{}",
                                session.lane.as_deref().unwrap_or("No lane"),
                                session
                                    .branch
                                    .as_ref()
                                    .map(|branch| format!(" · {branch}"))
                                    .unwrap_or_default(),
                                session
                                    .parent_key
                                    .as_ref()
                                    .map(|parent| format!(" · Forked from {parent}"))
                                    .unwrap_or_default()
                            )),
                    )
                    .child(
                        div()
                            .mt(px(14.))
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(Self::metric(
                                "TOKENS",
                                format_tokens(session.total_tokens()),
                                token_detail,
                                &p,
                            ))
                            .child(Self::metric(
                                "MODEL CALLS",
                                session.model_calls.to_string(),
                                format!(
                                    "{} runs · {} failed",
                                    session.runs, session.failed_model_calls
                                ),
                                &p,
                            ))
                            .child(Self::metric(
                                "TOOL CALLS",
                                session.tools.len().to_string(),
                                format!(
                                    "{} succeeded · {} failed · {} active",
                                    session.successful_tools(),
                                    session.failed_tools(),
                                    session.running_tools()
                                ),
                                &p,
                            ))
                            .child(Self::metric("CONTEXT", context_value, context_detail, &p))
                            .child(Self::metric(
                                "FILES CHANGED",
                                session.changes.len().to_string(),
                                "From successful edit calls".into(),
                                &p,
                            )),
                    )
            }
            None => {
                let state = &self.store.read(cx).state;
                let title = self
                    .filter
                    .lane_id
                    .and_then(|id| state.lanes.iter().find(|lane| lane.id == id))
                    .map(|lane| lane.task.clone())
                    .or_else(|| {
                        self.filter
                            .project_id
                            .and_then(|id| state.projects.iter().find(|project| project.id == id))
                            .map(|project| project.settings.name.clone())
                    })
                    .unwrap_or_else(|| "All activity".into());
                let event_count =
                    diagnostic_event_indices(&self.analysis, self.events.len(), self.filter, None)
                        .len();
                div()
                    .flex_none()
                    .child(div().t_title().child(title))
                    .child(
                        div()
                            .mt(px(4.))
                            .t_body()
                            .text_color(p.ink_muted)
                            .child(format!(
                                "{} retained events in this view. Select a session for usage, tools, forks and file changes.",
                                event_count
                            )),
                    )
            }
        }
    }
    fn filters(&self, p: &Palette, cx: &App) -> Div {
        let state = &self.store.read(cx).state;
        let project = self.filter.project_id.and_then(|id| {
            state
                .projects
                .iter()
                .find(|project| project.id == id)
                .map(|project| project.settings.name.clone())
        });
        let lane = self.filter.lane_id.and_then(|id| {
            state
                .lanes
                .iter()
                .find(|lane| lane.id == id)
                .map(|lane| lane.task.clone())
        });
        div()
            .h(px(48.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(20.))
            .border_b_1()
            .border_color(p.line)
            .child(div().t_label().text_color(p.ink_muted).child("PROJECT"))
            .child(
                Picker::new(
                    "diagnostics-project-filter",
                    &self.project_picker,
                    project_filter_groups(&state.projects, self.filter),
                    project.unwrap_or_else(|| "All projects".into()),
                    p,
                )
                .width(280.),
            )
            .child(
                div()
                    .ml(px(8.))
                    .t_label()
                    .text_color(p.ink_muted)
                    .child("LANE"),
            )
            .child(
                Picker::new(
                    "diagnostics-lane-filter",
                    &self.lane_picker,
                    lane_filter_groups(&state.lanes, &state.projects, self.filter),
                    lane.unwrap_or_else(|| "All lanes".into()),
                    p,
                )
                .width(280.),
            )
    }
}

impl Render for DiagnosticsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.reanalyze(cx);
        if matches!(self.tab, DetailTab::Activity | DetailTab::Raw) {
            let indices = self.filtered_events(&self.analysis, &query);
            if self
                .selected_event
                .is_none_or(|selected| !indices.contains(&selected))
            {
                self.selected_event = indices.first().copied();
            }
        }
        let diagnostics = &self.analysis;
        let selected = self
            .selected_key
            .as_deref()
            .and_then(|key| diagnostics.get_in(key, self.filter));
        let selected_event = self.selected_event.and_then(|index| self.events.get(index));
        let this = cx.entity().downgrade();
        let tabs = ds::tabs(
            vec![
                Tab {
                    id: "activity",
                    label: "Activity".into(),
                    icon: Some("history"),
                    count: None,
                },
                Tab {
                    id: "tools",
                    label: "Tool calls".into(),
                    icon: Some("terminal"),
                    count: selected.map(|s| s.tools.len()),
                },
                Tab {
                    id: "changes",
                    label: "Changes".into(),
                    icon: Some("file-diff"),
                    count: selected.map(|s| s.changes.len()),
                },
                Tab {
                    id: "raw",
                    label: "Raw JSON".into(),
                    icon: Some("file"),
                    count: None,
                },
            ],
            match self.tab {
                DetailTab::Activity => "activity",
                DetailTab::Tools => "tools",
                DetailTab::Changes => "changes",
                DetailTab::Raw => "raw",
            },
            &p,
            {
                let this = this.clone();
                move |id, _, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.tab = match id {
                            "tools" => DetailTab::Tools,
                            "changes" => DetailTab::Changes,
                            "raw" => DetailTab::Raw,
                            _ => DetailTab::Activity,
                        };
                        cx.notify();
                    });
                }
            },
        );
        let body = match self.tab {
            DetailTab::Activity => self
                .activity(diagnostics, &query, &p, cx)
                .into_any_element(),
            DetailTab::Tools => self.tools(selected, &query, &p, cx).into_any_element(),
            DetailTab::Changes => self.changes(selected, &query, &p, cx).into_any_element(),
            DetailTab::Raw => self.raw_panel(selected_event, &p).into_any_element(),
        };
        let refresh_this = this.clone();
        let folder_store = self.store.clone();
        let close_ui = self.ui.clone();
        div()
            .size_full()
            .bg(p.bg)
            .text_color(p.ink)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(20.))
                    .border_b_1()
                    .border_color(p.line)
                    .child(icon("watch", 22.).text_color(p.primary))
                    .child(
                        div().child(div().t_heading().child("Diagnostics")).child(
                            div()
                                .t_small()
                                .text_color(p.ink_muted)
                                .child("Local event history · retained for 30 days"),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-diagnostics")
                            .ghost()
                            .small()
                            .icon("refresh-cw")
                            .label(if self.loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            })
                            .disabled(self.loading)
                            .on_click(move |_, _, cx| {
                                let _ = refresh_this.update(cx, |this, cx| this.refresh(cx));
                            }),
                    )
                    .child(
                        Button::new("open-log-folder")
                            .ghost()
                            .small()
                            .icon("folder-open")
                            .label("Open log folder")
                            .on_click(move |_, _, cx| {
                                folder_store.update(cx, |store, cx| store.show_logs(cx));
                            }),
                    )
                    .child(
                        Button::new("close-diagnostics")
                            .ghost()
                            .small()
                            .icon("x")
                            .tooltip("Close diagnostics")
                            .on_click(move |_, _, cx| {
                                close_ui.update(cx, |ui, cx| ui.close_diagnostics(cx));
                            }),
                    ),
            )
            .child(self.filters(&p, cx))
            .children((!self.errors.is_empty()).then(|| {
                div()
                    .flex_none()
                    .px(px(20.))
                    .py(px(8.))
                    .bg(p.error_soft)
                    .text_color(p.error)
                    .t_small()
                    .child(format!(
                        "{} log read problem(s): {}",
                        self.errors.len(),
                        self.errors.join(" · ")
                    ))
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .w(px(310.))
                            .flex_none()
                            .h_full()
                            .flex()
                            .flex_col()
                            .border_r_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .child(
                                div().p(px(12.)).child(
                                    div()
                                        .h(px(32.))
                                        .px(px(10.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(ds::RADIUS_SM))
                                        .border_1()
                                        .border_color(p.line_strong)
                                        .bg(p.bg)
                                        .child(Input::new(&self.search).appearance(false).t_body()),
                                ),
                            )
                            .child(self.session_tree(diagnostics, &query, &p, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .p(px(20.))
                            .child(self.detail_header(selected, cx))
                            .child(div().flex_none().child(tabs))
                            .child(body),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_core::service::Session;
    use panoptes_harness::HarnessRef;

    fn session(key: &str, parent: Option<&str>) -> Session {
        Session {
            key: key.into(),
            id: None,
            lane_id: None,
            harness: "fake".into(),
            harness_ref: Some(HarnessRef {
                harness: "fake".into(),
                id: key.into(),
                locator: format!("/{key}"),
                cwd: "/tmp".into(),
            }),
            cwd: "/tmp".into(),
            title: key.into(),
            tags: Vec::new(),
            created_at: 1,
            updated_at: 1,
            message_count: 0,
            parent_locator: parent.map(str::to_string),
            project_root: "/tmp".into(),
            checkout: None,
            project_id: None,
            pinned: false,
            archived: false,
            open: false,
        }
    }

    #[::core::prelude::v1::test]
    fn a_parent_cycle_does_not_hide_sessions() {
        let diagnostics = analyze(
            &[session("a", Some("/b")), session("b", Some("/a"))],
            &[],
            &[],
            &[],
        );
        let members: Vec<_> = diagnostics.sessions.iter().collect();
        let rows = session_rows(&members, "");
        let keys: HashSet<_> = rows
            .into_iter()
            .filter_map(|row| match row {
                TreeRow::Session { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(keys, HashSet::from(["a".into(), "b".into()]));
    }
}
