//! Pure aggregation for the diagnostics pane. The JSONL log is the source
//! of history; the service listing supplies durable titles, lanes and forks.

use std::collections::{BTreeMap, HashMap};

use panoptes_core::service::Session;
use panoptes_core::store::{Lane, Project};
use panoptes_harness::SessionStatus;
use serde_json::Value;

use super::format::format_tokens;
use super::picker::{PickerGroup, PickerItem};

/// Restricts both the session tree and its events to one project or lane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticsFilter {
    pub project_id: Option<i64>,
    pub lane_id: Option<i64>,
}

impl DiagnosticsFilter {
    pub fn project(project_id: i64) -> Self {
        Self {
            project_id: Some(project_id),
            lane_id: None,
        }
    }

    pub fn lane(project_id: i64, lane_id: i64) -> Self {
        Self {
            project_id: Some(project_id),
            lane_id: Some(lane_id),
        }
    }

    pub fn is_all(self) -> bool {
        self.project_id.is_none() && self.lane_id.is_none()
    }

    pub fn matches(self, session: &SessionDiagnostics) -> bool {
        self.project_id
            .is_none_or(|id| session.project_id == Some(id))
            && self.lane_id.is_none_or(|id| session.lane_id == Some(id))
    }
}

/// What an open diagnostics pane does when a new scope is asked for. The
/// pane's own pickers change the scope too, so a request can arrive that the
/// pane already shows, and one can arrive for an open pane showing another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeChange {
    /// Nothing to do: no pane and nothing asked for.
    Nothing,
    /// Open a pane on the requested scope.
    Open(DiagnosticsFilter),
    /// Move the open pane to the requested scope.
    Rescope(DiagnosticsFilter),
    /// Close the open pane.
    Close,
}

pub fn scope_change(
    open: Option<DiagnosticsFilter>,
    wanted: Option<DiagnosticsFilter>,
) -> ScopeChange {
    match (open, wanted) {
        (None, None) => ScopeChange::Nothing,
        (None, Some(filter)) => ScopeChange::Open(filter),
        (Some(_), None) => ScopeChange::Close,
        (Some(open), Some(wanted)) if open == wanted => ScopeChange::Nothing,
        (Some(_), Some(wanted)) => ScopeChange::Rescope(wanted),
    }
}

pub fn project_filter_groups(projects: &[Project], filter: DiagnosticsFilter) -> Vec<PickerGroup> {
    let mut items = vec![PickerItem {
        id: String::new(),
        label: "All projects".into(),
        current: filter.project_id.is_none(),
        ..Default::default()
    }];
    items.extend(projects.iter().map(|project| PickerItem {
        id: project.id.to_string(),
        label: project.settings.name.clone(),
        detail: Some(project.repo.clone()),
        current: filter.project_id == Some(project.id),
        ..Default::default()
    }));
    vec![PickerGroup { label: None, items }]
}

pub fn lane_filter_groups(
    lanes: &[Lane],
    projects: &[Project],
    filter: DiagnosticsFilter,
) -> Vec<PickerGroup> {
    let mut items = vec![PickerItem {
        id: String::new(),
        label: "All lanes".into(),
        current: filter.lane_id.is_none(),
        ..Default::default()
    }];
    items.extend(
        lanes
            .iter()
            .filter(|lane| filter.project_id.is_none_or(|id| lane.project_id == id))
            .map(|lane| {
                let project = projects
                    .iter()
                    .find(|project| project.id == lane.project_id)
                    .map(|project| project.settings.name.as_str())
                    .unwrap_or("Unknown project");
                PickerItem {
                    id: lane.id.to_string(),
                    label: lane.task.clone(),
                    detail: Some(lane.branch.clone()),
                    keywords: vec![project.to_string()],
                    current: filter.lane_id == Some(lane.id),
                    ..Default::default()
                }
            }),
    );
    vec![PickerGroup { label: None, items }]
}

#[derive(Debug, Clone)]
pub struct ToolSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub updates: u32,
    pub edited_paths: Vec<String>,
}

