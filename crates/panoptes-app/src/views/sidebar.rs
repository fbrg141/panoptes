//! The sidebar: header, the Lanes and All sessions tabs, projects with their
//! lanes, and session rows.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::service::{AdoptionOutcome, LaneNavigation, Session};
use panoptes_core::store::{Lane, LaneState, Project};
use panoptes_harness::SessionStatus;

use crate::ds::{Button, LoopState, Tab, Type as _, icon, loop_mark, status_word, tabs};
use crate::model::format::{now_ms, relative_time};
use crate::model::sidebar::{
    EmptyLane, MenuEntry, ProjectOpen, empty_lane, is_live, lane_main_session, lane_menu_items,
    main_checkout_lane, primary_action, project_menu_items, project_open, session_detail,
    session_row_menu_items, work_label,
};
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::{Overlay, Ui};
use crate::views::menu::{OpenMenu, popup_menu};
use crate::zoom::px;

/// What the sidebar asks of the window: flows that open dialogs it does not own.
#[derive(Debug, Clone)]
pub enum SidebarEvent {
    RemoveProject(i64),
    ArchiveLane(i64),
    DeleteLane(i64),
    Adopt(Box<Session>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavTab {
    Lanes,
    All,
}

#[derive(Clone)]
enum Owner {
    Project(i64),
    Lane(i64),
    /// A session row; `lane` when the row is its lane's card.
    Session {
        session: Box<Session>,
        lane: Option<i64>,
    },
}

/// Everything a row needs to know about the store, read once per frame.
struct Snapshot {
    statuses: HashMap<String, SessionStatus>,
    active_key: Option<String>,
    now: i64,
}

impl Snapshot {
    fn status(&self, key: &str) -> SessionStatus {
        self.statuses
            .get(key)
            .copied()
            .unwrap_or(SessionStatus::Idle)
    }
}

/// How one session row is drawn.
struct Row<'a> {
    session: &'a Session,
    /// A lane card keeps the lane's task as its visible title.
    title: Option<String>,
    context: Option<String>,
    /// Checkout location and kind, shown before the rest of the detail.
    checkout: Option<(String, bool)>,
    lane_actions: bool,
    nested: bool,
    adoptable: bool,
    /// The lane this row is the card of.
    lane: Option<&'a Lane>,
    /// Whether the card's extra sessions are shown, when it has any.
    expand: Option<bool>,
    muted: bool,
}

pub struct Sidebar {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    tab: NavTab,
    collapsed: HashSet<i64>,
    archived_lanes_shown: HashSet<i64>,
    expanded: HashSet<i64>,
    archived_sessions_shown: HashSet<i64>,
    /// Lanes a session is being started or adopted into, so a second click
    /// before the first lands does not start another.
    starting: HashSet<i64>,
    menu: Option<OpenMenu<Owner>>,
    scroll: ScrollHandle,
    _clock: Task<()>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

/// One shared clock for every row's relative time, so "2m ago" keeps up.
const TICK: Duration = Duration::from_secs(30);

impl Sidebar {
    pub fn new(store: Entity<AppStore>, ui: Entity<Ui>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            store,
            ui,
            tab: NavTab::Lanes,
            collapsed: HashSet::new(),
            archived_lanes_shown: HashSet::new(),
            expanded: HashSet::new(),
            archived_sessions_shown: HashSet::new(),
            starting: HashSet::new(),
            menu: None,
            scroll: ScrollHandle::new(),
            _clock: clock,
        }
    }

    fn toggle(set: &mut HashSet<i64>, id: i64) {
        if !set.remove(&id) {
            set.insert(id);
        }
    }

    fn request_open(&mut self, session: &Session, cx: &mut Context<Self>) {
        crate::ui::request_open(&self.store, &self.ui, session, None, cx);
    }

