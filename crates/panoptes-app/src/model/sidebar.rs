//! Pure sidebar logic: status words, row details, a lane's main session, empty
//! lanes and the row menus.

use panoptes_core::lanes::LaneWork;
use panoptes_core::service::{LaneNavigation, Session};
use panoptes_core::store::{Lane, LaneState};
use panoptes_harness::SessionStatus;

use super::format::plural;

/// Whether a row carries weight: an agent at work or one that needs you.
pub fn is_live(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Running | SessionStatus::Waiting | SessionStatus::Failed
    )
}

/// The second line of a session row. Nested under a lane, the lane already
/// names the project and branch, so the row says only what is its own.
pub fn session_detail(session: &Session, context: Option<&str>) -> String {
    let own = match context {
        Some(context) => context.to_string(),
        None => plural(session.message_count as usize, "message", "messages"),
    };
    if session.archived {
        format!("Archived · {own}")
    } else {
        own
    }
}

/// The one action worth a button on the row itself: Stop, while it runs.
pub fn primary_action(session: &Session, status: SessionStatus) -> bool {
    session.open && status == SessionStatus::Running
}

/// The open root session, or the root most recently active when none is open.
pub fn lane_main_session(sessions: &[Session]) -> Option<&Session> {
    let roots: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.parent_locator.is_none())
        .collect();
    let candidates: Vec<&Session> = if roots.is_empty() {
        sessions.iter().collect()
    } else {
        roots
    };
    let mut best: Option<&Session> = None;
    for session in candidates {
        best = match best {
            _ if session.open => Some(session),
            Some(b) if b.open => Some(b),
            Some(b)
                if !(session.updated_at > b.updated_at
                    || (session.updated_at == b.updated_at
                        && session.id.unwrap_or(0) > b.id.unwrap_or(0))) =>
            {
                Some(b)
            }
            _ => Some(session),
        };
    }
    best
}

/// The lane's live session that opening `session` would stop.
pub fn lane_session_to_stop<'a>(session: &Session, sessions: &'a [Session]) -> Option<&'a Session> {
    if session.id.is_none() || session.open {
        return None;
    }
    let lane = session.lane_id?;
    sessions
        .iter()
        .find(|s| s.lane_id == Some(lane) && s.open && s.key != session.key)
}

/// Earlier root conversations available to /resume, newest created first.
pub fn lane_resume_sessions(sessions: &[Session]) -> Vec<Session> {
    let current = lane_main_session(sessions).map(|s| s.key.clone());
    let mut list: Vec<Session> = sessions
        .iter()
        .filter(|s| s.parent_locator.is_none() && !s.archived && Some(&s.key) != current.as_ref())
        .cloned()
        .collect();
    list.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(b.id.unwrap_or(0).cmp(&a.id.unwrap_or(0)))
    });
    list
}

/// What a lane row offers when the lane has no session of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyLane {
    Startable,
    /// Another session that edited the lane is still Running or Waiting.
    Busy {
        writer: String,
    },
}

pub fn empty_lane(lane: &Lane, navigation: Option<&LaneNavigation>) -> Option<EmptyLane> {
    if lane.state != LaneState::Active {
        return None;
    }
    if let Some(nav) = navigation
        && (!nav.sessions.is_empty() || !nav.archived_sessions.is_empty())
    {
        return None;
    }
    Some(match navigation.and_then(|n| n.busy_writer.as_ref()) {
        Some(writer) => EmptyLane::Busy {
            writer: writer.title.clone(),
        },
        None => EmptyLane::Startable,
    })
}

/// "6 files changed · 2 commits ahead", "No changes yet", or `None` if unknown.
pub fn work_label(work: Option<&LaneWork>) -> Option<String> {
    let work = work?;
    let mut parts = Vec::new();
    if work.changed_files > 0 {
        parts.push(format!(
            "{} changed",
            plural(work.changed_files as usize, "file", "files")
        ));
    }
    if let Some(ahead) = work.commits_ahead
        && ahead > 0
    {
        parts.push(format!(
            "{} ahead",
            plural(ahead as usize, "commit", "commits")
        ));
    }
    Some(if parts.is_empty() {
        "No changes yet".to_string()
    } else {
        parts.join(" · ")
    })
}

/// One entry in a row's menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        id: &'static str,
        label: String,
        icon: Option<&'static str>,
        danger: bool,
        disabled: bool,
    },
    Separator,
}

impl MenuEntry {
    pub fn item(id: &'static str, label: impl Into<String>, icon: Option<&'static str>) -> Self {
        MenuEntry::Item {
            id,
            label: label.into(),
            icon,
            danger: false,
            disabled: false,
        }
    }
}

/// A project can only be removed once it has no lanes, archived ones included.
pub fn project_menu_items(lane_count: usize) -> Vec<MenuEntry> {
    vec![
        MenuEntry::item("new-lane", "New lane", Some("git-branch")),
        MenuEntry::item("cleanup-lanes", "Clean up lanes…", Some("folder")),
        MenuEntry::Separator,
        MenuEntry::Item {
            id: "remove-project",
            label: if lane_count > 0 {
                "Remove project (delete its lanes first)".into()
            } else {
                "Remove project".into()
            },
            icon: Some("trash-2"),
            danger: true,
            disabled: lane_count > 0,
        },
    ]
}

