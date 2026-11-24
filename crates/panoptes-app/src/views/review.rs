//! The workspace's read-only lane review. Data and transitions live in the
//! store and `model::review`; this module only renders them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::changes::{FileChange, FileStatus, Hunk as CoreHunk, LaneChanges};
use panoptes_core::store::{Lane, LaneState};

use crate::ds::{Button, Type as _, icon};
use crate::model::format::plural;
use crate::model::review::{BaseNotice, BaseNoticeKind, Hunks, ReviewFileId, base_notices};
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::{Ui, toggle_review};
use crate::zoom::px;

pub const CONTEXT: &str = "Review";

actions!(
    panoptes,
    [
        CloseReview,
        NextReviewFile,
        PreviousReviewFile,
        NextReviewHunk,
        PreviousReviewHunk
    ]
);

pub fn init(cx: &mut App) {
    let at = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", CloseReview, at),
        KeyBinding::new("j", NextReviewFile, at),
        KeyBinding::new("k", PreviousReviewFile, at),
        KeyBinding::new("n", NextReviewHunk, at),
        KeyBinding::new("p", PreviousReviewHunk, at),
    ]);
}

pub struct ReviewPane {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    files_scroll: ScrollHandle,
    target: Option<(ReviewFileId, usize)>,
    /// Where each expanded file's hunks sat in the last layout, in window
    /// space. The hunk selected by `n`/`p` needs its own position: the
    /// sections scroll container only knows where whole files are.
    hunk_tops: Rc<RefCell<HashMap<ReviewFileId, Vec<Pixels>>>>,
}