    fn work(&mut self, lane_id: i64, cx: &mut Context<Self>) {
        if !self.starting.insert(lane_id) {
            return;
        }
        let created = self.store.update(cx, |store, cx| {
            store.create(lane_id, Default::default(), cx)
        });
        let ui = self.ui.clone();
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            created.await;
            // The lane stays guarded until a listing shows its new session;
            // until then a click would still find it empty and start another.
            store.update(cx, |store, cx| store.refresh(cx)).await;
            let _ = this.update(cx, |this, _| this.starting.remove(&lane_id));
            cx.update(|cx| {
                if let Some(key) = store.read(cx).state.active_key.clone() {
                    ui.update(cx, |ui, cx| ui.show_session(&key, None, cx));
                }
            });
        })
        .detach();
    }

    /// Adopts a conversation into `lane_id`, then opens it under its new key.
    fn adopt_and_open(&mut self, session: Session, lane_id: i64, cx: &mut Context<Self>) {
        if !self.starting.insert(lane_id) {
            return;
        }
        let store = self.store.clone();
        let adopted = store.update(cx, |store, cx| store.adopt(&session, Some(lane_id), cx));
        cx.spawn(async move |this, cx| {
            let outcome = adopted.await.flatten();
            // As in `work`: keep the lane guarded until the listing has the
            // adopted session, and open the record the listing has for it,
            // since the service may return a record it already kept.
            store.update(cx, |store, cx| store.refresh(cx)).await;
            let _ = this.update(cx, |this, cx| {
                this.starting.remove(&lane_id);
                if let Some(AdoptionOutcome::Adopted { key }) = outcome {
                    let listed = this
                        .store
                        .read(cx)
                        .state
                        .sessions
                        .iter()
                        .find(|s| s.key == key)
                        .cloned();
                    let session = listed.unwrap_or(Session {
                        key,
                        lane_id: Some(lane_id),
                        ..session
                    });
                    this.request_open(&session, cx);
                }
            });
        })
        .detach();
    }

    fn add_project(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.pick_project(cx));
    }

    fn open_menu(
        &mut self,
        owner: Owner,
        items: Vec<MenuEntry>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        self.menu = Some(OpenMenu {
            owner,
            items,
            position,
        });
        cx.notify();
    }

    fn select(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        cx.notify();
        match menu.owner {
            Owner::Project(project_id) => match id {
                "new-lane" => self.ui.update(cx, |ui, cx| {
                    ui.open(
                        Overlay::NewLane {
                            project_id,
                            prompt: None,
                        },
                        cx,
                    )
                }),
                "cleanup-lanes" => self
                    .ui
                    .update(cx, |ui, cx| ui.open(Overlay::Cleanup { project_id }, cx)),
                "remove-project" => cx.emit(SidebarEvent::RemoveProject(project_id)),
                _ => {}
            },
            Owner::Lane(lane_id) => self.select_lane_action(lane_id, id, cx),
            Owner::Session { session, lane } => {
                match id {
                    "adopt" => cx.emit(SidebarEvent::Adopt(session.clone())),
                    "stop" => {
                        let key = session.key.clone();
                        self.store.update(cx, |store, cx| store.close(key, cx));
                    }
                    "archive" => {
                        let (key, archived) = (session.key.clone(), !session.archived);
                        self.store
                            .update(cx, |store, cx| store.set_archived(key, archived, cx));
                    }
                    _ => {}
                }
                if let Some(lane_id) = lane {
                    self.select_lane_action(lane_id, id, cx);
                }
            }
        }
    }

    fn select_lane_action(&mut self, lane_id: i64, id: &str, cx: &mut Context<Self>) {
        let restoring = self
            .store
            .read(cx)
            .state
            .lanes
            .iter()
            .any(|l| l.id == lane_id && l.state == LaneState::Archived);
        match id {
            "new-session" => {
                self.store
                    .update(cx, |store, cx| store.new_lane_session(lane_id, cx).detach());
            }
            "resume-session" => self.ui.update(cx, |ui, cx| {
                ui.open(
                    Overlay::ResumeSession {
                        lane_id,
                        session_id: None,
                    },
                    cx,
                )
            }),
            "archive-lane" => cx.emit(SidebarEvent::ArchiveLane(lane_id)),
            "restore-lane" if restoring => {
                self.store
                    .update(cx, |store, cx| store.restore_lane(lane_id, cx));
            }
            "delete-lane" => cx.emit(SidebarEvent::DeleteLane(lane_id)),
            _ => {}
        }
    }

    // --- Rendering ---------------------------------------------------------

    fn render_header(&self, p: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui.clone();
        let open = move |overlay: Overlay| {
            let ui = ui.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                ui.update(cx, |ui, cx| ui.open(overlay.clone(), cx))
            }
        };
        let this = cx.entity().downgrade();
        let diagnostics_store = self.store.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(38.))
            .px(px(10.))
            .child(loop_mark("brand", 18., LoopState::Idle, p.ink_muted))
            .child(
                div()
                    .flex_1()
                    .t_body_strong()
                    .text_color(p.ink)
                    .child("Panoptes"),
            )
            .child(
                Button::new("search")
                    .ghost()
                    .small()
                    .icon("search")
                    .tooltip("Search projects, lanes and sessions (⌘P)")
                    .on_click(open(Overlay::Search)),
            )
            .child(
                Button::new("diagnostics")
                    .ghost()
                    .small()
                    .icon("watch")
                    .tooltip("Open diagnostics window")
                    .on_click(move |_, _, cx| {
                        crate::views::diagnostics::open(diagnostics_store.clone(), cx);
                    }),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .small()
                    .icon("settings")
                    .tooltip("Appearance, profiles and preferences")
                    .on_click(open(Overlay::Settings)),
            )
            .child(
                Button::new("add-project")
                    .ghost()
                    .small()
                    .icon("folder")
                    .tooltip("Add a git repository as a project (⌘N)")
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| this.add_project(cx));
                    }),
            )
    }

    fn render_project(
        &self,
        project: &Project,
        lanes: &[&Lane],
        navigation: &HashMap<i64, &LaneNavigation>,
        snap: &Snapshot,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = project.id;
        let collapsed = self.collapsed.contains(&id);
        let group = SharedString::from(format!("project-{id}"));
        let this = cx.entity().downgrade();
        let toggle = {
            let this = this.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    Self::toggle(&mut this.collapsed, id);
                    cx.notify();
                });
            }
        };
        // Clicking the project name opens its main checkout. The chevron to
        // its left still folds the lane list, and the menu still acts on the
        // project.
        // Without a main-checkout lane the name folds the list, as before.
        let opens_checkout = main_checkout_lane(&self.store.read(cx).state.lanes, id).is_some();
        let open_project = {
            let this = this.clone();
            let repo = project.repo.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |this, cx| {
                    let action = {
                        let state = &this.store.read(cx).state;
                        project_open(id, &state.lanes, &state.navigation, &state.sessions, &repo)
                    };
                    match action {
                        Some(ProjectOpen::Open(session)) => this.request_open(&session, cx),
                        Some(ProjectOpen::Adopt { session, lane }) => {
                            this.adopt_and_open(session, lane, cx)
                        }
                        Some(ProjectOpen::Start { lane }) => this.work(lane, cx),
                        Some(ProjectOpen::Busy) => {}
                        None => {
                            Self::toggle(&mut this.collapsed, id);
                            cx.notify();
                        }
                    }
                });
            }
        };
        let menu_items = project_menu_items(lanes.len());
        let menu_open =
            matches!(&self.menu, Some(m) if matches!(m.owner, Owner::Project(o) if o == id));
        let reveal = |el: Div| {
            let group = group.clone();
            el.when(!menu_open, |el| {
                el.opacity(0.).group_hover(group, |s| s.opacity(1.))
            })
        };
        let header = div()
            .flex()
            .items_center()
            .min_h(px(32.))
            .px(px(8.))
            .text_color(p.ink_muted)
            .child(
                Button::new(SharedString::from(format!("collapse-{id}")))
                    .ghost()
                    .small()
                    .icon(if collapsed {
                        "chevron-right"
                    } else {
                        "chevron-down"
                    })
                    .on_click(toggle.clone()),
            )
            .child(
                div()
                    .id(SharedString::from(format!("project-name-{id}")))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .py(px(4.))
                    .t_caption()
                    .child(project.settings.name.to_uppercase())
                    .when(opens_checkout, |el| el.cursor_pointer())
                    .on_click(open_project),
            )
            .child(reveal(div()).child({
                let ui = self.ui.clone();
                Button::new(SharedString::from(format!("new-lane-{id}")))
                    .ghost()
                    .small()
                    .icon("plus")
                    .tooltip(format!("New lane in {}", project.settings.name))
                    .on_click(move |_, _, cx| {
                        ui.update(cx, |ui, cx| {
                            ui.open(
                                Overlay::NewLane {
                                    project_id: id,
                                    prompt: None,
                                },
                                cx,
                            )
                        })
                    })
            }))
            .child(reveal(div().ml(px(4.))).child({
                let this = this.clone();
                Button::new(SharedString::from(format!("project-menu-{id}")))
                    .ghost()
                    .small()
                    .icon("ellipsis-vertical")
                    .active(menu_open)
                    .tooltip(format!("Actions for project {}", project.settings.name))
                    .on_click(move |event, _, cx| {
                        let items = menu_items.clone();
                        let at = event.position();
                        let _ = this.update(cx, |this, cx| {
                            this.open_menu(Owner::Project(id), items, at, cx)
                        });
                    })
            }));

        let mut section = div().group(group).mt(px(8.)).child(header);
        if !collapsed {
            let active: Vec<&Lane> = lanes
                .iter()
                .copied()
                .filter(|l| l.state == LaneState::Active)
                .collect();
            let archived: Vec<&Lane> = lanes
                .iter()
                .copied()
                .filter(|l| l.state == LaneState::Archived)
                .collect();
            let shown = self.archived_lanes_shown.contains(&id);
            let mut body = div().ml(px(12.)).pl(px(4.));
            for lane in &active {
                body = body.child(self.render_lane(
                    lane,
                    navigation.get(&lane.id).copied(),
                    snap,
                    p,
                    cx,
                ));
            }
            if shown {
                for lane in &archived {
                    body = body.child(self.render_lane(
                        lane,
                        navigation.get(&lane.id).copied(),
                        snap,
                        p,
                        cx,
                    ));
                }
            }
            if active.is_empty() {
                body = body.child(
                    div()
                        .px(px(12.))
                        .py(px(4.))
                        .t_body()
                        .text_color(p.ink_muted)
                        .child("No active lanes."),
                );
            }
            if !archived.is_empty() {
                let this = this.clone();
                body = body.child(
                    div().flex().ml(px(8.)).child(
                        Button::new(SharedString::from(format!("archived-lanes-{id}")))
                            .ghost()
                            .small()
                            .label(if shown {
                                "Hide archived lanes".to_string()
                            } else {
                                format!("{} archived lanes", archived.len())
                            })
                            .on_click(move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    Self::toggle(&mut this.archived_lanes_shown, id);
                                    cx.notify();
                                });
                            }),
                    ),
                );
            }
            section = section.child(body);
        }
        section.into_any_element()
    }

    /// A lane's root session is its card; only extra sessions disclose.
    fn render_lane(
        &self,
        lane: &Lane,
        navigation: Option<&LaneNavigation>,
        snap: &Snapshot,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let main_checkout = lane.worktrees.iter().any(|w| w.main_checkout);
        let archived = lane.state == LaneState::Archived;
        let checkpoint = lane.worktrees.iter().any(|w| w.checkpoint_ref.is_some());
        let empty = empty_lane(lane, navigation);
        let work = work_label(navigation.and_then(|n| n.work.as_ref()));
        let location = if main_checkout {
            "Main checkout".to_string()
        } else {
            lane.branch.clone()
        };
        let detail = [
            if archived {
                if checkpoint {
                    "archived with a checkpoint".to_string()
                } else {
                    "archived".to_string()
                }
            } else {
                String::new()
            },
            work.clone().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let lane_sessions: Vec<Session> = navigation
            .map(|n| {
                n.sessions
                    .iter()
                    .chain(n.archived_sessions.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let main = lane_main_session(&lane_sessions).cloned();
        let extras: Vec<&Session> = lane_sessions
            .iter()
            .filter(|s| Some(&s.key) != main.as_ref().map(|m| &m.key))
            .collect();
        let subsessions: Vec<&Session> = if archived {
            extras.clone()
        } else {
            extras.iter().copied().filter(|s| !s.archived).collect()
        };
        let archived_extras: Vec<&Session> = if archived {
            Vec::new()
        } else {
            extras.iter().copied().filter(|s| s.archived).collect()
        };
        let lane_id = lane.id;
        let this = cx.entity().downgrade();
        let archived_list = (!archived_extras.is_empty()).then(|| {
            let shown = self.archived_sessions_shown.contains(&lane_id);
            let this = this.clone();
            let mut list = div().child(
                div().flex().child(
                    Button::new(SharedString::from(format!("archived-sessions-{lane_id}")))
                        .ghost()
                        .small()
                        .label(if shown {
                            "Hide archived sessions".to_string()
                        } else {
                            format!("{} archived sessions", archived_extras.len())
                        })
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                Self::toggle(&mut this.archived_sessions_shown, lane_id);
                                cx.notify();
                            });
                        }),
                ),
            );
            if shown {
                for session in &archived_extras {
                    list = list.child(self.render_session(
                        Row {
                            session,
                            title: None,
                            context: None,
                            checkout: None,
                            lane_actions: true,
                            nested: true,
                            adoptable: false,
                            lane: None,
                            expand: None,
                            muted: false,
                        },
                        snap,
                        p,
                        cx,
                    ));
                }
            }
            list
        });

        if let Some(main) = &main {
            let expanded = self.expanded.contains(&lane_id);
            let expandable = !extras.is_empty();
            let card = self.render_session(
                Row {
                    session: main,
                    title: Some(lane.task.clone()),
                    context: Some(detail),
                    checkout: Some((location, main_checkout)),
                    lane_actions: true,
                    nested: false,
                    adoptable: false,
                    lane: Some(lane),
                    expand: expandable.then_some(expanded),
                    muted: archived,
                },
                snap,
                p,
                cx,
            );
            let mut item = div().child(card);
            if expanded && expandable {
                let mut more = div().ml(px(16.));
                for session in &subsessions {
                    more = more.child(self.render_session(
                        Row {
                            session,
                            title: None,
                            context: None,
                            checkout: None,
                            lane_actions: true,
                            nested: true,
                            adoptable: false,
                            lane: None,
                            expand: None,
                            muted: false,
                        },
                        snap,
                        p,
                        cx,
                    ));
                }
                item = item.child(more.children(archived_list));
            }
            return item.into_any_element();
        }

        let group = SharedString::from(format!("lane-{lane_id}"));
        let startable = empty == Some(EmptyLane::Startable);
        let menu_open =
            matches!(&self.menu, Some(m) if matches!(m.owner, Owner::Lane(o) if o == lane_id));
        let items = lane_menu_items(lane);
        let checkout_line = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .map(|el| el.t_code_sm().text_color(p.ink_muted))
            .child(checkout_icon(main_checkout, p))
            .child(div().truncate().child(format!(
                "{location}{}",
                if archived {
                    if checkpoint {
                        " · archived with a checkpoint"
                    } else {
                        " · archived"
                    }
                } else {
                    ""
                }
            )));
        let hit = div()
            .id(SharedString::from(format!("lane-hit-{lane_id}")))
            .flex_1()
            .min_w_0()
            .py(px(4.))
            .pl(px(8.))
            .pr(px(32.))
            .when(startable, |el| {
                let this = this.clone();
                el.cursor_pointer().on_click(move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.work(lane_id, cx));
                })
            })
            .child(div().truncate().t_body().child(lane.task.clone()))
            .child(checkout_line)
            .children(work.map(|w| {
                div()
                    .truncate()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(p.ink_muted)
                    .child(w)
            }));
        let menu = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(4.))
            .flex()
            .items_center()
            .when(!menu_open, |el| {
                el.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.))
            })
            .child({
                let this = this.clone();
                Button::new(SharedString::from(format!("lane-menu-{lane_id}")))
                    .ghost()
                    .small()
                    .icon("ellipsis-vertical")
                    .active(menu_open)
                    .tooltip(format!("Actions for lane {}", lane.name))
                    .on_click(move |event, _, cx| {
                        let (items, at) = (items.clone(), event.position());
                        let _ = this.update(cx, |this, cx| {
                            this.open_menu(Owner::Lane(lane_id), items, at, cx)
                        });
                    })
            });
        let mut below = div().ml(px(16.)).children(archived_list);
        if startable {
            let this = this.clone();
            below = below.child(
                div().flex().ml(px(8.)).child(
                    Button::new(SharedString::from(format!("start-{lane_id}")))
                        .ghost()
                        .small()
                        .label("Start session")
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| this.work(lane_id, cx));
                        }),
                ),
            );
        }
        if let Some(EmptyLane::Busy { writer }) = &empty {
            below = below.child(
                div()
                    .ml(px(8.))
                    .py(px(4.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(p.ink_muted)
                    .child(format!(
                        "“{writer}” is changing this lane. Start a session here once it is Done."
                    )),
            );
        }
        div()
            .when(archived, |el| el.text_color(p.ink_muted))
            .child(
                div()
                    .group(group)
                    .relative()
                    .flex()
                    .items_center()
                    .min_h(px(48.))
                    .px(px(4.))
                    .child(hit)
                    .child(menu),
            )
            .child(below)
            .into_any_element()
    }

    /// A session in the sidebar. The whole row opens it; Stop (while it runs)
    /// and the actions menu sit over the meta column and show on hover.
    fn render_session(
        &self,
        row: Row,
        snap: &Snapshot,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = row.session;
        let key = session.key.clone();
        let status = snap.status(&key);
        let active = snap.active_key.as_deref() == Some(key.as_str());
        let items = session_row_menu_items(session, row.adoptable, row.lane_actions, row.lane);
        let primary = primary_action(session, status);
        let has_actions = !items.is_empty() || primary;
        let word = status_word(status);
        let detail = session_detail(session, row.context.as_deref());
        let time = relative_time(session.updated_at, snap.now.max(session.updated_at));
        let title = row.title.clone().unwrap_or_else(|| session.title.clone());
        let group = SharedString::from(format!("session-{key}"));
        let owner = Owner::Session {
            session: Box::new(session.clone()),
            lane: row.lane.map(|l| l.id),
        };
        let menu_open = matches!(&self.menu, Some(m) if matches!(&m.owner, Owner::Session { session, .. } if session.key == key));
        let this = cx.entity().downgrade();

        let mut second = div()
            .flex()
            .items_center()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(p.ink_muted);
        if let Some(word) = word {
            let tone = match status {
                SessionStatus::Running => p.running,
                SessionStatus::Waiting => p.waiting,
                SessionStatus::Failed => p.error,
                SessionStatus::Done => p.done,
                SessionStatus::Idle => p.ink_muted,
            };
            second = second
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tone)
                        .child(word),
                )
                .child(div().child("\u{a0}·\u{a0}"));
        }
        // The rest is one run of text, so it is cut at its end, as on the web.
        let mut rest = detail;
        if let Some((label, main_checkout)) = &row.checkout {
            second = second.child(div().mr(px(4.)).child(checkout_icon(*main_checkout, p)));
            rest = if rest.is_empty() {
                label.clone()
            } else {
                format!("{label}\u{a0}·\u{a0}{rest}")
            };
        }
        second = second.child(div().min_w_0().truncate().child(rest));

        let hit = div()
            .id(SharedString::from(format!("hit-{key}")))
            .on_drag(
                crate::ui::DraggedSession { key: key.clone() },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            )
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .min_h(px(44.))
            .py(px(4.))
            // Reserve the full width of the absolutely positioned action
            // buttons so their hover/open state never covers the label.
            .pr(px(action_inset(primary, !items.is_empty())))
            .pl(px(if row.expand.is_some() { 32. } else { 4. }))
            .rounded(px(4.))
            .text_color(p.ink)
            .cursor_pointer()
            .on_click({
                let this = this.clone();
                let session = session.clone();
                move |_, _, cx| {
                    let _ = this.update(cx, |this, cx| this.request_open(&session, cx));
                }
            })
            .on_mouse_down(MouseButton::Right, {
                let this = this.clone();
                let items = items.clone();
                let owner = owner.clone();
                move |event, _, cx| {
                    let (items, owner, at) = (items.clone(), owner.clone(), event.position);
                    let _ = this.update(cx, |this, cx| this.open_menu(owner, items, at, cx));
                }
            })
            .child(
                div()
                    .w(px(24.))
                    .h(px(16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(row_glyph(&key, status, p)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .map(|el| {
                                if is_live(status) {
                                    el.t_body_strong()
                                } else {
                                    el.t_body()
                                }
                            })
                            .when(session.archived || row.muted, |el| {
                                el.text_color(p.ink_muted)
                            })
                            .child(title),
                    )
                    .child(second),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .flex_none()
                    .min_w(px(if row.lane.is_some() { 0. } else { 52. }))
                    .font_family(crate::theme::FONT_MONO)
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .text_color(p.ink_muted)
                    .when(has_actions && !menu_open, |el| {
                        el.group_hover(group.clone(), |s| s.opacity(0.))
                    })
                    .when(has_actions && menu_open, |el| el.opacity(0.))
                    .when(row.lane.is_none(), |el| el.child(time)),
            );

        let actions = has_actions.then(|| {
            let mut actions = div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(8.))
                .flex()
                .items_center()
                .gap(px(4.))
                .when(!menu_open, |el| {
                    el.opacity(0.)
                        .invisible()
                        .group_hover(group.clone(), |s| s.opacity(1.).visible())
                });
            if primary {
                let key = key.clone();
                let store = self.store.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("stop-{key}")))
                        .ghost()
                        .small()
                        .icon("square")
                        .tooltip("Stop")
                        .on_click(move |_, _, cx| {
                            let key = key.clone();
                            store.update(cx, |store, cx| store.close(key, cx));
                        }),
                );
            }
            if !items.is_empty() {
                let this = this.clone();
                actions = actions.child(
                    Button::new(SharedString::from(format!("menu-{key}")))
                        .ghost()
                        .small()
                        .icon("ellipsis-vertical")
                        .active(menu_open)
                        .tooltip(format!("Actions for session {}", session.title))
                        .on_click(move |event, _, cx| {
                            let (items, owner, at) =
                                (items.clone(), owner.clone(), event.position());
                            let _ =
                                this.update(cx, |this, cx| this.open_menu(owner, items, at, cx));
                        }),
                );
            }
            actions
        });

        let expand = row.expand.map(|expanded| {
            let lane_id = row.lane.map(|l| l.id).unwrap_or_default();
            let this = this.clone();
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .flex()
                .items_center()
                .child(
                    Button::new(SharedString::from(format!("expand-{lane_id}")))
                        .ghost()
                        .small()
                        .icon(if expanded {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        })
                        .on_click(move |_, _, cx| {
                            let _ = this.update(cx, |this, cx| {
                                Self::toggle(&mut this.expanded, lane_id);
                                cx.notify();
                            });
                        }),
                )
        });

        let selection = p.selection;
        let raised = p.surface_raised;
        div()
            .when(row.nested, |el| el.ml(px(8.)))
            .child(
                div()
                    .id(SharedString::from(format!("row-{key}")))
                    .group(group)
                    .relative()
                    .rounded(px(4.))
                    .when(active, |el| el.bg(selection))
                    .when(!active, |el| el.hover(move |s| s.bg(raised)))
                    .children(expand)
                    .child(hit)
                    .children(actions),
            )
            .into_any_element()
    }
}

