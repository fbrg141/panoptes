//! Rows for the command palettes: the search palette, the pane session picker,
//! and what a picked row stands for.

use panoptes_core::service::Session;
use panoptes_core::store::{Lane, LaneState, Project};

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

/// What picking a palette row does; parsed back from the row's id.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteTarget {
    Session(Box<Session>),
    Lane(Lane),
    Project(Project),
    NewSession,
    NewLaneSession,
    ResumeSession,
    Review,
    NewProject,
    Shortcuts,
    ShowLogs,
}

fn basename(path: &str) -> &str {
    path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path)
}

fn project_name(projects: &[Project], id: Option<i64>) -> Option<&str> {
    projects
        .iter()
        .find(|p| Some(p.id) == id)
        .map(|p| p.settings.name.as_str())
}

/// One line under a session's title: its lane, its project, or its folder.
fn session_hint(session: &Session, projects: &[Project], lanes: &[Lane]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if session.archived {
        parts.push("Archived".into());
    }
    if let Some(lane) = lanes.iter().find(|l| Some(l.id) == session.lane_id) {
        parts.push(
            project_name(projects, Some(lane.project_id))
                .unwrap_or("Project")
                .to_string(),
        );
        parts.push(lane.task.clone());
    } else {
        parts.push(
            project_name(projects, session.project_id)
                .unwrap_or_else(|| basename(&session.cwd))
                .to_string(),
        );
    }
    parts.join(" \u{b7} ")
}

/// Sessions in the order a switcher wants them: live ones before archived,
/// most recently updated first, and the one already on screen last, so
/// Enter on an empty query jumps to the session you were in before.
pub fn recent_sessions(sessions: &[Session], active_key: Option<&str>) -> Vec<Session> {
    let rank = |s: &Session| {
        if Some(s.key.as_str()) == active_key {
            2
        } else if s.archived {
            1
        } else {
            0
        }
    };
    let mut list = sessions.to_vec();
    list.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.updated_at.cmp(&a.updated_at)));
    list
}

fn item(id: String, label: String, hint: Option<String>, icon: &'static str) -> PaletteItem {
    PaletteItem {
        id,
        label,
        hint,
        icon: Some(icon),
        kbd: Vec::new(),
    }
}

fn action(id: &str, label: &str, icon: &'static str, kbd: &[&'static str]) -> PaletteItem {
    PaletteItem {
        id: format!("action:{id}"),
        label: label.into(),
        hint: None,
        icon: Some(icon),
        kbd: kbd.to_vec(),
    }
}

/// The search palette's rows. Sessions lead, because jumping between them is
/// what the palette is for; the actions follow. Ids carry the kind and the
/// key or id of the thing they stand for.
pub fn palette_groups(
    sessions: &[Session],
    projects: &[Project],
    lanes: &[Lane],
    active_key: Option<&str>,
) -> Vec<PaletteGroup> {
    let active_lane = sessions
        .iter()
        .find(|s| Some(s.key.as_str()) == active_key)
        .and_then(|s| s.lane_id);
    let has_active_lane = lanes
        .iter()
        .any(|l| Some(l.id) == active_lane && l.state == LaneState::Active);
    let has_review_lane = active_lane.is_some();
    let mut actions = Vec::new();
    if has_review_lane {
        actions.push(action(
            "review",
            "Review changes in current lane",
            "file-diff",
            &["Ctrl", "⇧", "R"],
        ));
    }
    if has_active_lane {
        actions.push(action(
            "newLaneSession",
            "New session in current lane (/new)",
            "plus",
            &["⌘", "⇧", "N"],
        ));
        actions.push(action(
            "resumeSession",
            "Resume earlier session (/resume)",
            "history",
            &[],
        ));
    }
    actions.push(action("newSession", "New lane", "plus", &["⌘", "T"]));
    actions.push(action("newProject", "Add a project", "folder", &["⌘", "N"]));
    actions.push(action(
        "shortcuts",
        "Keyboard shortcuts",
        "command",
        &["⌘", "/"],
    ));
    actions.push(action("showLogs", "Open diagnostics pane", "watch", &[]));
    vec![
        PaletteGroup {
            label: "Sessions".into(),
            items: recent_sessions(sessions, active_key)
                .into_iter()
                .map(|s| {
                    let hint = session_hint(&s, projects, lanes);
                    let hint = if Some(s.key.as_str()) == active_key {
                        format!("Current \u{b7} {hint}")
                    } else {
                        hint
                    };
                    item(format!("session:{}", s.key), s.title, Some(hint), "session")
                })
                .collect(),
        },
        PaletteGroup {
            label: "Lanes".into(),
            items: lanes
                .iter()
                .filter(|l| l.state == LaneState::Active)
                .map(|lane| {
                    // A lane with no session yet starts one when picked.
                    let where_ = format!(
                        "{} \u{b7} {}",
                        project_name(projects, Some(lane.project_id)).unwrap_or("Project"),
                        lane.branch
                    );
                    let started = sessions.iter().any(|s| s.lane_id == Some(lane.id));
                    let hint = if started {
                        where_
                    } else {
                        format!("Start a session \u{b7} {where_}")
                    };
                    item(
                        format!("lane:{}", lane.id),
                        lane.task.clone(),
                        Some(hint),
                        "git-branch",
                    )
                })
                .collect(),
        },
        PaletteGroup {
            label: "Projects".into(),
            items: projects
                .iter()
                .map(|p| {
                    item(
                        format!("project:{}", p.id),
                        p.settings.name.clone(),
                        Some(format!("New lane \u{b7} {}", p.repo)),
                        "folder",
                    )
                })
                .collect(),
        },
        PaletteGroup {
            label: "Actions".into(),
            items: actions,
        },
    ]
}

