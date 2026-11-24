//! Project-scoped worktree usage and explicit, checkpoint-safe bulk archive.

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::cleanup::LaneCleanup;
use panoptes_core::lanes::ArchivePreservation;

use crate::ds::{self, Button, RADIUS_MD, Type as _, icon};
use crate::model::format::{format_bytes, idle_label, now_ms, usage_label};
use crate::model::worktrees::{self, WorktreeTone};
use crate::store::AppStore;
use crate::theme::PaletteExt as _;
use crate::ui::Ui;
use crate::zoom::px;

struct Outcome {
    archived: usize,
    failed: Vec<String>,
}

pub struct Cleanup {
    project_id: i64,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    data: Option<LaneCleanup>,
    loading: bool,
    error: Option<String>,
    selected: BTreeSet<i64>,
    confirming: bool,
    busy: bool,
    outcome: Option<Outcome>,
    generation: u64,
    show_all_projects: bool,
    managed_only: bool,
    pruning: bool,
    focus: FocusHandle,
}

impl Cleanup {
    pub fn new(
        project_id: i64,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        // Hold the keyboard so Escape reaches the layer.
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            project_id,
            store,
            ui,
            data: None,
            loading: true,
            error: None,
            selected: BTreeSet::new(),
            confirming: false,
            busy: false,
            outcome: None,
            generation: 0,
            show_all_projects: true,
            managed_only: false,
            pruning: false,
            focus,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let request = self.generation;
        self.loading = true;
        self.error = None;
        cx.notify();
        let work = self.store.read(cx).lane_cleanup(self.project_id);
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(data) => {
                        let available: BTreeSet<i64> =
                            data.candidates.iter().map(|c| c.lane.id).collect();
                        this.selected.retain(|id| available.contains(id));
                        this.data = Some(data);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle(&mut self, lane_id: i64, checked: bool, cx: &mut Context<Self>) {
        if checked {
            self.selected.insert(lane_id);
        } else {
            self.selected.remove(&lane_id);
        }
        self.outcome = None;
        cx.notify();
    }

    fn prune(&mut self, cx: &mut Context<Self>) {
        if self.pruning || self.busy || self.confirming {
            return;
        }
        self.pruning = true;
        cx.notify();
        let task = self.store.update(cx, |store, cx| store.prune_worktrees(cx));
        cx.spawn(async move |this, cx| {
            let _ = task.await;
            let _ = this.update(cx, |this, cx| {
                this.pruning = false;
                this.load(cx);
            });
        })
        .detach();
    }

    fn primary(&mut self, cx: &mut Context<Self>) {
        if !self.confirming {
            self.outcome = None;
            self.confirming = true;
            cx.notify();
            return;
        }
        if self.busy || self.pruning || self.selected.is_empty() || self.data.is_none() {
            return;
        }
        self.busy = true;
        self.outcome = None;
        cx.notify();
        let ids: Vec<i64> = self.selected.iter().copied().collect();
        let task = self
            .store
            .update(cx, |store, cx| store.archive_lanes(ids, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let lanes = &this.store.read(cx).state.lanes;
                let failed = result
                    .failed
                    .iter()
                    .map(|(id, message)| {
                        let task = this
                            .data
                            .as_ref()
                            .and_then(|d| d.candidates.iter().find(|c| c.lane.id == *id))
                            .map(|c| c.lane.task.clone())
                            .or_else(|| lanes.iter().find(|l| l.id == *id).map(|l| l.task.clone()))
                            .unwrap_or_else(|| format!("Lane {id}"));
                        format!("{task}: {message}")
                    })
                    .collect();
                this.busy = false;
                this.confirming = false;
                this.selected = result.failed.iter().map(|(id, _)| *id).collect();
                this.outcome = Some(Outcome {
                    archived: result.archived,
                    failed,
                });
                this.load(cx);
            });
        })
        .detach();
    }

    fn close(&self, cx: &mut App) -> bool {
        if self.busy || self.pruning {
            return false;
        }
        self.ui.update(cx, |ui, cx| ui.close(cx));
        true
    }
}