/// Space kept at a session row's right edge for its hover/open actions.
fn action_inset(primary: bool, menu: bool) -> f32 {
    let buttons = usize::from(primary) + usize::from(menu);
    if buttons == 0 {
        8.
    } else {
        8. + buttons as f32 * crate::ds::CONTROL_SM + (buttons - 1) as f32 * 4.
    }
}

/// One optical size for every status: a 16px box, each shape filling it evenly.
fn row_glyph(key: &str, status: SessionStatus, p: &Palette) -> AnyElement {
    match status {
        SessionStatus::Running => loop_mark(
            SharedString::from(format!("glyph-{key}")),
            20.,
            LoopState::Thinking,
            p.running,
        ),
        SessionStatus::Waiting => div()
            .size(px(10.))
            .rounded_full()
            .border_2()
            .border_color(p.waiting)
            .into_any_element(),
        SessionStatus::Failed => icon("circle-alert", 16.)
            .text_color(p.error)
            .into_any_element(),
        SessionStatus::Done => icon("check", 16.).text_color(p.done).into_any_element(),
        SessionStatus::Idle => div()
            .size(px(8.))
            .rounded_full()
            .border(px(1.5))
            .border_color(p.ink_muted)
            .into_any_element(),
    }
}

/// Distinguishes a lane's main checkout from an isolated worktree.
fn checkout_icon(main_checkout: bool, p: &Palette) -> crate::ds::Icon {
    icon(
        if main_checkout {
            "folder"
        } else {
            "git-branch"
        },
        12.,
    )
    .text_color(p.ink_muted)
}