impl ReviewPane {
    pub fn new(store: Entity<AppStore>, ui: Entity<Ui>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.observe(&ui, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            ui,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            files_scroll: ScrollHandle::new(),
            target: None,
            hunk_tops: Rc::default(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn close(&self, lane_id: Option<i64>, cx: &mut App) {
        toggle_review(&self.store, &self.ui, lane_id, cx);
    }

    fn header(
        &self,
        lane: Option<&Lane>,
        review: Option<&LaneChanges>,
        loading: bool,
        narrow: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lane_id = lane.map(|lane| lane.id);
        let scope = match review {
            Some(review) if review.worktrees.len() == 1 => {
                format!("Lane vs {}", review.worktrees[0].base_branch)
            }
            Some(review) => format!("Lane · {} repositories", review.worktrees.len()),
            None => "Lane".to_string(),
        };
        let refresh_store = self.store.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .min_h(px(48.))
            .px(px(12.))
            .border_b_1()
            .border_color(p.line)
            .child(icon("file-diff", 16.).text_color(p.ink_muted))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div().truncate().t_body_strong().child(
                            lane.map(|lane| lane.task.clone())
                                .unwrap_or_else(|| "Review".into()),
                        ),
                    )
                    .child(
                        div()
                            .truncate()
                            .t_caption()
                            .text_color(p.ink_muted)
                            .child(scope),
                    ),
            )
            .when_some(review, |el, review| {
                el.child(change_totals(review, p, true))
            })
            .child(if narrow {
                Button::new("back-to-chat")
                    .ghost()
                    .small()
                    .glyph(icon("arrow-up", 14.).rotate(-90.))
                    .label("Back")
                    .tooltip("Back to conversation")
                    .on_click(cx.listener(move |this, _, _, cx| this.close(lane_id, cx)))
            } else {
                Button::new("refresh-review")
                    .ghost()
                    .small()
                    .icon("refresh-cw")
                    .disabled(loading || lane_id.is_none())
                    .tooltip("Refresh changes")
                    .on_click(move |_, _, cx| {
                        if let Some(lane_id) = lane_id {
                            refresh_store
                                .update(cx, |store, cx| store.refresh_review(lane_id, cx).detach());
                        }
                    })
            })
            .when(!narrow, |el| {
                el.child(
                    Button::new("close-review")
                        .ghost()
                        .small()
                        .icon("x")
                        .tooltip("Close review (Esc)")
                        .on_click(cx.listener(move |this, _, _, cx| this.close(lane_id, cx))),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn content(
        &mut self,
        lane: &Lane,
        review: &LaneChanges,
        loading: bool,
        error: Option<&str>,
        selected: Option<&ReviewFileId>,
        expanded: &HashSet<ReviewFileId>,
        hunk: usize,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if review.totals.files == 0 {
            // The last good listing may be empty while this refresh failed: a
            // silent "No changes" would hide the error.
            return div()
                .relative()
                .flex()
                .flex_1()
                .child(centered_message(
                    "file-diff",
                    if loading {
                        "Checking for changes…"
                    } else {
                        "No changes"
                    },
                    "The lane's worktree matches its comparison base.",
                    p,
                ))
                .children(error.map(|error| error_banner(error, p)))
                .into_any_element();
        }

        let multiple = review.worktrees.len() > 1;
        let mut file_rows = HashMap::new();
        let mut section_rows = HashMap::new();
        let mut files = Vec::new();
        let mut sections = Vec::new();
        for worktree in &review.worktrees {
            if multiple {
                files.push(
                    div()
                        .pt(px(8.))
                        .pb(px(4.))
                        .t_caption()
                        .text_color(p.ink_muted)
                        .child(worktree.project_name.to_uppercase())
                        .into_any_element(),
                );
                sections.push(
                    div()
                        .py(px(6.))
                        .px(px(12.))
                        .t_label()
                        .text_color(p.ink_muted)
                        .child(worktree.project_name.clone())
                        .into_any_element(),
                );
            }
            for file in &worktree.changes.files {
                let id = ReviewFileId {
                    lane_id: lane.id,
                    project_id: worktree.project_id,
                    path: file.path.clone(),
                };
                file_rows.insert(id.clone(), files.len());
                files.push(self.file_row(&id, file, selected == Some(&id), p));
                section_rows.insert(id.clone(), sections.len());
                let state = self
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .get(&lane.id)
                    .and_then(|state| state.hunks(&id).cloned());
                sections.push(self.file_section(
                    lane,
                    &id,
                    file,
                    expanded.contains(&id),
                    state,
                    selected == Some(&id),
                    hunk,
                    p,
                ));
            }
        }

        if let Some(id) = selected {
            let target = (id.clone(), hunk);
            if self.target.as_ref() != Some(&target) {
                let hunk_moved = self
                    .target
                    .as_ref()
                    .is_some_and(|(previous, _)| previous == id && hunk > 0);
                if hunk_moved {
                    self.scroll_to_hunk(id, hunk);
                } else {
                    // A new file: show its first hunk from the header down,
                    // and its row in the file list.
                    if let Some(row) = section_rows.get(id) {
                        self.scroll.scroll_to_top_of_item(*row);
                    }
                    if let Some(row) = file_rows.get(id) {
                        self.files_scroll.scroll_to_item(*row);
                    }
                }
                self.target = Some(target);
            }
        } else {
            self.target = None;
        }

        div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .children(error.map(|error| error_banner(error, p)))
            .child(
                div()
                    .id("review-files")
                    .w(px(190.))
                    .flex_none()
                    .overflow_y_scroll()
                    .track_scroll(&self.files_scroll)
                    .border_r_1()
                    .border_color(p.line)
                    .bg(p.surface)
                    .p(px(6.))
                    .children(files),
            )
            .child(
                div()
                    .id("review-sections")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(12.))
                    .children(sections),
            )
            .when(loading, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(8.))
                        .right(px(12.))
                        .rounded(px(4.))
                        .bg(p.surface_raised)
                        .px(px(8.))
                        .py(px(3.))
                        .t_caption()
                        .text_color(p.ink_muted)
                        .child("Refreshing…"),
                )
            })
            .into_any_element()
    }

    /// Brings the selected hunk to the top of the sections view. Its position
    /// comes from the last layout; until one has happened there is nothing to
    /// aim at.
    fn scroll_to_hunk(&self, id: &ReviewFileId, hunk: usize) {
        let Some(top) = self
            .hunk_tops
            .borrow()
            .get(id)
            .and_then(|tops| tops.get(hunk).copied())
        else {
            return;
        };
        let bounds = self.scroll.bounds();
        let offset = (bounds.top() - top)
            .min(Pixels::ZERO)
            .max(-self.scroll.max_offset().y);
        self.scroll.set_offset(point(Pixels::ZERO, offset));
    }

    fn file_row(
        &self,
        id: &ReviewFileId,
        file: &FileChange,
        selected: bool,
        p: &Palette,
    ) -> AnyElement {
        let store = self.store.clone();
        let id = id.clone();
        div()
            .id(SharedString::from(format!(
                "review-file-{}-{}-{}",
                id.lane_id, id.project_id, id.path
            )))
            .flex()
            .items_center()
            .gap(px(6.))
            .min_h(px(32.))
            .px(px(6.))
            .rounded(px(4.))
            .cursor_pointer()
            .when(selected, |el| el.bg(p.selection))
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.select_review_file(id.clone(), cx))
            })
            .child(status_label(file.status, p))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .t_small()
                    .child(file.path.clone()),
            )
            .child(
                div()
                    .t_code_sm()
                    .text_color(p.done)
                    .child(format!("+{}", file.added)),
            )
            .child(
                div()
                    .t_code_sm()
                    .text_color(p.error)
                    .child(format!("−{}", file.removed)),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn file_section(
        &self,
        lane: &Lane,
        id: &ReviewFileId,
        file: &FileChange,
        expanded: bool,
        hunks: Option<Hunks>,
        selected: bool,
        selected_hunk: usize,
        p: &Palette,
    ) -> AnyElement {
        let store = self.store.clone();
        let toggle_id = id.clone();
        let full_path = lane
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == id.project_id)
            .map(|worktree| Path::new(&worktree.path).join(&id.path));
        let can_open = lane.state == LaneState::Active && file.status != FileStatus::Deleted;
        let mut header =
            div()
                .id(SharedString::from(format!(
                    "review-section-{}-{}-{}",
                    id.lane_id, id.project_id, id.path
                )))
                .flex()
                .items_center()
                .gap(px(6.))
                .min_h(px(32.))
                .px(px(8.))
                .cursor_pointer()
                .border_b_1()
                .border_color(p.line)
                .when(selected, |el| el.bg(p.selection))
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.toggle_review_file(toggle_id.clone(), cx)
                    })
                })
                .child(icon(
                    if expanded {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    12.,
                ))
                .child(status_label(file.status, p))
                .child(div().flex_1().min_w_0().truncate().t_code_sm().child(
                    match &file.old_path {
                        Some(old) => format!("{old} → {}", file.path),
                        None => file.path.clone(),
                    },
                ))
                .child(
                    div()
                        .t_code_sm()
                        .text_color(p.done)
                        .child(format!("+{}", file.added)),
                )
                .child(
                    div()
                        .t_code_sm()
                        .text_color(p.error)
                        .child(format!("−{}", file.removed)),
                );
        if let Some(path) = full_path {
            let copy = path.clone();
            let copy_store = self.store.clone();
            header = header
                .child(
                    Button::new(SharedString::from(format!(
                        "copy-review-path-{}-{}-{}",
                        id.lane_id, id.project_id, id.path
                    )))
                    .ghost()
                    .small()
                    .icon("copy")
                    .tooltip("Copy full path")
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            path.to_string_lossy().into_owned(),
                        ));
                    }),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "open-review-file-{}-{}-{}",
                        id.lane_id, id.project_id, id.path
                    )))
                    .ghost()
                    .small()
                    .icon("external-link")
                    .tooltip("Open in editor")
                    .disabled(!can_open)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        copy_store.update(cx, |store, cx| store.open_path(copy.clone(), cx));
                    }),
                );
        }

        let mut section = div()
            .flex()
            .flex_none()
            .flex_col()
            .mb(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(p.line)
            .bg(p.surface_sunken)
            .overflow_hidden()
            .child(header);
        if expanded {
            section = section.child(match hunks {
                _ if file.binary => notice("Binary file; its line counts are not available.", p),
                _ if file.too_large => notice(
                    "Large diff. Hunks are omitted by the backend's size limit.",
                    p,
                ),
                None | Some(Hunks::Loading) => notice("Loading hunks…", p),
                Some(Hunks::Failed(error)) => {
                    let store = self.store.clone();
                    let id = id.clone();
                    div()
                        .p(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().t_small().text_color(p.error).child(error))
                        .child(
                            Button::new(SharedString::from(format!(
                                "retry-hunks-{}-{}-{}",
                                id.lane_id, id.project_id, id.path
                            )))
                            .small()
                            .label("Try again")
                            .on_click(move |_, _, cx| {
                                store.update(cx, |store, cx| {
                                    store.retry_review_file(id.clone(), cx)
                                });
                            }),
                        )
                        .into_any_element()
                }
                Some(Hunks::Ready(hunks)) if hunks.is_empty() => {
                    notice("No textual hunks in this file.", p)
                }
                Some(Hunks::Ready(hunks)) => {
                    let tops = self.hunk_tops.clone();
                    let key = id.clone();
                    hunks_view(
                        &hunks,
                        if selected { selected_hunk } else { usize::MAX },
                        p,
                        move |found| {
                            tops.borrow_mut().insert(key.clone(), found);
                        },
                    )
                }
            });
        }
        section.into_any_element()
    }
}