/// The thing a palette row's id stands for, or `None` if it is gone.
pub fn palette_target(
    id: &str,
    sessions: &[Session],
    projects: &[Project],
    lanes: &[Lane],
) -> Option<PaletteTarget> {
    // Keys contain colons, so the id is split on the first one only.
    let (kind, rest) = id.split_once(':')?;
    match kind {
        "session" => sessions
            .iter()
            .find(|s| s.key == rest)
            .map(|s| PaletteTarget::Session(Box::new(s.clone()))),
        "lane" => {
            let id: i64 = rest.parse().ok()?;
            lanes
                .iter()
                .find(|l| l.id == id)
                .map(|l| PaletteTarget::Lane(l.clone()))
        }
        "project" => {
            let id: i64 = rest.parse().ok()?;
            projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| PaletteTarget::Project(p.clone()))
        }
        "action" => match rest {
            "newSession" => Some(PaletteTarget::NewSession),
            "newLaneSession" => Some(PaletteTarget::NewLaneSession),
            "resumeSession" => Some(PaletteTarget::ResumeSession),
            "review" => Some(PaletteTarget::Review),
            "newProject" => Some(PaletteTarget::NewProject),
            "shortcuts" => Some(PaletteTarget::Shortcuts),
            "showLogs" => Some(PaletteTarget::ShowLogs),
            _ => None,
        },
        _ => None,
    }
}