impl ToolSummary {
    pub fn duration_ms(&self) -> Option<i64> {
        Some(self.ended_at? - self.started_at?)
    }
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionDiagnostics {
    pub key: String,
    pub title: String,
    pub project_id: Option<i64>,
    pub project: Option<String>,
    pub lane_id: Option<i64>,
    pub lane: Option<String>,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub parent_locator: Option<String>,
    pub parent_key: Option<String>,
    pub status: SessionStatus,
    pub last_activity: i64,
    pub runs: u32,
    pub model_calls: u32,
    pub failed_model_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: Option<f64>,
    pub models: Vec<String>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub tools: BTreeMap<String, ToolSummary>,
    pub changes: BTreeMap<String, FileChange>,
    pub event_indices: Vec<usize>,
}

impl SessionDiagnostics {
    fn new(session: &Session, projects: &[Project], lanes: &[Lane]) -> Self {
        let lane = session
            .lane_id
            .and_then(|id| lanes.iter().find(|lane| lane.id == id));
        let project_id = session.project_id.or(lane.map(|lane| lane.project_id));
        let project = project_id.and_then(|id| projects.iter().find(|project| project.id == id));
        Self {
            key: session.key.clone(),
            title: session.title.clone(),
            project_id,
            project: project.map(|project| project.settings.name.clone()),
            lane_id: session.lane_id,
            lane: lane.map(|lane| lane.task.clone()),
            branch: lane.map(|lane| lane.branch.clone()),
            cwd: Some(session.cwd.clone()),
            parent_locator: session.parent_locator.clone(),
            parent_key: None,
            status: SessionStatus::Idle,
            last_activity: session.updated_at,
            runs: 0,
            model_calls: 0,
            failed_model_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: None,
            models: Vec::new(),
            context_tokens: None,
            context_window: None,
            tools: BTreeMap::new(),
            changes: BTreeMap::new(),
            event_indices: Vec::new(),
        }
    }