pub fn lane_menu_items(lane: &Lane) -> Vec<MenuEntry> {
    let mut items = Vec::new();
    if lane.state == LaneState::Active {
        items.push(MenuEntry::item(
            "new-session",
            "New session (/new)",
            Some("plus"),
        ));
        items.push(MenuEntry::item(
            "resume-session",
            "Resume session (/resume)…",
            Some("history"),
        ));
        items.push(MenuEntry::Separator);
        items.push(MenuEntry::item("archive-lane", "Archive lane", Some("x")));
    } else {
        items.push(MenuEntry::item(
            "restore-lane",
            "Restore lane",
            Some("rotate-ccw"),
        ));
    }
    items.push(MenuEntry::Separator);
    items.push(MenuEntry::Item {
        id: "delete-lane",
        label: "Delete lane".into(),
        icon: Some("trash-2"),
        danger: true,
        disabled: false,
    });
    items
}

/// A session row's menu: adopt, stop, archive.
pub fn session_menu_items(
    session: &Session,
    adoptable: bool,
    lane_actions: bool,
) -> Vec<MenuEntry> {
    let mut items = Vec::new();
    if adoptable {
        items.push(MenuEntry::item("adopt", "Adopt into lane", None));
    }
    if session.open {
        items.push(MenuEntry::item("stop", "Stop", None));
    }
    if lane_actions && session.lane_id.is_some() {
        items.push(MenuEntry::item(
            "archive",
            if session.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            None,
        ));
    }
    items
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn session(key: &str) -> Session {
        Session {
            key: key.into(),
            id: None,
            lane_id: None,
            harness: "pi".into(),
            harness_ref: None,
            cwd: "/x".into(),
            title: key.into(),
            tags: Vec::new(),
            created_at: 0,
            updated_at: 0,
            message_count: 1,
            parent_locator: None,
            project_root: "/x".into(),
            checkout: None,
            project_id: None,
            pinned: false,
            archived: false,
            open: false,
        }
    }

    fn lane(state: LaneState) -> Lane {
        Lane {
            id: 1,
            project_id: 1,
            name: "l".into(),
            task: "t".into(),
            branch: "b".into(),
            state,
            worktrees: Vec::new(),
        }
    }

    #[test]
    fn rows_say_what_is_their_own() {
        let mut s = session("a");
        s.message_count = 3;
        assert_eq!(session_detail(&s, None), "3 messages");
        s.archived = true;
        assert_eq!(
            session_detail(&s, Some("Proj / task")),
            "Archived · Proj / task"
        );
        assert!(is_live(SessionStatus::Failed));
        assert!(!is_live(SessionStatus::Done));
    }

    #[test]
    fn the_main_session_is_the_open_root_else_the_latest() {
        let mut a = session("a");
        a.updated_at = 5;
        let mut b = session("b");
        b.updated_at = 9;
        let mut child = session("c");
        child.parent_locator = Some("p".into());
        child.updated_at = 99;
        let list = vec![a.clone(), b.clone(), child];
        assert_eq!(lane_main_session(&list).unwrap().key, "b");
        let mut open = a;
        open.open = true;
        let list = vec![open, b];
        assert_eq!(lane_main_session(&list).unwrap().key, "a");
    }

    #[test]
    fn opening_an_earlier_record_would_stop_the_live_one() {
        let mut live = session("panoptes:1");
        live.id = Some(1);
        live.lane_id = Some(7);
        live.open = true;
        let mut earlier = session("panoptes:2");
        earlier.id = Some(2);
        earlier.lane_id = Some(7);
        let list = vec![live.clone(), earlier.clone()];
        assert_eq!(
            lane_session_to_stop(&earlier, &list).unwrap().key,
            "panoptes:1"
        );
        assert!(lane_session_to_stop(&live, &list).is_none());
    }

    #[test]
    fn empty_lanes_start_or_wait_for_the_writer() {
        assert_eq!(
            empty_lane(&lane(LaneState::Active), None),
            Some(EmptyLane::Startable)
        );
        assert_eq!(empty_lane(&lane(LaneState::Archived), None), None);
        let nav = LaneNavigation {
            lane_id: 1,
            sessions: Vec::new(),
            archived_sessions: Vec::new(),
            work: None,
            busy_writer: Some(panoptes_core::service::BusyWriter {
                session_key: "k".into(),
                title: "Fix it".into(),
            }),
        };
        assert_eq!(
            empty_lane(&lane(LaneState::Active), Some(&nav)),
            Some(EmptyLane::Busy {
                writer: "Fix it".into()
            })
        );
    }

    #[test]
    fn work_labels() {
        assert_eq!(work_label(None), None);
        let work = |changed_files, commits_ahead| LaneWork {
            changed_files,
            commits_ahead,
        };
        assert_eq!(
            work_label(Some(&work(0, Some(0)))).unwrap(),
            "No changes yet"
        );
        assert_eq!(
            work_label(Some(&work(6, Some(2)))).unwrap(),
            "6 files changed · 2 commits ahead"
        );
        assert_eq!(work_label(Some(&work(1, None))).unwrap(), "1 file changed");
    }

    #[test]
    fn menus_offer_what_applies() {
        assert!(matches!(
            &project_menu_items(2)[3],
            MenuEntry::Item { disabled: true, .. }
        ));
        let ids = |items: Vec<MenuEntry>| -> Vec<&'static str> {
            items
                .into_iter()
                .filter_map(|i| match i {
                    MenuEntry::Item { id, .. } => Some(id),
                    MenuEntry::Separator => None,
                })
                .collect()
        };
        assert_eq!(
            ids(lane_menu_items(&lane(LaneState::Archived))),
            ["restore-lane", "delete-lane"]
        );
        let mut s = session("k");
        s.open = true;
        s.lane_id = Some(1);
        assert_eq!(
            ids(session_menu_items(&s, true, true)),
            ["adopt", "stop", "archive"]
        );
    }
}