impl Render for Cleanup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let this = cx.entity().downgrade();
        let name = self
            .store
            .read(cx)
            .state
            .projects
            .iter()
            .find(|p| p.id == self.project_id)
            .map(|p| p.settings.name.clone());
        let busy = self.busy || self.pruning;
        let pruning = self.pruning;
        let on_layer_close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.close(cx)).unwrap_or(false)
            }
        };
        let on_header_close: Option<ds::CloseHandler> = (!busy).then(|| {
            let this = this.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.close(cx));
            }) as ds::CloseHandler
        });

        let mut body = div().flex().flex_col().gap(px(16.));
        if self.loading && self.data.is_none() {
            body = body.child(
                div()
                    .py(px(24.))
                    .text_center()
                    .text_color(p.ink_muted)
                    .child("Measuring worktrees\u{2026}"),
            );
        }
        if let Some(error) = &self.error {
            let retry = this.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(12.))
                    .py(px(16.))
                    .child(div().text_color(p.error).child(error.clone()))
                    .child(
                        Button::new("cleanup-retry")
                            .icon("refresh-cw")
                            .label("Try again")
                            .on_click(move |_, _, cx| {
                                let _ = retry.update(cx, |this, cx| this.load(cx));
                            }),
                    ),
            );
        }
        if let Some(data) = &self.data {
            let usage_row = |label: &str, usage: String, first: bool| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .py(px(4.))
                    .when(!first, |el| el.border_t_1().border_color(p.line))
                    .child(div().text_color(p.ink_muted).child(label.to_string()))
                    .child(div().t_body_strong().child(usage))
            };
            body = body.child(
                div()
                    .rounded(px(RADIUS_MD))
                    .border_1()
                    .border_color(p.line)
                    .px(px(12.))
                    .py(px(8.))
                    .child(usage_row(
                        "This project",
                        usage_label(data.project.worktrees, data.project.bytes),
                        true,
                    ))
                    .child(usage_row(
                        "All projects",
                        usage_label(data.total.worktrees, data.total.bytes),
                        false,
                    )),
            );
            if !data.errors.is_empty() {
                body = body.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .text_color(p.error)
                        .child(icon("circle-alert", 16.).mt(px(2.)).flex_none())
                        .child(
                            div()
                                .child("Some cleanup information is unavailable.")
                                .children(data.errors.iter().map(|message| {
                                    div().mt(px(4.)).t_code_sm().child(message.clone())
                                })),
                        ),
                );
            }
            let inventory = &data.worktree_inventory;
            let filtered = worktrees::filter(
                &inventory.worktrees,
                self.project_id,
                self.show_all_projects,
                self.managed_only,
            );
            let (shown_count, shown_bytes) = worktrees::usage(&filtered);
            // Only what Git will really remove: a locked registration is
            // stale but unprunable, and counting it would leave a button that
            // never goes away.
            let prunable_count = inventory
                .worktrees
                .iter()
                .filter(|worktree| worktrees::is_prunable(worktree))
                .count();
            let all_projects_toggle = this.clone();
            let managed_toggle = this.clone();
            let prune_this = this.clone();
            let mut inventory_section = div()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(div().t_body_strong().child("Git worktrees"))
                        .child(div().flex_1())
                        .child(ds::option_toggle(
                            SharedString::from("cleanup-all-projects"),
                            "All projects",
                            None,
                            self.show_all_projects,
                            false,
                            &p,
                            move |enabled, _, cx| {
                                let _ = all_projects_toggle.update(cx, |this, cx| {
                                    this.show_all_projects = enabled;
                                    cx.notify();
                                });
                            },
                        ))
                        .child(ds::option_toggle(
                            SharedString::from("cleanup-managed-only"),
                            "Managed only",
                            None,
                            self.managed_only,
                            false,
                            &p,
                            move |enabled, _, cx| {
                                let _ = managed_toggle.update(cx, |this, cx| {
                                    this.managed_only = enabled;
                                    cx.notify();
                                });
                            },
                        ))
                        .when(prunable_count > 0, |el| {
                            el.child(
                                Button::new("cleanup-prune-worktrees")
                                    .ghost()
                                    .small()
                                    .icon("trash-2")
                                    .label(if pruning { "Pruning…" } else { "Prune stale metadata" })
                                    .disabled(busy || self.loading || self.confirming)
                                    .tooltip(
                                        "Remove stale Git registrations; no files are deleted. \
                                         Locked ones need git worktree unlock first.",
                                    )
                                    .on_click(move |_, _, cx| {
                                        let _ = prune_this
                                            .update(cx, |this, cx| this.prune(cx));
                                    }),
                            )
                        }),
                )
                .child(
                    div()
                        .mt(px(4.))
                        .t_label()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(p.ink_muted)
                        .child(
                            "All Git checkouts known to registered repositories. Sizes are allocated \
                             disk blocks, including ignored files; a repository's own checkout is \
                             not measured, since its worktrees are counted on their own rows. \
                             Unmanaged worktrees are never deleted by Panoptes; managed lanes can be \
                             archived above.",
                        ),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .t_small()
                        .text_color(p.ink_muted)
                        .child(format!("Shown: {}", usage_label(shown_count, shown_bytes))),
                );
            if !inventory.errors.is_empty() {
                inventory_section = inventory_section.child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .text_color(p.error)
                        .child(icon("circle-alert", 16.).mt(px(2.)).flex_none())
                        .child(
                            div()
                                .child("Some worktree information is unavailable.")
                                .children(inventory.errors.iter().map(|message| {
                                    div().mt(px(4.)).t_code_sm().child(message.clone())
                                })),
                        ),
                );
            }
            let rows = if filtered.is_empty() {
                div()
                    .py(px(20.))
                    .text_center()
                    .text_color(p.ink_muted)
                    .child("No worktrees match this filter.")
            } else {
                div().mt(px(12.)).flex().flex_col().gap(px(8.)).children(
                    filtered.into_iter().enumerate().map(|(index, worktree)| {
                        let path = worktree.path.clone();
                        let copy_path = path.clone();
                        let reveal_path = path.clone();
                        let reveal_store = self.store.clone();
                        let exists = worktree.exists;
                        let status = worktrees::label(worktree);
                        let status_color = match worktrees::tone(worktree) {
                            WorktreeTone::Managed => p.done,
                            WorktreeTone::Archived => p.ink_faint,
                            WorktreeTone::Locked => p.waiting,
                            WorktreeTone::Stale => p.error,
                            WorktreeTone::Neutral => p.ink_muted,
                        };
                        div()
                            .rounded(px(RADIUS_MD))
                            .border_1()
                            .border_color(p.line)
                            .px(px(12.))
                            .py(px(8.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .t_code_sm()
                                            .text_color(p.ink)
                                            .child(path),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "worktree-copy-{index}"
                                        )))
                                        .ghost()
                                        .small()
                                        .icon("copy")
                                        .tooltip("Copy path")
                                        .on_click(
                                            move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy_path.clone(),
                                                ));
                                            },
                                        ),
                                    )
                                    .when(exists, move |el| {
                                        el.child(
                                            Button::new(SharedString::from(format!(
                                                "worktree-reveal-{index}"
                                            )))
                                            .ghost()
                                            .small()
                                            .icon("folder-open")
                                            .tooltip("Reveal in Finder")
                                            .on_click(move |_, _, cx| {
                                                reveal_store
                                                    .update(cx, |store, cx| {
                                                        store.reveal_in_finder(
                                                            reveal_path.clone(),
                                                            cx,
                                                        )
                                                    })
                                                    .detach();
                                            }),
                                        )
                                    })
                                    .child(
                                        div()
                                            .t_code_sm()
                                            .text_color(p.ink_muted)
                                            .child(worktrees::size(worktree)),
                                    ),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(div().t_label().text_color(status_color).child(status))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .t_small()
                                            .text_color(p.ink_muted)
                                            .child(worktrees::detail(worktree)),
                                    ),
                            )
                    }),
                )
            };
            body = body.child(inventory_section.child(rows));
            if let Some(outcome) = &self.outcome {
                body =
                    body.child(
                        div()
                            .when(outcome.archived > 0, |el| {
                                el.child(div().text_color(p.done).child(format!(
                                    "Archived {} {}.",
                                    outcome.archived,
                                    if outcome.archived == 1 {
                                        "lane"
                                    } else {
                                        "lanes"
                                    }
                                )))
                            })
                            .children(outcome.failed.iter().map(|text| {
                                div().mt(px(4.)).text_color(p.error).child(text.clone())
                            })),
                    );
            }
            let now = now_ms();
            let disabled = busy || self.confirming;
            let list = if data.candidates.is_empty() {
                div()
                    .py(px(20.))
                    .text_center()
                    .text_color(p.ink_muted)
                    .child("No safe idle lanes to archive.")
            } else {
                div().mt(px(12.)).flex().flex_col().gap(px(8.)).children(
                    data.candidates.iter().map(|candidate| {
                        let id = candidate.lane.id;
                        let size = candidate
                            .bytes
                            .map_or("Size unknown".to_string(), format_bytes);
                        let preservation = match candidate.preservation {
                            ArchivePreservation::Checkpoint => {
                                "Uncommitted changes will be checkpointed"
                            }
                            ArchivePreservation::Clean => "Clean worktree",
                        };
                        let hint = format!(
                            "{size} \u{b7} {} \u{b7} {preservation}",
                            idle_label(candidate.last_activity, now)
                        );
                        let toggle = this.clone();
                        div()
                            .rounded(px(RADIUS_MD))
                            .border_1()
                            .border_color(p.line)
                            .px(px(12.))
                            .py(px(8.))
                            .child(ds::checkbox(
                                SharedString::from(format!("cleanup-{id}")),
                                self.selected.contains(&id),
                                candidate.lane.task.clone(),
                                Some(hint.into()),
                                disabled,
                                &p,
                                move |checked, _, cx| {
                                    let _ =
                                        toggle.update(cx, |this, cx| this.toggle(id, checked, cx));
                                },
                            ))
                    }),
                )
            };
            body = body.child(
                div()
                    .child(
                        div()
                            .t_body_strong()
                            .child(format!("Idle for {} or more days", data.idle_days)),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .t_label()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_muted)
                            .child(
                                "Only lanes with no open sessions that can be archived without \
                                 branch recovery are listed. Nothing is selected by default.",
                            ),
                    )
                    .child(list),
            );
            if self.confirming {
                let count = self.selected.len();
                body = body.child(
                    div()
                        .rounded(px(RADIUS_MD))
                        .border_1()
                        .border_color(p.line)
                        .bg(p.surface_sunken)
                        .px(px(12.))
                        .py(px(8.))
                        .child(div().t_body_strong().child(format!(
                            "Archive {count} {}?",
                            if count == 1 { "lane" } else { "lanes" }
                        )))
                        .child(div().mt(px(4.)).text_color(p.ink_muted).child(
                            "Their worktrees will be removed. Branches and conversation history \
                             are kept, and archived lanes can be restored from the sidebar. Any \
                             uncommitted changes will be preserved as Panoptes checkpoints.",
                        )),
                );
            }
        }

        let count = self.selected.len();
        let label = if busy {
            "Archiving\u{2026}".to_string()
        } else if self.confirming {
            format!("Confirm archive ({count})")
        } else if count > 0 {
            format!("Archive selected ({count})")
        } else {
            "Archive selected".to_string()
        };
        let (refresh, back, primary) = (this.clone(), this.clone(), this.clone());
        let footer = ds::overlay_footer(&p)
            .child(
                Button::new("cleanup-refresh")
                    .ghost()
                    .icon("refresh-cw")
                    .label("Refresh")
                    .disabled(busy || self.loading || self.confirming)
                    .on_click(move |_, _, cx| {
                        let _ = refresh.update(cx, |this, cx| {
                            this.outcome = None;
                            this.load(cx);
                        });
                    }),
            )
            .when(self.confirming, |el| {
                el.child(
                    Button::new("cleanup-back")
                        .label("Back")
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            let _ = back.update(cx, |this, cx| {
                                this.confirming = false;
                                cx.notify();
                            });
                        }),
                )
            })
            .child(
                div().ml_auto().child(
                    Button::new("cleanup-archive")
                        .primary()
                        .label(label)
                        .disabled(busy || self.loading || count == 0)
                        .on_click(move |_, _, cx| {
                            let _ = primary.update(cx, |this, cx| this.primary(cx));
                        }),
                ),
            );

        let panel = ds::overlay_panel(620., window, &p)
            .track_focus(&self.focus)
            .child(ds::overlay_header(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().t_heading().child(format!(
                        "Clean up {}",
                        name.unwrap_or_else(|| "lanes".into())
                    )))
                    .when(self.loading && self.data.is_some(), |el| {
                        el.child(
                            div()
                                .t_label()
                                .text_color(p.ink_muted)
                                .child("Refreshing\u{2026}"),
                        )
                    }),
                on_header_close,
            ))
            .child(
                div()
                    .id("cleanup-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(16.))
                    .child(body),
            )
            .child(footer);
        ds::modal_layer(window, on_layer_close, panel, &p)
    }
}