impl Render for ReviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let narrow = window.viewport_size().width < px(1100.);
        let (lane, data, loading, error, selected, expanded, hunk) = {
            let state = &self.store.read(cx).state;
            let lane_id = state
                .reviews
                .values()
                .find(|review| review.visible)
                .map(|review| review.lane_id);
            let lane = lane_id.and_then(|id| state.lanes.iter().find(|lane| lane.id == id));
            let review = lane_id.and_then(|id| state.reviews.get(&id));
            (
                lane.cloned(),
                review.and_then(|review| review.data.clone()),
                review.is_some_and(|review| review.loading),
                review.and_then(|review| review.error.clone()),
                review.and_then(|review| review.selected.clone()),
                review
                    .map(|review| review.expanded.clone())
                    .unwrap_or_default(),
                review.map_or(0, |review| review.hunk),
            )
        };

        let body = match (lane.as_ref(), data.as_ref()) {
            (None, _) => centered_message(
                "file-diff",
                "Select a lane to review",
                "Review is available for sessions attached to a lane.",
                &p,
            ),
            (_, None) if loading => centered_message(
                "loader-circle",
                "Reading lane changes…",
                "The tree comparison includes shell edits and untracked files.",
                &p,
            ),
            (_, None) => centered_message(
                "circle-alert",
                "Could not read changes",
                error.as_deref().unwrap_or("The lane review did not load."),
                &p,
            ),
            (Some(lane), Some(data)) => self.content(
                lane,
                data,
                loading,
                error.as_deref(),
                selected.as_ref(),
                &expanded,
                hunk,
                &p,
                cx,
            ),
        };

        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseReview, _, cx| {
                let lane_id = this
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .values()
                    .find(|review| review.visible)
                    .map(|review| review.lane_id);
                this.close(lane_id, cx);
            }))
            .on_action(cx.listener(|this, _: &NextReviewFile, _, cx| {
                if let Some(lane_id) = this
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .values()
                    .find(|review| review.visible)
                    .map(|review| review.lane_id)
                {
                    this.store
                        .update(cx, |store, cx| store.move_review_file(lane_id, 1, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousReviewFile, _, cx| {
                if let Some(lane_id) = this
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .values()
                    .find(|review| review.visible)
                    .map(|review| review.lane_id)
                {
                    this.store
                        .update(cx, |store, cx| store.move_review_file(lane_id, -1, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &NextReviewHunk, _, cx| {
                if let Some(lane_id) = this
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .values()
                    .find(|review| review.visible)
                    .map(|review| review.lane_id)
                {
                    this.store
                        .update(cx, |store, cx| store.move_review_hunk(lane_id, 1, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousReviewHunk, _, cx| {
                if let Some(lane_id) = this
                    .store
                    .read(cx)
                    .state
                    .reviews
                    .values()
                    .find(|review| review.visible)
                    .map(|review| review.lane_id)
                {
                    this.store
                        .update(cx, |store, cx| store.move_review_hunk(lane_id, -1, cx));
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(p.bg)
            .text_color(p.ink)
            .child(self.header(lane.as_ref(), data.as_deref(), loading, narrow, &p, cx))
            .children(base_notice_views(data.as_deref(), &p))
            .child(body)
    }
}

fn base_notice_views(review: Option<&LaneChanges>, p: &Palette) -> Vec<AnyElement> {
    review
        .into_iter()
        .flat_map(base_notices)
        .map(|notice| base_notice(&notice, p))
        .collect()
}

fn base_notice(notice: &BaseNotice, p: &Palette) -> AnyElement {
    let (ink, bg, icon_name) = match notice.kind {
        BaseNoticeKind::Clean => (p.done, p.done_soft, "check"),
        BaseNoticeKind::Conflict => (p.error, p.error_soft, "circle-alert"),
        BaseNoticeKind::Unavailable => (p.waiting, p.waiting_soft, "circle-alert"),
    };
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(4.))
        .border_b_1()
        .border_color(p.line)
        .bg(bg)
        .px(px(12.))
        .py(px(8.))
        .text_color(ink)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(icon_name, 14.))
                .child(div().t_small().child(notice.message.clone())),
        )
        .children(notice.paths.iter().map(|path| {
            div()
                .pl(px(22.))
                .t_code_sm()
                .child(path.clone())
                .into_any_element()
        }))
        .children(notice.detail.as_ref().map(|reason| {
            div()
                .pl(px(22.))
                .t_caption()
                .child(reason.clone())
                .into_any_element()
        }))
        .into_any_element()
}

fn centered_message(icon_name: &'static str, title: &str, detail: &str, p: &Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .p(px(24.))
        .text_center()
        .child(icon(icon_name, 24.).text_color(p.ink_muted))
        .child(div().t_body_strong().child(title.to_string()))
        .child(
            div()
                .t_small()
                .text_color(p.ink_muted)
                .child(detail.to_string()),
        )
        .into_any_element()
}

fn change_totals(review: &LaneChanges, p: &Palette, files: bool) -> AnyElement {
    let mut row = div().flex().items_center().gap(px(6.)).t_code_sm();
    if files {
        row = row.child(div().text_color(p.ink_muted).child(plural(
            review.totals.files,
            "file",
            "files",
        )));
    }
    row.child(
        div()
            .text_color(p.done)
            .child(format!("+{}", review.totals.added)),
    )
    .child(
        div()
            .text_color(p.error)
            .child(format!("−{}", review.totals.removed)),
    )
    .into_any_element()
}

fn status_label(status: FileStatus, p: &Palette) -> AnyElement {
    let (label, color) = match status {
        FileStatus::Added => ("A", p.done),
        FileStatus::Modified => ("M", p.ink_muted),
        FileStatus::Deleted => ("D", p.error),
        FileStatus::Renamed => ("R", p.link),
        FileStatus::TypeChanged => ("T", p.waiting),
    };
    div()
        .w(px(12.))
        .flex_none()
        .text_center()
        .t_code_sm()
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn notice(text: &str, p: &Palette) -> AnyElement {
    div()
        .p(px(12.))
        .t_small()
        .text_color(p.ink_muted)
        .child(text.to_string())
        .into_any_element()
}

/// Renders the hunks and reports where each one was laid out, so the pane can
/// scroll to the selected one.
fn hunks_view(
    hunks: &[CoreHunk],
    selected: usize,
    p: &Palette,
    record: impl Fn(Vec<Pixels>) + 'static,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .on_children_prepainted(move |bounds, _, _| {
            record(bounds.iter().map(|bounds| bounds.top()).collect())
        })
        .overflow_x_scrollbar()
        .children(hunks.iter().enumerate().map(|(index, hunk)| {
            let mut view = div()
                .flex()
                .flex_none()
                .flex_col()
                .when(index == selected, |el| {
                    el.border_l_2().border_color(p.focus)
                })
                .child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .t_code_sm()
                        .text_color(p.ink_muted)
                        .bg(p.surface)
                        .whitespace_nowrap()
                        .child(format!(
                            "@@ -{},{} +{},{} @@ {}",
                            hunk.old_start,
                            hunk.old_lines,
                            hunk.new_start,
                            hunk.new_lines,
                            hunk.header
                        )),
                );
            for line in &hunk.lines {
                let (bg, sign) = match line.kind {
                    panoptes_core::changes::DiffLineKind::Add => (Some(p.diff_add), "+"),
                    panoptes_core::changes::DiffLineKind::Remove => (Some(p.diff_remove), "−"),
                    panoptes_core::changes::DiffLineKind::Context => (None, " "),
                };
                view = view.child(
                    div()
                        .flex()
                        .flex_none()
                        .t_code()
                        .whitespace_nowrap()
                        .when_some(bg, |el, bg| el.bg(bg))
                        .child(line_number(line.old_line, p))
                        .child(line_number(line.new_line, p))
                        .child(div().w(px(16.)).flex_none().text_center().child(sign))
                        .child(div().pr(px(12.)).child(if line.text.is_empty() {
                            " ".into()
                        } else {
                            line.text.clone()
                        })),
                );
            }
            view.into_any_element()
        }))
        .into_any_element()
}

/// The refresh error, over whatever the pane shows.
fn error_banner(error: &str, p: &Palette) -> AnyElement {
    div()
        .absolute()
        .top(px(8.))
        .right(px(12.))
        .left(px(12.))
        .p(px(8.))
        .rounded(px(4.))
        .bg(p.error_soft)
        .text_color(p.error)
        .t_small()
        .child(error.to_string())
        .into_any_element()
}

fn line_number(number: Option<u32>, p: &Palette) -> AnyElement {
    div()
        .w(px(42.))
        .flex_none()
        .px(px(4.))
        .text_right()
        .text_color(p.ink_faint)
        .child(number.map_or_else(String::new, |number| number.to_string()))
        .into_any_element()
}