/// "Project / task" for a session listed outside its lane.
fn session_context(session: &Session, projects: &[Project], lanes: &[Lane]) -> Option<String> {
    let lane = lanes.iter().find(|l| Some(l.id) == session.lane_id);
    let project = lanes
        .iter()
        .find(|l| Some(l.id) == session.lane_id)
        .and_then(|l| projects.iter().find(|p| p.id == l.project_id));
    match (project, lane) {
        (Some(project), Some(lane)) => Some(format!("{} / {}", project.settings.name, lane.task)),
        _ => None,
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let state = &self.store.read(cx).state;
        let snap = Snapshot {
            statuses: state.statuses.clone(),
            active_key: state.active_key.clone(),
            now: now_ms(),
        };
        let sessions = state.sessions.clone();
        let projects = state.projects.clone();
        let lanes = state.lanes.clone();
        let navigation_list = state.navigation.clone();
        let list_errors = state.list_errors.clone();
        let navigation: HashMap<i64, &LaneNavigation> =
            navigation_list.iter().map(|n| (n.lane_id, n)).collect();

        let this = cx.entity().downgrade();
        let tab_bar = tabs(
            vec![
                Tab {
                    id: "lanes",
                    label: "Lanes".into(),
                    icon: None,
                    count: None,
                },
                Tab {
                    id: "all",
                    label: "All sessions".into(),
                    icon: None,
                    count: Some(sessions.len()),
                },
            ],
            if self.tab == NavTab::Lanes {
                "lanes"
            } else {
                "all"
            },
            &p,
            move |id, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.tab = if id == "lanes" {
                        NavTab::Lanes
                    } else {
                        NavTab::All
                    };
                    cx.notify();
                });
            },
        );

        let mut list = div()
            .id("sidebar-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb(px(16.));
        match self.tab {
            NavTab::Lanes => {
                for project in &projects {
                    let project_lanes: Vec<&Lane> = lanes
                        .iter()
                        .filter(|l| l.project_id == project.id)
                        .collect();
                    list = list.child(self.render_project(
                        project,
                        &project_lanes,
                        &navigation,
                        &snap,
                        &p,
                        cx,
                    ));
                }
                if projects.is_empty() {
                    list = list.child(
                        div()
                            .px(px(12.))
                            .t_body()
                            .text_color(p.ink_muted)
                            .child("No projects yet. Add one with the folder button."),
                    );
                }
            }
            NavTab::All => {
                for session in &sessions {
                    let context = session_context(session, &projects, &lanes);
                    list = list.child(self.render_session(
                        Row {
                            session,
                            title: None,
                            context,
                            checkout: None,
                            lane_actions: false,
                            nested: false,
                            adoptable: session.lane_id.is_none(),
                            lane: None,
                            expand: None,
                            muted: false,
                        },
                        &snap,
                        &p,
                        cx,
                    ));
                }
                if sessions.is_empty() {
                    list = list.child(
                        div()
                            .px(px(12.))
                            .t_body()
                            .text_color(p.ink_muted)
                            .child("No harness sessions found."),
                    );
                }
            }
        }

        let menu = self.menu.as_ref().map(|menu| {
            let select = cx.entity().downgrade();
            let dismiss = cx.entity().downgrade();
            popup_menu(
                &menu.items,
                menu.position,
                &p,
                move |id, _, cx| {
                    let _ = select.update(cx, |this, cx| this.select(id, cx));
                },
                move |_, cx| {
                    let _ = dismiss.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                },
            )
        });

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(280.))
            .h_full()
            .border_r_1()
            .border_color(p.line)
            .bg(p.surface)
            .text_color(p.ink)
            .child(self.render_header(&p, cx))
            .child(div().px(px(8.)).pt(px(8.)).child(tab_bar))
            .children(list_errors.into_iter().map(|error| {
                div()
                    .px(px(12.))
                    .pt(px(8.))
                    .pb(px(8.))
                    .t_body()
                    .text_color(p.error)
                    .child(error)
            }))
            .child(list)
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn session_actions_have_room_beside_the_label() {
        assert_eq!(action_inset(false, false), 8.);
        assert_eq!(action_inset(true, false), 32.);
        assert_eq!(action_inset(false, true), 32.);
        assert_eq!(action_inset(true, true), 60.);
    }
}