    fn unknown(key: &str) -> Self {
        Self {
            key: key.to_string(),
            title: key.to_string(),
            project_id: None,
            project: None,
            lane_id: None,
            lane: None,
            branch: None,
            cwd: None,
            parent_locator: None,
            parent_key: None,
            status: SessionStatus::Idle,
            last_activity: 0,
            runs: 0,
            model_calls: 0,
            failed_model_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost: None,
            models: Vec::new(),
            context_tokens: None,
            context_window: None,
            tools: BTreeMap::new(),
            changes: BTreeMap::new(),
            event_indices: Vec::new(),
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn successful_tools(&self) -> usize {
        self.tools
            .values()
            .filter(|tool| tool.status == "done")
            .count()
    }

    pub fn failed_tools(&self) -> usize {
        self.tools
            .values()
            .filter(|tool| tool.status == "failed")
            .count()
    }

    pub fn running_tools(&self) -> usize {
        self.tools
            .values()
            .filter(|tool| matches!(tool.status.as_str(), "pending" | "running"))
            .count()
    }
}

#[derive(Debug, Default)]
pub struct Diagnostics {
    pub sessions: Vec<SessionDiagnostics>,
}

impl Diagnostics {
    pub fn get(&self, key: &str) -> Option<&SessionDiagnostics> {
        self.sessions.iter().find(|session| session.key == key)
    }

    pub fn get_in(&self, key: &str, filter: DiagnosticsFilter) -> Option<&SessionDiagnostics> {
        self.get(key).filter(|session| filter.matches(session))
    }
}

/// Event rows for the selected session, or for every session in the scope.
/// Unscoped diagnostics also retain events that have no session key.
pub fn diagnostic_event_indices(
    diagnostics: &Diagnostics,
    event_count: usize,
    filter: DiagnosticsFilter,
    selected_key: Option<&str>,
) -> Vec<usize> {
    if let Some(key) = selected_key {
        return diagnostics
            .get_in(key, filter)
            .map(|session| session.event_indices.clone())
            .unwrap_or_default();
    }
    if filter.is_all() {
        return (0..event_count).collect();
    }
    let mut indices: Vec<usize> = diagnostics
        .sessions
        .iter()
        .filter(|session| filter.matches(session))
        .flat_map(|session| session.event_indices.iter().copied())
        .collect();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn text<'a>(event: &'a Value, field: &str) -> Option<&'a str> {
    event.get(field).and_then(Value::as_str)
}

fn integer(event: &Value, field: &str) -> Option<u64> {
    event.get(field).and_then(Value::as_u64)
}

/// Folds all retained events. A tool's streaming updates replace one another;
/// a completed assistant message contributes one model call.
pub fn analyze(
    sessions: &[Session],
    projects: &[Project],
    lanes: &[Lane],
    events: &[Value],
) -> Diagnostics {
    let mut by_key: BTreeMap<String, SessionDiagnostics> = sessions
        .iter()
        .map(|session| {
            (
                session.key.clone(),
                SessionDiagnostics::new(session, projects, lanes),
            )
        })
        .collect();

    for (index, event) in events.iter().enumerate() {
        let Some(key) = text(event, "key") else {
            continue;
        };
        let at = event.get("at").and_then(Value::as_i64).unwrap_or(0);
        let diagnostic = by_key
            .entry(key.to_string())
            .or_insert_with(|| SessionDiagnostics::unknown(key));
        diagnostic.last_activity = diagnostic.last_activity.max(at);
        diagnostic.event_indices.push(index);
        match text(event, "event").unwrap_or_default() {
            "runStarted" => diagnostic.runs += 1,
            "status" => {
                diagnostic.status = match text(event, "status") {
                    Some("running") => SessionStatus::Running,
                    Some("waiting") => SessionStatus::Waiting,
                    Some("failed") => SessionStatus::Failed,
                    Some("done") => SessionStatus::Done,
                    _ => SessionStatus::Idle,
                };
            }
            "toolCall" => {
                let Some(id) = text(event, "id") else {
                    continue;
                };
                let paths: Vec<String> = event
                    .get("editedPaths")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                let tool = diagnostic
                    .tools
                    .entry(id.to_string())
                    .or_insert_with(|| ToolSummary {
                        id: id.to_string(),
                        name: "Tool".into(),
                        kind: "other".into(),
                        status: "pending".into(),
                        started_at: None,
                        ended_at: None,
                        updates: 0,
                        edited_paths: Vec::new(),
                    });
                tool.updates += 1;
                tool.name = text(event, "name").unwrap_or("Tool").to_string();
                tool.kind = text(event, "kind").unwrap_or("other").to_string();
                tool.status = text(event, "status").unwrap_or("pending").to_string();
                tool.started_at = event
                    .get("startedAt")
                    .and_then(Value::as_i64)
                    .or(tool.started_at);
                tool.ended_at = event
                    .get("endedAt")
                    .and_then(Value::as_i64)
                    .or(tool.ended_at);
                tool.edited_paths = paths.clone();
                for path in paths {
                    diagnostic.changes.insert(
                        path.clone(),
                        FileChange {
                            path,
                            tool_call_id: id.to_string(),
                            tool_name: tool.name.clone(),
                            at,
                        },
                    );
                }
            }
            "modelCall" => {
                diagnostic.model_calls += 1;
                if event.get("failed").and_then(Value::as_bool) == Some(true) {
                    diagnostic.failed_model_calls += 1;
                }
                if let Some(usage) = event.get("usage").filter(|usage| !usage.is_null()) {
                    diagnostic.input_tokens += integer(usage, "inputTokens").unwrap_or(0);
                    diagnostic.output_tokens += integer(usage, "outputTokens").unwrap_or(0);
                    diagnostic.cache_read_tokens += integer(usage, "cacheReadTokens").unwrap_or(0);
                    diagnostic.cache_write_tokens +=
                        integer(usage, "cacheWriteTokens").unwrap_or(0);
                    if let Some(cost) = usage.get("cost").and_then(Value::as_f64) {
                        diagnostic.cost = Some(diagnostic.cost.unwrap_or(0.0) + cost);
                    }
                    if let Some(model) = text(usage, "model")
                        && !diagnostic.models.iter().any(|known| known == model)
                    {
                        diagnostic.models.push(model.to_string());
                    }
                }
            }
            "context" => {
                diagnostic.context_tokens = event.get("tokens").and_then(Value::as_u64);
                diagnostic.context_window = integer(event, "window").or(diagnostic.context_window);
            }
            _ => {}
        }
    }

    let locators: HashMap<String, String> = sessions
        .iter()
        .filter_map(|session| {
            Some((
                session.harness_ref.as_ref()?.locator.clone(),
                session.key.clone(),
            ))
        })
        .collect();
    for session in by_key.values_mut() {
        session.parent_key = session
            .parent_locator
            .as_ref()
            .and_then(|locator| locators.get(locator))
            .cloned();
    }
    let mut sessions: Vec<_> = by_key.into_values().collect();
    sessions.sort_by(|a, b| {
        b.last_activity
            .cmp(&a.last_activity)
            .then_with(|| a.title.cmp(&b.title))
    });
    Diagnostics { sessions }
}

/// One short, useful label for an event row; the full object stays available.
pub fn event_summary(event: &Value) -> String {
    match text(event, "event").unwrap_or("event") {
        "toolCall" => {
            let files = event
                .get("editedPaths")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let mut summary = format!(
                "{} · {}",
                text(event, "name").unwrap_or("Tool"),
                text(event, "status").unwrap_or("unknown")
            );
            if files > 0 {
                summary.push_str(&format!(
                    " · {files} {}",
                    if files == 1 { "file" } else { "files" }
                ));
            }
            summary
        }
        "modelCall" => event
            .get("usage")
            .filter(|usage| !usage.is_null())
            .map(|usage| {
                let input = integer(usage, "inputTokens").unwrap_or(0)
                    + integer(usage, "cacheReadTokens").unwrap_or(0)
                    + integer(usage, "cacheWriteTokens").unwrap_or(0);
                format!(
                    "{} in · {} out",
                    format_tokens(input),
                    format_tokens(integer(usage, "outputTokens").unwrap_or(0))
                )
            })
            .unwrap_or_else(|| "No usage reported".into()),
        "context" => match (
            event.get("tokens").and_then(Value::as_u64),
            integer(event, "window"),
        ) {
            (Some(tokens), Some(window)) => format!(
                "{} / {} tokens",
                format_tokens(tokens),
                format_tokens(window)
            ),
            _ => "Context size unknown".into(),
        },
        "sessionStarted" => {
            if event.get("resumed").and_then(Value::as_bool) == Some(true) {
                "Resumed harness conversation".into()
            } else {
                "Started a new harness conversation".into()
            }
        }
        "runSettled" => {
            if event.get("aborted").and_then(Value::as_bool) == Some(true) {
                "Run aborted".into()
            } else {
                "Run finished".into()
            }
        }
        "error" => text(event, "message")
            .unwrap_or("Command failed")
            .to_string(),
        "status" => format!("Status: {}", text(event, "status").unwrap_or("unknown")),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_core::store::{LaneState, ProjectSettings};
    use panoptes_harness::HarnessRef;
    use serde_json::json;

    fn project(id: i64, name: &str) -> Project {
        Project {
            id,
            repo: format!("/{name}"),
            settings: ProjectSettings {
                name: name.into(),
                base_branch: "main".into(),
                copy_files: Vec::new(),
                setup: None,
            },
        }
    }

    fn lane(id: i64, project_id: i64, task: &str) -> Lane {
        Lane {
            id,
            project_id,
            name: task.into(),
            task: task.into(),
            branch: format!("lane/{task}"),
            state: LaneState::Active,
            worktrees: Vec::new(),
        }
    }

    fn session(key: &str, locator: &str, parent: Option<&str>) -> Session {
        Session {
            key: key.into(),
            id: None,
            lane_id: None,
            harness: "fake".into(),
            harness_ref: Some(HarnessRef {
                harness: "fake".into(),
                id: key.into(),
                locator: locator.into(),
                cwd: "/tmp".into(),
            }),
            cwd: "/tmp".into(),
            title: format!("Session {key}"),
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

    #[test]
    fn folds_tool_updates_usage_context_and_forks() {
        let events = vec![
            json!({"at": 1, "event": "runStarted", "key": "child"}),
            json!({"at": 2, "event": "toolCall", "key": "child", "id": "t1", "name": "edit", "kind": "edit", "status": "running"}),
            json!({"at": 3, "event": "toolCall", "key": "child", "id": "t1", "name": "edit", "kind": "edit", "status": "done", "startedAt": 2, "endedAt": 8, "editedPaths": ["src/lib.rs"]}),
            json!({"at": 4, "event": "toolCall", "key": "child", "id": "t2", "name": "bash", "kind": "execute", "status": "failed"}),
            json!({"at": 5, "event": "modelCall", "key": "child", "messageId": "m1", "usage": {"model": "model-a", "inputTokens": 10, "outputTokens": 3, "cacheReadTokens": 20, "cacheWriteTokens": 2, "cost": 0.01}}),
            json!({"at": 6, "event": "context", "key": "child", "tokens": 35, "window": 100}),
        ];
        let diagnostics = analyze(
            &[
                session("parent", "/parent", None),
                session("child", "/child", Some("/parent")),
            ],
            &[],
            &[],
            &events,
        );
        let child = diagnostics.get("child").unwrap();
        assert_eq!(child.parent_key.as_deref(), Some("parent"));
        assert_eq!(child.runs, 1);
        assert_eq!(child.tools.len(), 2);
        assert_eq!(child.tools["t1"].updates, 2);
        assert_eq!(child.tools["t1"].duration_ms(), Some(6));
        assert_eq!(child.successful_tools(), 1);
        assert_eq!(child.failed_tools(), 1);
        assert_eq!(child.total_tokens(), 35);
        assert_eq!(
            (child.context_tokens, child.context_window),
            (Some(35), Some(100))
        );
        assert_eq!(child.changes["src/lib.rs"].tool_call_id, "t1");
        assert_eq!(child.models, ["model-a"]);
    }

    #[test]
    fn keeps_logged_sessions_that_are_not_in_the_current_listing() {
        let diagnostics = analyze(
            &[],
            &[],
            &[],
            &[json!({"at": 9, "event": "status", "key": "fake:old", "status": "failed"})],
        );
        assert_eq!(diagnostics.sessions.len(), 1);
        assert_eq!(diagnostics.sessions[0].status, SessionStatus::Failed);
        assert_eq!(diagnostics.sessions[0].last_activity, 9);
    }

    #[test]
    fn project_and_lane_filters_scope_sessions_and_their_events() {
        let mut first = session("first", "/first", None);
        first.project_id = Some(1);
        first.lane_id = Some(10);
        let mut second = session("second", "/second", None);
        second.project_id = Some(1);
        second.lane_id = Some(11);
        let mut other = session("other", "/other", None);
        other.project_id = Some(2);
        let events = vec![
            json!({"at": 1, "event": "error", "message": "global"}),
            json!({"at": 2, "event": "status", "key": "first", "status": "done"}),
            json!({"at": 3, "event": "status", "key": "second", "status": "done"}),
            json!({"at": 4, "event": "status", "key": "other", "status": "done"}),
        ];
        let diagnostics = analyze(&[first, second, other], &[], &[], &events);
        let project = DiagnosticsFilter::project(1);
        assert_eq!(
            diagnostic_event_indices(&diagnostics, events.len(), project, None),
            [1, 2]
        );
        let lane = DiagnosticsFilter::lane(1, 11);
        assert_eq!(
            diagnostic_event_indices(&diagnostics, events.len(), lane, None),
            [2]
        );
        assert_eq!(
            diagnostic_event_indices(&diagnostics, events.len(), lane, Some("first")),
            [] as [usize; 0]
        );
        assert_eq!(
            diagnostic_event_indices(&diagnostics, events.len(), Default::default(), None),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn filter_pickers_list_projects_and_only_the_selected_projects_lanes() {
        let projects = [project(1, "One"), project(2, "Two")];
        let lanes = [lane(10, 1, "First"), lane(20, 2, "Second")];
        let filter = DiagnosticsFilter::project(1);
        let project_items = &project_filter_groups(&projects, filter)[0].items;
        assert_eq!(project_items[0].label, "All projects");
        assert!(project_items[1].current);
        let lane_items = &lane_filter_groups(&lanes, &projects, filter)[0].items;
        assert_eq!(
            lane_items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["", "10"]
        );
        assert!(lane_items[0].current);
    }

    #[test]
    fn asking_for_a_scope_moves_an_open_pane_to_it() {
        let first = DiagnosticsFilter::project(1);
        let second = DiagnosticsFilter::project(2);
        assert_eq!(scope_change(None, None), ScopeChange::Nothing);
        assert_eq!(scope_change(None, Some(first)), ScopeChange::Open(first));
        assert_eq!(scope_change(Some(first), None), ScopeChange::Close);
        // The pane's own pickers may have moved it: asking for what it shows
        // changes nothing.
        assert_eq!(
            scope_change(Some(second), Some(second)),
            ScopeChange::Nothing
        );
        // ...and asking for another scope must still move it, which is what
        // used to be lost because only the opened scope was remembered.
        assert_eq!(
            scope_change(Some(second), Some(first)),
            ScopeChange::Rescope(first)
        );
    }

    #[test]
    fn event_labels_explain_usage_and_changes() {
        assert_eq!(
            event_summary(
                &json!({"event": "toolCall", "name": "edit", "status": "done", "editedPaths": ["a", "b"]})
            ),
            "edit · done · 2 files"
        );
        assert_eq!(
            event_summary(
                &json!({"event": "modelCall", "usage": {"inputTokens": 1000, "outputTokens": 20}})
            ),
            "1k in · 20 out"
        );
    }
}
