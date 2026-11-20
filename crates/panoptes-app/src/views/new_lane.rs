//! The project + flow: one prompt creates a lane and its first session. The
//! prompt names the lane and becomes its first message. Port of
//! `NewLaneDialog.tsx`.

use std::rc::Rc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Enter, InputEvent, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ds::{self, Button, Type as _, icon};
use crate::model::composer::continue_list;
use crate::model::picker::{
    DEFAULT_CHOICE, borrowed_models, model_groups, profile_groups, short_model_id, thinking_groups,
};
use crate::store::{AppStore, SessionOptions};
use crate::theme::PaletteExt as _;
use crate::ui::Ui;
use crate::views::picker::{Picker, PickerEvent, PickerState};

pub struct NewLane {
    project_id: i64,
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    task: Entity<TextareaState>,
    main_checkout: bool,
    model: String,
    thinking: String,
    profile_id: Option<i64>,
    instructions: String,
    busy: bool,
    models_failed: bool,
    profile_picker: Entity<PickerState>,
    model_picker: Entity<PickerState>,
    thinking_picker: Entity<PickerState>,
}

impl NewLane {
    pub fn new(
        project_id: i64,
        prompt: Option<String>,
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("What should Pi do? e.g. Fix login redirects")
                .auto_grow(1, 10);
            if let Some(prompt) = &prompt {
                state.set_value(prompt.clone(), window, cx);
            }
            state
        });
        cx.subscribe(&task, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let focus = task.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let picker = |label: &str, window: &mut Window, cx: &mut Context<Self>| {
            let focus = focus.clone();
            cx.new(|cx| PickerState::new(label, window, cx).return_focus(focus))
        };
        let profile_picker = picker("Profile", window, cx);
        let model_picker = picker("Model", window, cx);
        let thinking_picker = picker("Thinking", window, cx);
        cx.subscribe(&profile_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(id) = event {
                this.choose_profile(id, cx);
            }
        })
        .detach();
        cx.subscribe(&model_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(id) = event {
                this.model = id.clone();
                // The borrowed levels belong to the previously active model.
                this.thinking = DEFAULT_CHOICE.into();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&thinking_picker, |this, _, event, cx| {
            if let PickerEvent::Selected(id) = event {
                this.thinking = id.clone();
                cx.notify();
            }
        })
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();

        // Model and thinking choices are read from an open session: listing
        // them needs a harness process. Without one, the harness default is
        // the only honest choice.
        let state = &store.read(cx).state;
        let active = state.active_key.clone();
        if borrowed_models(&state.models, active.as_deref()).is_none()
            && let Some(key) = active
        {
            let loaded = store.update(cx, |store, cx| store.load_models(key, cx));
            cx.spawn(async move |this, cx| {
                let ok = loaded.await == Some(true);
                let _ = this.update(cx, |this, cx| {
                    this.models_failed = !ok;
                    cx.notify();
                });
            })
            .detach();
        }

        Self {
            project_id,
            store,
            ui,
            task,
            main_checkout: false,
            model: DEFAULT_CHOICE.into(),
            thinking: DEFAULT_CHOICE.into(),
            profile_id: None,
            instructions: String::new(),
            busy: false,
            models_failed: false,
            profile_picker,
            model_picker,
            thinking_picker,
        }
    }

    fn choose_profile(&mut self, id: &str, cx: &mut Context<Self>) {
        let profile = id.parse::<i64>().ok().and_then(|id| {
            self.store
                .read(cx)
                .state
                .preferences
                .profiles
                .iter()
                .find(|p| p.id == id)
                .cloned()
        });
        self.profile_id = profile.as_ref().map(|p| p.id);
        self.model = profile
            .as_ref()
            .and_then(|p| p.model_id.clone())
            .unwrap_or_default();
        self.thinking = profile
            .as_ref()
            .and_then(|p| p.thinking_level.clone())
            .unwrap_or_default();
        self.instructions = profile.and_then(|p| p.instructions).unwrap_or_default();
        cx.notify();
    }

    fn text(&self, cx: &App) -> String {
        self.task.read(cx).value().trim().to_string()
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if text.is_empty() || self.busy {
            return;
        }
        self.busy = true;
        cx.notify();
        let options = SessionOptions {
            model: Some(self.model.clone()).filter(|m| !m.is_empty()),
            thinking: Some(self.thinking.clone()).filter(|t| !t.is_empty()),
            instructions: Some(self.instructions.clone()).filter(|i| !i.is_empty()),
            prompt: None,
        };
        let (project_id, main_checkout) = (self.project_id, self.main_checkout);
        let created = self.store.update(cx, |store, cx| {
            store.create_from_prompt(project_id, text, main_checkout, options, cx)
        });
        cx.spawn(async move |this, cx| {
            let created = created.await;
            let _ = this.update(cx, |this, cx| {
                if created {
                    this.ui.update(cx, |ui, cx| ui.close(cx));
                } else {
                    this.busy = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Closes the panel, stopping a lane that is still being made.
    fn cancel(&mut self, cx: &mut App) -> bool {
        if [
            &self.profile_picker,
            &self.model_picker,
            &self.thinking_picker,
        ]
        .iter()
        .any(|picker| picker.read(cx).is_open())
        {
            return false;
        }
        if self.busy {
            let project_id = self.project_id;
            self.store
                .update(cx, |store, _| store.cancel_lane_creation(project_id));
        }
        self.ui.update(cx, |ui, cx| ui.close(cx));
        true
    }
}

impl Render for NewLane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let this = cx.entity().downgrade();
        let on_layer_close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.cancel(cx)).unwrap_or(false)
            }
        };
        let on_header_close: ds::CloseHandler = {
            let this = this.clone();
            Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| this.cancel(cx));
            })
        };
        let state = &self.store.read(cx).state;
        let Some(project) = state
            .projects
            .iter()
            .find(|p| p.id == self.project_id)
            .cloned()
        else {
            let store = self.store.clone();
            let panel = ds::overlay_panel(480., window, &p)
                .child(ds::overlay_header(
                    div().t_heading().child("New lane"),
                    Some(on_header_close),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.))
                        .px(px(24.))
                        .pt(px(16.))
                        .pb(px(32.))
                        .text_center()
                        .child(icon("folder-open", 24.).text_color(p.ink_muted))
                        .child(
                            div()
                                .max_w(px(320.))
                                .text_color(p.ink_muted)
                                .child("Lanes belong to projects. Add a git repository to begin."),
                        )
                        .child(
                            Button::new("add-project")
                                .primary()
                                .label("Add a project")
                                .on_click(move |_, _, cx| {
                                    store.update(cx, |store, cx| store.pick_project(cx))
                                }),
                        ),
                );
            return ds::modal_layer(window, on_layer_close, panel, &p).into_any_element();
        };

        let busy = self.busy;
        let models = borrowed_models(&state.models, state.active_key.as_deref()).cloned();
        let levels = state
            .active_key
            .as_ref()
            .and_then(|key| state.settings.get(key))
            .filter(|s| !s.thinking_levels.is_empty())
            .or_else(|| {
                state
                    .settings
                    .values()
                    .find(|s| !s.thinking_levels.is_empty())
            })
            .map(|s| s.thinking_levels.clone())
            .unwrap_or_default();
        let preferences = state.preferences.clone();
        let has_active = state.active_key.is_some();
        let chosen_model = models
            .as_ref()
            .and_then(|m| m.iter().find(|m| m.id == self.model).cloned());
        let chosen_profile = preferences
            .profiles
            .iter()
            .find(|p| Some(p.id) == self.profile_id)
            .map(|p| p.name.clone());
        let empty = self.text(cx).is_empty();

        let help = if self.main_checkout {
            "Shares the project's own checkout: no worktree, copied files or setup command. \
             Archiving the lane never removes it."
                .to_string()
        } else {
            format!(
                "Gets its own worktree and branch from {}. Your prompt starts the lane's first session.",
                project.settings.base_branch
            )
        };

        let model_title = if models.is_some() {
            None
        } else if self.models_failed {
            Some("Could not load models. Close and reopen this panel to retry.")
        } else if has_active {
            Some("Loading models")
        } else {
            Some(
                "Models are listed from an open session; switch in the composer once this one runs",
            )
        };
        let mut model_picker = Picker::new(
            "new-lane-model",
            &self.model_picker,
            models
                .as_ref()
                .map(|m| model_groups(m, &self.model, Some(&preferences)))
                .unwrap_or_default(),
            match &chosen_model {
                Some(m) => div()
                    .t_code_sm()
                    .child(short_model_id(&m.id, &m.provider))
                    .into_any_element(),
                None => div().child("Default model").into_any_element(),
            },
            &p,
        )
        .icon("agent")
        .empty_text(if self.models_failed {
            "Could not load models"
        } else {
            "Loading models\u{2026}"
        })
        .disabled(busy || models.is_none());
        if let Some(title) = model_title {
            model_picker = model_picker.title(title);
        }

        let toggle = this.clone();
        let on_enter = this.clone();
        let task_focus = self.task.read(cx).focus_handle(cx);
        let footer = ds::overlay_footer(&p)
            .child(ds::option_toggle(
                "main-checkout",
                "Use the main checkout",
                Some("Work in the project's own checkout instead of a new worktree".into()),
                self.main_checkout,
                busy,
                &p,
                move |pressed, _, cx| {
                    let _ = toggle.update(cx, |this, cx| {
                        this.main_checkout = pressed;
                        cx.notify();
                    });
                },
            ))
            .child(
                Picker::new(
                    "new-lane-profile",
                    &self.profile_picker,
                    profile_groups(&preferences.profiles, self.profile_id),
                    chosen_profile.unwrap_or_else(|| "No profile".into()),
                    &p,
                )
                .icon("agent")
                .title(
                    "Apply saved model, thinking and additional instructions to this new session",
                )
                .searchable(preferences.profiles.len() > 6)
                .disabled(busy),
            )
            .child(model_picker)
            .when(!levels.is_empty(), |el| {
                el.child(
                    Picker::new(
                        "new-lane-thinking",
                        &self.thinking_picker,
                        thinking_groups(&levels, &self.thinking),
                        if self.thinking.is_empty() {
                            "Default thinking".to_string()
                        } else {
                            format!("Thinking: {}", self.thinking)
                        },
                        &p,
                    )
                    .icon("circle-dot")
                    .searchable(false)
                    .width(200.)
                    .disabled(busy),
                )
            })
            .child({
                let submit = this.clone();
                let button = Button::new("create-lane")
                    .primary()
                    .label(if busy {
                        "Creating lane\u{2026}"
                    } else {
                        "Create lane"
                    })
                    .disabled(busy || empty)
                    .on_click(move |_, _, cx| {
                        let _ = submit.update(cx, |this, cx| this.submit(cx));
                    });
                div().ml_auto().child(if busy {
                    button
                } else {
                    button.kbd(vec!["↵"])
                })
            });

        let panel = ds::overlay_panel(560., window, &p)
            .capture_action(move |action: &Enter, window, cx| {
                if !task_focus.is_focused(window) {
                    return;
                }
                let _ = on_enter.update(cx, |this, cx| {
                    if action.shift {
                        let text = this.task.read(cx).value().to_string();
                        let range = this.task.read(cx).selected_range();
                        if let Some(next) = continue_list(&text, range.start, range.end) {
                            cx.stop_propagation();
                            this.task.update(cx, |task, cx| {
                                task.set_value(next.text, window, cx);
                                task.set_selected_range(next.caret..next.caret, cx);
                            });
                        }
                        return;
                    }
                    cx.stop_propagation();
                    this.submit(cx);
                });
            })
            .child(ds::overlay_header(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .min_w_0()
                            .t_label()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_muted)
                            .child(icon("folder", 14.).flex_none())
                            .child(div().truncate().child(project.settings.name.clone())),
                    )
                    .child(
                        icon("chevron-right", 12.)
                            .flex_none()
                            .text_color(p.ink_faint),
                    )
                    .child(div().t_body_strong().child("New lane")),
                Some(on_header_close),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .pt(px(4.))
                    .pb(px(16.))
                    .child(
                        Textarea::new(&self.task)
                            .appearance(false)
                            .disabled(busy)
                            .xsmall()
                            .t_heading()
                            .font_weight(if empty {
                                FontWeight::NORMAL
                            } else {
                                FontWeight::MEDIUM
                            })
                            .mx(px(-4.))
                            .p(px(0.)),
                    )
                    .child(
                        div()
                            .t_label()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_muted)
                            .child(help),
                    ),
            )
            .child(footer);
        ds::modal_layer(window, on_layer_close, panel, &p).into_any_element()
    }
}
