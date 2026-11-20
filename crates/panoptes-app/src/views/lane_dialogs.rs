//! The confirmations a lane or project action can need: removing a project,
//! deleting a lane, preserving a dirty lane, recovering a detached HEAD and
//! choosing a lane to adopt a conversation into. Port of the dialogs in
//! `Sidebar.tsx`.

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::lanes::{ArchiveStrategy, DetachedHead};
use panoptes_core::service::{AdoptionOutcome, ArchiveLaneOutcome, DeleteLaneOutcome, Session};
use panoptes_core::store::{Lane, LaneState, Project};

use crate::ds::{Button, Type as _, dialog, v_flex_gap};
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Archive,
    Delete,
}

pub enum LaneDialog {
    RemoveProject(Project),
    DeleteLane(Lane),
    /// The lane has uncommitted changes to keep before archiving.
    Dirty(Lane),
    Detached {
        lane: Lane,
        head: DetachedHead,
        removal: Removal,
    },
    Adopt(Box<Session>),
}

pub struct LaneDialogs {
    store: Entity<AppStore>,
    current: Option<LaneDialog>,
    busy: bool,
}

impl LaneDialogs {
    pub fn new(store: Entity<AppStore>) -> Self {
        Self {
            store,
            current: None,
            busy: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    fn show(&mut self, dialog: Option<LaneDialog>, cx: &mut Context<Self>) {
        self.current = dialog;
        self.busy = false;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            self.show(None, cx);
        }
    }

    pub fn remove_project(&mut self, id: i64, cx: &mut Context<Self>) {
        let project = self
            .store
            .read(cx)
            .state
            .projects
            .iter()
            .find(|p| p.id == id)
            .cloned();
        self.show(project.map(LaneDialog::RemoveProject), cx);
    }

    pub fn delete_lane(&mut self, id: i64, cx: &mut Context<Self>) {
        let lane = self.lane(id, cx);
        self.show(lane.map(LaneDialog::DeleteLane), cx);
    }

    fn lane(&self, id: i64, cx: &App) -> Option<Lane> {
        self.store
            .read(cx)
            .state
            .lanes
            .iter()
            .find(|l| l.id == id)
            .cloned()
    }

    /// Archives a lane, asking first only when it would lose work.
    pub fn archive(
        &mut self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
        recreate_branch: bool,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        cx.notify();
        let task = self.store.update(cx, |store, cx| {
            store.archive_lane(id, strategy, recreate_branch, cx)
        });
        cx.spawn(async move |this, cx| {
            let outcome = task.await.flatten();
            let _ = this.update(cx, |this, cx| {
                let removal = match &this.current {
                    Some(LaneDialog::Detached { removal, .. }) => *removal,
                    _ => Removal::Archive,
                };
                match outcome {
                    Some(ArchiveLaneOutcome::Archived) => this.show(None, cx),
                    Some(ArchiveLaneOutcome::Dirty { lane }) => {
                        this.show(Some(LaneDialog::Dirty(lane)), cx)
                    }
                    Some(ArchiveLaneOutcome::DetachedHead {
                        lane,
                        detached_head,
                    }) => this.show(
                        Some(LaneDialog::Detached {
                            lane,
                            head: detached_head,
                            removal,
                        }),
                        cx,
                    ),
                    None => {
                        this.busy = false;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn delete(&mut self, id: i64, allow_detached: bool, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        let task = self
            .store
            .update(cx, |store, cx| store.delete_lane(id, allow_detached, cx));
        cx.spawn(async move |this, cx| {
            let outcome = task.await.flatten();
            let _ = this.update(cx, |this, cx| match outcome {
                Some(DeleteLaneOutcome::Deleted) => this.show(None, cx),
                Some(DeleteLaneOutcome::DetachedHead {
                    lane,
                    detached_head,
                }) => this.show(
                    Some(LaneDialog::Detached {
                        lane,
                        head: detached_head,
                        removal: Removal::Delete,
                    }),
                    cx,
                ),
                None => {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Adopts a conversation, asking for a lane when none matches.
    pub fn adopt(&mut self, session: Session, lane_id: Option<i64>, cx: &mut Context<Self>) {
        self.busy = lane_id.is_some();
        let task = self
            .store
            .update(cx, |store, cx| store.adopt(&session, lane_id, cx));
        cx.spawn(async move |this, cx| {
            let outcome = task.await.flatten();
            let _ = this.update(cx, |this, cx| match outcome {
                Some(AdoptionOutcome::ChooseLane) => {
                    this.show(Some(LaneDialog::Adopt(Box::new(session))), cx)
                }
                Some(AdoptionOutcome::Adopted { .. }) => this.show(None, cx),
                None => {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Runs a store call that answers whether it worked, closing on success.
    fn run(&mut self, task: Task<Option<bool>>, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = task.await.unwrap_or(false);
            let _ = this.update(cx, |this, cx| {
                if done {
                    this.show(None, cx);
                } else {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// Code inline in a dialog's prose.
fn code(text: impl Into<SharedString>, p: &Palette) -> Div {
    div()
        .t_code()
        .px(px(4.))
        .rounded(px(2.))
        .bg(p.surface_sunken)
        .text_color(p.ink)
        .child(text.into())
}

/// A paragraph that mixes prose and inline code.
fn prose(parts: Vec<AnyElement>) -> Div {
    div().flex().flex_wrap().items_baseline().children(parts)
}

fn words(text: &str) -> Vec<AnyElement> {
    // Each word is its own child so the paragraph wraps between them.
    text.split_inclusive(' ')
        .map(|w| div().child(w.replace(' ', "\u{a0}")).into_any_element())
        .collect()
}

impl Render for LaneDialogs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(current) = &self.current else {
            return div().into_any_element();
        };
        let p = *cx.palette();
        let busy = self.busy;
        let this = cx.entity().downgrade();
        let close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.close(cx));
            }
        };
        let cancel = {
            let this = this.clone();
            move || {
                let this = this.clone();
                Button::new("dialog-cancel")
                    .label("Cancel")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| this.close(cx));
                    })
            }
        };
        let actions = |buttons: Vec<Button>| {
            div()
                .flex()
                .gap(px(8.))
                .child(cancel())
                .children(buttons)
                .into_any_element()
        };

        let (title, width, body, buttons): (String, f32, AnyElement, Option<AnyElement>) =
            match current {
                LaneDialog::RemoveProject(project) => {
                    let id = project.id;
                    let this = this.clone();
                    (
                        format!("Remove {}?", project.settings.name),
                        480.,
                        prose(
                            [
                                words("Panoptes forgets this project and its settings. The repository at "),
                                vec![code(project.repo.clone(), &p).into_any_element()],
                                words(" and Pi conversation files are not changed."),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        )
                        .into_any_element(),
                        Some(actions(vec![
                            Button::new("remove-project")
                                .danger()
                                .label("Remove project")
                                .disabled(busy)
                                .on_click(move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        let task = this
                                            .store
                                            .update(cx, |store, cx| store.remove_project(id, cx));
                                        this.run(task, cx);
                                    });
                                }),
                        ])),
                    )
                }
                LaneDialog::DeleteLane(lane) => {
                    let id = lane.id;
                    let main = lane.worktrees.iter().any(|w| w.main_checkout);
                    let body = if main {
                        prose(words(
                            "This removes the lane and its Panoptes session records. The main checkout, its branch and your files are not changed. Pi conversation files are kept.",
                        ))
                    } else {
                        prose(
                            [
                                words("This permanently deletes the local worktree, including uncommitted changes, the local branch "),
                                vec![code(lane.branch.clone(), &p).into_any_element()],
                                words(", and this lane's recovery refs. Commits that exist only on this branch will be lost. The main repository, remote branches and Pi conversation files are kept."),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        )
                    };
                    let this = this.clone();
                    (
                        format!("Delete {}?", lane.name),
                        540.,
                        body.into_any_element(),
                        Some(actions(vec![
                            Button::new("delete-lane")
                                .danger()
                                .label("Delete permanently")
                                .disabled(busy)
                                .on_click(move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| this.delete(id, false, cx));
                                }),
                        ])),
                    )
                }
                LaneDialog::Dirty(lane) => {
                    let id = lane.id;
                    let strategy = |label: &'static str,
                                    strategy: ArchiveStrategy,
                                    primary: bool| {
                        let this = this.clone();
                        let button = Button::new(SharedString::from(format!("archive-{label}")))
                            .label(label)
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.archive(id, Some(strategy), false, cx)
                                });
                            });
                        if primary { button.primary() } else { button }
                    };
                    (
                        format!("Preserve {} before archiving", lane.name),
                        540.,
                        prose(
                            [
                                words("This lane has uncommitted changes. Nothing has been changed or removed yet. Commit them on "),
                                vec![code(lane.branch.clone(), &p).into_any_element()],
                                words(", stash them on that branch, or save a private Panoptes checkpoint that can be restored later."),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        )
                        .into_any_element(),
                        Some(actions(vec![
                            strategy("Commit", ArchiveStrategy::Commit, false),
                            strategy("Stash", ArchiveStrategy::Stash, false),
                            strategy("Keep as checkpoint", ArchiveStrategy::Checkpoint, true),
                        ])),
                    )
                }
                LaneDialog::Detached {
                    lane,
                    head,
                    removal,
                } => self.detached(lane, head, *removal, busy, &p, &this, actions),
                LaneDialog::Adopt(session) => {
                    let state = &self.store.read(cx).state;
                    let free: Vec<(i64, String)> = state
                        .lanes
                        .iter()
                        .filter(|lane| lane.state == LaneState::Active)
                        .filter(|lane| {
                            state
                                .navigation
                                .iter()
                                .find(|n| n.lane_id == lane.id)
                                .is_none_or(|n| {
                                    n.sessions.is_empty() && n.archived_sessions.is_empty()
                                })
                        })
                        .map(|lane| {
                            let project = state
                                .projects
                                .iter()
                                .find(|p| p.id == lane.project_id)
                                .map(|p| p.settings.name.clone())
                                .unwrap_or_default();
                            (lane.id, format!("{project} / {}", lane.task))
                        })
                        .collect();
                    let body = v_flex_gap(4.)
                        .child(div().pb(px(8.)).child(
                            "The conversation stays in the harness's own files. Only its Panoptes lane membership is added. Only one session runs in a lane at a time.",
                        ))
                        .when(free.is_empty(), |el| {
                            el.child(div().text_color(p.ink).child(
                                "No active lane without a session is available. Create a lane for a new task instead.",
                            ))
                        })
                        .children(free.into_iter().map(|(lane_id, label)| {
                            let this = this.clone();
                            let session = (**session).clone();
                            div().flex().child(
                                Button::new(SharedString::from(format!("adopt-{lane_id}")))
                                    .ghost()
                                    .full_width()
                                    .label(label)
                                    .disabled(busy)
                                    .on_click(move |_, _, cx| {
                                        let session = session.clone();
                                        let _ = this.update(cx, |this, cx| {
                                            this.adopt(session, Some(lane_id), cx)
                                        });
                                    }),
                            )
                        }));
                    (
                        format!("Adopt \u{201c}{}\u{201d} into a lane", session.title),
                        480.,
                        body.into_any_element(),
                        None,
                    )
                }
            };
        dialog(title, width, body, buttons, close, &p).into_any_element()
    }
}

impl LaneDialogs {
    #[allow(clippy::too_many_arguments)]
    fn detached(
        &self,
        lane: &Lane,
        head: &DetachedHead,
        removal: Removal,
        busy: bool,
        p: &Palette,
        this: &WeakEntity<Self>,
        actions: impl Fn(Vec<Button>) -> AnyElement,
    ) -> (String, f32, AnyElement, Option<AnyElement>) {
        let id = lane.id;
        let safe = head.is_safe_to_remove();
        let archive = removal == Removal::Archive;
        let title = format!(
            "{} {} before {}",
            if safe { "Recover" } else { "Save" },
            lane.name,
            if archive { "archiving" } else { "deletion" }
        );
        let mut buttons = Vec::new();
        if !safe {
            let this = this.clone();
            buttons.push(
                Button::new("create-branch")
                    .primary()
                    .label("Create branch here")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            let task = this
                                .store
                                .update(cx, |store, cx| store.create_lane_branch(id, cx));
                            this.run(task, cx);
                        });
                    }),
            );
        }
        if safe && archive {
            let this = this.clone();
            buttons.push(
                Button::new("recreate-archive")
                    .primary()
                    .label("Recreate branch and archive")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| this.archive(id, None, true, cx));
                    }),
            );
        }
        if safe {
            let this = this.clone();
            buttons.push(
                Button::new("delete-detached")
                    .danger()
                    .label("Delete lane")
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, cx| this.delete(id, true, cx));
                    }),
            );
        }
        let short = head.commit.chars().take(12).collect::<String>();
        let mut body = v_flex_gap(12.).child(prose(
            [
                words("The worktree HEAD is detached at "),
                vec![code(short, p).into_any_element()],
                words(if safe {
                    ". Its tree is clean, and this commit is already reachable from a local or remote branch. Removing the worktree would not lose that work."
                } else {
                    ". Nothing has been changed or removed, because archiving or deleting now could lose work."
                }),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        ));
        if !head.unreachable_commits.is_empty() {
            let intro = if archive {
                "These commits exist only at detached HEAD and are not on any local or remote branch:"
                    .to_string()
            } else {
                format!(
                    "These commits are not on any local or remote branch other than {}, which deleting the lane removes:",
                    lane.branch
                )
            };
            body = body.child(
                div().child(intro).child(
                    div()
                        .id("unreachable")
                        .mt(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .children(head.unreachable_commits.iter().map(|commit| {
                            let subject = if commit.subject.is_empty() {
                                "No commit subject".to_string()
                            } else {
                                commit.subject.clone()
                            };
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(code(commit.hash.chars().take(12).collect::<String>(), p))
                                .child(subject)
                        })),
                ),
            );
        }
        if head.dirty {
            body =
                body.child("The worktree has uncommitted changes that removal would also delete.");
        }
        let advice = if safe && archive {
            prose(
                [
                    words("Recreate "),
                    vec![code(lane.branch.clone(), p).into_any_element()],
                    words(" at this commit, then archive as usual. If the branch already exists elsewhere, Panoptes will advance it only when this commit is ahead of it; a diverged branch is never reset. Deleting the lane instead permanently removes its worktree, local branch and recovery refs. Commits that exist only on that local branch will be lost."),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            )
        } else if safe {
            prose(words(
                "Delete the lane to permanently remove its worktree, local branch and recovery refs. Commits that exist only on that local branch will be lost. The detached commit remains on its existing local or remote branch.",
            ))
        } else {
            prose(
                [
                    words("Create "),
                    vec![code(lane.branch.clone(), p).into_any_element()],
                    words(" at HEAD to keep the detached commits on a branch. The worktree stays in place, and you can then choose how to preserve any uncommitted changes. An existing diverged branch will not be moved."),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            )
        };
        body = body.child(advice);
        (title, 560., body.into_any_element(), Some(actions(buttons)))
    }
}