/// The pane session picker: new work first, recent sessions after it.
pub fn session_picker_groups(sessions: &[Session], projects: &[Project]) -> Vec<PaletteGroup> {
    let hint = projects
        .first()
        .map_or("Add a project first".to_string(), |p| {
            format!("Start a lane in {}", p.settings.name)
        });
    vec![
        PaletteGroup {
            label: "Actions".into(),
            items: vec![item("new".into(), "New session".into(), Some(hint), "plus")],
        },
        PaletteGroup {
            label: "Sessions".into(),
            items: recent_sessions(sessions, None)
                .into_iter()
                .map(|s| {
                    item(
                        format!("session:{}", s.key),
                        s.title,
                        Some(s.cwd),
                        "session",
                    )
                })
                .collect(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use panoptes_core::store::ProjectSettings;

    use super::*;
    use crate::model::sidebar::tests::session as base;

    fn project() -> Project {
        Project {
            id: 1,
            repo: "/code/web".into(),
            settings: ProjectSettings {
                name: "Web app".into(),
                base_branch: "main".into(),
                copy_files: Vec::new(),
                setup: None,
            },
        }
    }

    fn lane() -> Lane {
        Lane {
            id: 4,
            project_id: 1,
            name: "fix-login".into(),
            task: "Fix login".into(),
            branch: "panoptes/fix-login".into(),
            state: LaneState::Active,
            worktrees: Vec::new(),
        }
    }

    fn session() -> Session {
        Session {
            title: "Release checklist".into(),
            cwd: "/code/web".into(),
            project_id: Some(1),
            ..base("pi:s1")
        }
    }

    fn ids(group: &PaletteGroup) -> Vec<&str> {
        group.items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn lists_sessions_active_lanes_projects_and_actions_with_traceable_ids() {
        let archived = Lane {
            id: 5,
            state: LaneState::Archived,
            ..lane()
        };
        let groups = palette_groups(&[session()], &[project()], &[lane(), archived], None);
        let labels: Vec<&str> = groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Sessions", "Lanes", "Projects", "Actions"]);
        assert_eq!(
            ids(&groups[3]),
            [
                "action:newSession",
                "action:newProject",
                "action:shortcuts",
                "action:showLogs"
            ]
        );
        assert_eq!(groups[0].items[0].id, "session:pi:s1");
        assert_eq!(groups[0].items[0].label, "Release checklist");
        assert_eq!(groups[1].items.len(), 1);
        assert_eq!(groups[1].items[0].id, "lane:4");
        assert_eq!(
            groups[1].items[0].hint.as_deref(),
            Some("Start a session \u{b7} Web app \u{b7} panoptes/fix-login")
        );
        assert_eq!(groups[2].items[0].id, "project:1");
        assert_eq!(
            groups[2].items[0].hint.as_deref(),
            Some("New lane \u{b7} /code/web")
        );
    }

    #[test]
    fn hints_lane_sessions_and_only_warns_about_starting_when_a_lane_has_none() {
        let owned = Session {
            lane_id: Some(4),
            ..session()
        };
        let groups = palette_groups(std::slice::from_ref(&owned), &[project()], &[lane()], None);
        assert_eq!(
            groups[1].items[0].hint.as_deref(),
            Some("Web app \u{b7} panoptes/fix-login")
        );
        assert_eq!(
            groups[0].items[0].hint.as_deref(),
            Some("Web app \u{b7} Fix login")
        );
        let archived = Session {
            archived: true,
            ..owned
        };
        let groups = palette_groups(&[archived], &[project()], &[lane()], None);
        assert_eq!(
            groups[0].items[0].hint.as_deref(),
            Some("Archived \u{b7} Web app \u{b7} Fix login")
        );
    }

    #[test]
    fn offers_new_and_resume_only_for_the_active_lane_on_screen() {
        let owned = Session {
            lane_id: Some(4),
            ..session()
        };
        let groups = palette_groups(&[owned], &[project()], &[lane()], Some("pi:s1"));
        assert_eq!(
            ids(&groups[3]),
            [
                "action:review",
                "action:newLaneSession",
                "action:resumeSession",
                "action:newSession",
                "action:newProject",
                "action:shortcuts",
                "action:showLogs"
            ]
        );
        assert!(
            groups[0].items[0]
                .hint
                .as_deref()
                .unwrap()
                .starts_with("Current \u{b7} ")
        );
    }

    #[test]
    fn recent_sessions_put_live_first_archived_after_and_the_current_last() {
        let at = |key: &str, updated_at: i64, archived: bool| Session {
            updated_at,
            archived,
            ..base(key)
        };
        let order: Vec<String> = recent_sessions(
            &[
                at("old", 1, false),
                at("current", 9, false),
                at("new", 5, false),
                at("archived", 8, true),
            ],
            Some("current"),
        )
        .into_iter()
        .map(|s| s.key)
        .collect();
        assert_eq!(order, ["new", "old", "archived", "current"]);
    }

    #[test]
    fn targets_trace_ids_back() {
        let (s, p, l) = ([session()], [project()], [lane()]);
        assert_eq!(
            palette_target("session:pi:s1", &s, &p, &l),
            Some(PaletteTarget::Session(Box::new(session())))
        );
        assert_eq!(
            palette_target("lane:4", &s, &p, &l),
            Some(PaletteTarget::Lane(lane()))
        );
        assert_eq!(
            palette_target("project:1", &s, &p, &l),
            Some(PaletteTarget::Project(project()))
        );
        assert_eq!(
            palette_target("action:review", &s, &p, &l),
            Some(PaletteTarget::Review)
        );
        assert_eq!(
            palette_target("action:showLogs", &s, &p, &l),
            Some(PaletteTarget::ShowLogs)
        );
        assert_eq!(palette_target("lane:9", &s, &p, &l), None);
        assert_eq!(palette_target("action:nope", &s, &p, &l), None);
    }

    #[test]
    fn the_session_picker_offers_new_work_first() {
        let groups = session_picker_groups(&[session()], &[project()]);
        assert_eq!(ids(&groups[0]), ["new"]);
        assert_eq!(
            groups[0].items[0].hint.as_deref(),
            Some("Start a lane in Web app")
        );
        assert_eq!(
            session_picker_groups(&[], &[])[0].items[0].hint.as_deref(),
            Some("Add a project first")
        );
    }

    #[test]
    fn filtering_matches_label_or_hint_and_drops_empty_groups() {
        let groups = palette_groups(&[session()], &[project()], &[lane()], None);
        let found = filter_palette(&groups, " RELEASE ");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].items[0].id, "session:pi:s1");
        assert_eq!(filter_palette(&groups, "").len(), 4);
    }
}
