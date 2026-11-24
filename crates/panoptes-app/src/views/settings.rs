//! Panoptes-only session presets, model-picker presentation, lane cleanup and
//! notifications.

use std::collections::BTreeSet;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use panoptes_core::store::{AgentProfile, NotificationField, ProfileInput};
use panoptes_harness::ModelInfo;

use crate::ds::{self, Button, RADIUS_MD, Tab, Type as _};
use crate::model::picker::{
    DEFAULT_CHOICE, missing_profile_model, model_groups, short_model_id, thinking_groups,
};
use crate::store::AppStore;
use crate::theme::{Palette, PaletteExt as _};
use crate::ui::Ui;
use crate::views::picker::{Picker, PickerEvent, PickerState};
use crate::zoom::{MAX_ZOOM, MIN_ZOOM, ZoomCommand, px};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Appearance,
    Profiles,
    Models,
    Lanes,
    Notifications,
    Diagnostics,
}

/// A profile being written: a new one, or an edit of a saved one.
struct Editor {
    id: Option<i64>,
    name: Entity<InputState>,
    instructions: Entity<TextareaState>,
    model: String,
    thinking: String,
    busy: bool,
    model_picker: Entity<PickerState>,
    thinking_picker: Entity<PickerState>,
}

pub struct Settings {
    store: Entity<AppStore>,
    ui: Entity<Ui>,
    tab: SettingsTab,
    editor: Option<Editor>,
    confirm_delete: Option<i64>,
    days: Entity<InputState>,
    days_busy: bool,
    asked_models: bool,
    focus: FocusHandle,
}

/// Every model list already obtained from an open harness, deduplicated by id.
fn available_models(store: &AppStore) -> Vec<ModelInfo> {
    let mut seen = BTreeSet::new();
    let mut keys: Vec<&String> = store.state.models.keys().collect();
    keys.sort();
    keys.into_iter()
        .flat_map(|key| store.state.models[key].iter())
        .filter(|m| seen.insert(m.id.clone()))
        .cloned()
        .collect()
}

fn thinking_levels(store: &AppStore) -> Vec<String> {
    store
        .state
        .settings
        .values()
        .find(|s| !s.thinking_levels.is_empty())
        .map(|s| s.thinking_levels.clone())
        .unwrap_or_default()
}

fn profile_summary(profile: &AgentProfile) -> String {
    [
        Some(
            profile
                .model_id
                .clone()
                .unwrap_or_else(|| "Harness default model".into()),
        ),
        profile
            .thinking_level
            .as_ref()
            .map(|level| format!("Thinking: {level}")),
        profile
            .instructions
            .as_ref()
            .map(|_| "Additional instructions".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" \u{b7} ")
}

/// The days field's value when it is a whole number from 1 to 365.
fn valid_days(text: &str) -> Option<u32> {
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|d| (1..=365).contains(d))
}

impl Settings {
    pub fn new(
        store: Entity<AppStore>,
        ui: Entity<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let days_value = store
            .read(cx)
            .state
            .preferences
            .cleanup_idle_days
            .to_string();
        let days = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(days_value, window, cx);
            state
        });
        cx.subscribe(&days, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&days, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_days(cx);
            }
        })
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            store,
            ui,
            tab: SettingsTab::Appearance,
            editor: None,
            confirm_delete: None,
            days,
            days_busy: false,
            asked_models: false,
            focus,
        }
    }

    fn close(&self, cx: &mut App) -> bool {
        if let Some(editor) = &self.editor
            && (editor.model_picker.read(cx).is_open() || editor.thinking_picker.read(cx).is_open())
        {
            return false;
        }
        self.ui.update(cx, |ui, cx| ui.close(cx));
        true
    }

    fn edit(&mut self, profile: Option<AgentProfile>, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Careful implementation");
            if let Some(profile) = &profile {
                state.set_value(profile.name.clone(), window, cx);
            }
            state
        });
        cx.subscribe(&name, |this, _, event, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.save_profile(cx),
            _ => {}
        })
        .detach();
        let instructions = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("Prefer small changes. Run all relevant tests.")
                .auto_grow(5, 12);
            if let Some(text) = profile.as_ref().and_then(|p| p.instructions.clone()) {
                state.set_value(text, window, cx);
            }
            state
        });
        let model_picker = cx.new(|cx| PickerState::new("Model", window, cx));
        let thinking_picker = cx.new(|cx| PickerState::new("Thinking", window, cx));
        cx.subscribe(&model_picker, |this, _, event, cx| {
            if let (PickerEvent::Selected(id), Some(editor)) = (event, this.editor.as_mut()) {
                editor.model = id.clone();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&thinking_picker, |this, _, event, cx| {
            if let (PickerEvent::Selected(id), Some(editor)) = (event, this.editor.as_mut()) {
                editor.thinking = id.clone();
                cx.notify();
            }
        })
        .detach();
        name.update(cx, |name, cx| name.focus(window, cx));
        self.editor = Some(Editor {
            id: profile.as_ref().map(|p| p.id),
            name,
            instructions,
            model: profile
                .as_ref()
                .and_then(|p| p.model_id.clone())
                .unwrap_or_else(|| DEFAULT_CHOICE.into()),
            thinking: profile
                .as_ref()
                .and_then(|p| p.thinking_level.clone())
                .unwrap_or_else(|| DEFAULT_CHOICE.into()),
            busy: false,
            model_picker,
            thinking_picker,
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let name = editor.name.read(cx).value().to_string();
        if name.trim().is_empty() || editor.busy {
            return;
        }
        editor.busy = true;
        let instructions = editor.instructions.read(cx).value().trim().to_string();
        let input = ProfileInput {
            name,
            model_id: Some(editor.model.clone()).filter(|m| !m.is_empty()),
            thinking_level: Some(editor.thinking.clone()).filter(|t| !t.is_empty()),
            instructions: Some(instructions).filter(|i| !i.is_empty()),
        };
        let id = editor.id;
        let saved = self
            .store
            .update(cx, |store, cx| store.save_profile(id, input, cx));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let ok = saved.await == Some(true);
            let _ = this.update(cx, |this, cx| {
                if ok {
                    this.editor = None;
                } else if let Some(editor) = this.editor.as_mut() {
                    editor.busy = false;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_days(&mut self, cx: &mut Context<Self>) {
        let Some(days) = valid_days(&self.days.read(cx).value()) else {
            return;
        };
        if self.days_busy {
            return;
        }
        self.days_busy = true;
        cx.notify();
        let saved = self
            .store
            .update(cx, |store, cx| store.set_cleanup_idle_days(days, cx));
        cx.spawn(async move |this, cx| {
            saved.await;
            let _ = this.update(cx, |this, cx| {
                this.days_busy = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn profiles_tab(
        &self,
        models: &[ModelInfo],
        window: &Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let preferences = self.store.read(cx).state.preferences.clone();
        let intro = div()
            .t_label()
            .font_weight(FontWeight::NORMAL)
            .text_color(p.ink_muted)
            .child(
                "Profiles set up new sessions. Empty fields keep the harness default; \
                 instructions are appended to its system prompt. They do not change harness \
                 defaults or existing sessions.",
            );
        let body = div().flex().flex_col().gap(px(12.)).child(intro);
        if let Some(editor) = &self.editor {
            return body.child(self.editor_form(editor, models, window, p, cx));
        }
        let rows = preferences.profiles.iter().map(|profile| {
            let id = profile.id;
            let missing = missing_profile_model(profile, models).is_some();
            let (edit, ask, confirm, cancel) =
                (this.clone(), this.clone(), this.clone(), this.clone());
            let edited = profile.clone();
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(p.line)
                .px(px(12.))
                .py(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().t_body_strong().child(profile.name.clone()))
                        .child(
                            div()
                                .truncate()
                                .t_label()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(p.ink_muted)
                                .child(profile_summary(profile)),
                        )
                        .when(missing, |el| {
                            el.child(
                                div()
                                    .t_label()
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(p.error)
                                    .child(
                                        "Model is not in the loaded model list; review it before \
                                         creating a session.",
                                    ),
                            )
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("edit-{id}")))
                        .ghost()
                        .small()
                        .label("Edit")
                        .on_click(move |_, window, cx| {
                            let edited = edited.clone();
                            let _ = edit.update(cx, |this, cx| this.edit(Some(edited), window, cx));
                        }),
                )
                .map(|el| {
                    if self.confirm_delete == Some(id) {
                        el.child(
                            Button::new(SharedString::from(format!("confirm-delete-{id}")))
                                .danger()
                                .small()
                                .label("Confirm delete")
                                .on_click(move |_, _, cx| {
                                    let _ = confirm.update(cx, |this, cx| {
                                        this.store
                                            .update(cx, |store, cx| store.delete_profile(id, cx));
                                        this.confirm_delete = None;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new(SharedString::from(format!("cancel-delete-{id}")))
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                    let _ = cancel.update(cx, |this, cx| {
                                        this.confirm_delete = None;
                                        cx.notify();
                                    });
                                }),
                        )
                    } else {
                        el.child(
                            Button::new(SharedString::from(format!("delete-{id}")))
                                .ghost()
                                .small()
                                .icon("trash-2")
                                .tooltip("Delete profile")
                                .on_click(move |_, _, cx| {
                                    let _ = ask.update(cx, |this, cx| {
                                        this.confirm_delete = Some(id);
                                        cx.notify();
                                    });
                                }),
                        )
                    }
                })
        });
        let add = this.clone();
        body.children(rows)
            .when(preferences.profiles.is_empty(), |el| {
                el.child(
                    div()
                        .py(px(20.))
                        .text_center()
                        .text_color(p.ink_muted)
                        .child("No profiles yet. Save a model, thinking level or instructions for reuse."),
                )
            })
            .child(
                div().flex().child(
                    Button::new("new-profile")
                        .icon("plus")
                        .label("New profile")
                        .on_click(move |_, window, cx| {
                            let _ = add.update(cx, |this, cx| this.edit(None, window, cx));
                        }),
                ),
            )
    }

    fn editor_form(
        &self,
        editor: &Editor,
        models: &[ModelInfo],
        window: &Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let preferences = self.store.read(cx).state.preferences.clone();
        let levels = thinking_levels(self.store.read(cx));
        let busy = editor.busy;
        let name_focused = editor.name.read(cx).focus_handle(cx).is_focused(window);
        let empty = editor.name.read(cx).value().trim().is_empty();
        let chosen = models.iter().find(|m| m.id == editor.model);
        let model_value = match chosen {
            Some(m) => div()
                .t_code_sm()
                .child(short_model_id(&m.id, &m.provider))
                .into_any_element(),
            None if !editor.model.is_empty() => div()
                .t_code_sm()
                .child(editor.model.clone())
                .into_any_element(),
            None => div().child("Harness default").into_any_element(),
        };
        let (cancel, save) = (this.clone(), this.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(ds::text_field(
                "Profile name",
                Input::new(&editor.name)
                    .appearance(false)
                    .disabled(busy)
                    .t_body()
                    .px(px(0.)),
                name_focused,
                None,
                None,
                p,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Picker::new(
                            "profile-model",
                            &editor.model_picker,
                            model_groups(models, &editor.model, Some(&preferences)),
                            model_value,
                            p,
                        )
                        .icon("agent")
                        .empty_text("Open a session to load models")
                        .disabled(busy),
                    )
                    .child(
                        Picker::new(
                            "profile-thinking",
                            &editor.thinking_picker,
                            thinking_groups(&levels, &editor.thinking),
                            if editor.thinking.is_empty() {
                                "Model default".to_string()
                            } else {
                                format!("Thinking: {}", editor.thinking)
                            },
                            p,
                        )
                        .icon("circle-dot")
                        .searchable(false)
                        .width(200.)
                        .empty_text("Open a session to load thinking levels")
                        .disabled(busy),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .t_label()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_muted)
                    .child("Additional instructions")
                    .child(
                        div()
                            .rounded(px(RADIUS_MD))
                            .border_1()
                            .border_color(p.line)
                            .bg(p.surface)
                            .px(px(8.))
                            .py(px(4.))
                            .child(
                                Textarea::new(&editor.instructions)
                                    .appearance(false)
                                    .disabled(busy)
                                    .xsmall()
                                    .t_body()
                                    .text_color(p.ink),
                            ),
                    )
                    .child(
                        "Appended to the harness system prompt; the default prompt stays intact.",
                    ),
            )
            .child(
                ds::overlay_footer(p)
                    .child(
                        Button::new("profile-cancel")
                            .ghost()
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let _ = cancel.update(cx, |this, cx| this.cancel_edit(window, cx));
                            }),
                    )
                    .child(
                        div().ml_auto().child(
                            Button::new("profile-save")
                                .primary()
                                .label(if busy {
                                    "Saving\u{2026}"
                                } else {
                                    "Save profile"
                                })
                                .disabled(busy || empty)
                                .on_click(move |_, _, cx| {
                                    let _ = save.update(cx, |this, cx| this.save_profile(cx));
                                }),
                        ),
                    ),
            )
    }

    fn models_tab(&mut self, models: &[ModelInfo], p: &Palette, cx: &mut Context<Self>) -> Div {
        let state = &self.store.read(cx).state;
        let active = state.active_key.clone();
        let preferences = state.preferences.clone();
        if let Some(key) = active.clone()
            && models.is_empty()
            && !self.asked_models
        {
            self.asked_models = true;
            self.store
                .update(cx, |store, cx| store.load_models(key, cx).detach());
        }
        let mut ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
        for id in preferences
            .favorite_models
            .iter()
            .chain(&preferences.hidden_models)
        {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        let sort_key = |id: &String| {
            let model = models.iter().find(|m| &m.id == id);
            format!(
                "{}/{}",
                model.map_or("", |m| m.provider.as_str()),
                model.map_or(id.as_str(), |m| m.name.as_str())
            )
            .to_lowercase()
        };
        ids.sort_by_key(sort_key);
        let store = self.store.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .t_label()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_muted)
                    .child(
                        "Favorites appear first in Panoptes\u{2019} pickers. Hidden models stay out \
                         of pickers but remain configured in the harness and usable by existing \
                         sessions.",
                    ),
            )
            .children(ids.iter().enumerate().map(|(i, id)| {
                let model = models.iter().find(|m| &m.id == id);
                let favorite = preferences.favorite_models.contains(id);
                let hidden = preferences.hidden_models.contains(id);
                let (fav_store, hide_store) = (store.clone(), store.clone());
                let (fav_id, hide_id) = (id.clone(), id.clone());
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_h(px(40.))
                    .rounded(px(RADIUS_MD))
                    .border_1()
                    .border_color(p.line)
                    .px(px(12.))
                    .py(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .child(model.map_or(id.clone(), |m| m.name.clone())),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .t_label()
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(p.ink_muted)
                                    .child(model.map_or("Saved model id".to_string(), |m| {
                                        format!(
                                            "{} \u{b7} {}",
                                            m.provider,
                                            short_model_id(&m.id, &m.provider)
                                        )
                                    })),
                            ),
                    )
                    .child({
                        let button = Button::new(SharedString::from(format!("favorite-{i}")))
                            .small()
                            .label(if favorite { "Favorited" } else { "Favorite" })
                            .on_click(move |_, _, cx| {
                                let id = fav_id.clone();
                                fav_store
                                    .update(cx, |store, cx| store.set_model_favorite(id, !favorite, cx))
                            });
                        if favorite { button } else { button.ghost() }
                    })
                    .child(
                        Button::new(SharedString::from(format!("hide-{i}")))
                            .ghost()
                            .small()
                            .label(if hidden { "Show" } else { "Hide" })
                            .on_click(move |_, _, cx| {
                                let id = hide_id.clone();
                                hide_store
                                    .update(cx, |store, cx| store.set_model_hidden(id, !hidden, cx))
                            }),
                    )
            }))
            .when(ids.is_empty(), |el| {
                el.child(
                    div()
                        .py(px(20.))
                        .text_center()
                        .text_color(p.ink_muted)
                        .child(if active.is_some() {
                            "Loading models\u{2026}"
                        } else {
                            "Open a session to load the models Pi offers."
                        }),
                )
            })
    }

    fn appearance_tab(&self, p: &Palette) -> Div {
        let dark_store = self.store.downgrade();
        let decrease_store = self.store.downgrade();
        let increase_store = self.store.downgrade();
        let reset_store = self.store.downgrade();
        let percent = crate::zoom::font_size();
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        ds::v_flex_gap(4.)
                            .child(div().t_body_strong().child("Dark mode"))
                            .child(
                                div()
                                    .t_small()
                                    .text_color(p.ink_muted)
                                    .child("Use dark colors throughout Panoptes."),
                            ),
                    )
                    .child(ds::switch(
                        "dark-mode",
                        p.dark,
                        "Dark mode",
                        p,
                        move |dark, _, cx| {
                            crate::theme::set_dark_mode(dark, cx);
                            let _ =
                                dark_store.update(cx, |store, cx| store.set_dark_mode(dark, cx));
                        },
                    )),
            )
            .child(div().h(px(1.)).bg(p.line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        ds::v_flex_gap(4.)
                            .child(div().t_body_strong().child("Font size"))
                            .child(
                                div()
                                    .t_small()
                                    .text_color(p.ink_muted)
                                    .child("Scale text and interface controls from 50% to 200%."),
                            ),
                    )
                    .child(
                        ds::h_flex_gap(4.)
                            .items_center()
                            .child(
                                Button::new("font-size-decrease")
                                    .ghost()
                                    .small()
                                    .label("−")
                                    .tooltip("Decrease font size")
                                    .disabled(percent <= (MIN_ZOOM * 100.) as u32)
                                    .on_click(move |_, window, cx| {
                                        if let Some(percent) =
                                            crate::zoom::change(ZoomCommand::Out, window, cx)
                                        {
                                            let _ = decrease_store.update(cx, |store, cx| {
                                                store.set_font_size(percent, cx)
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .min_w(px(48.))
                                    .text_center()
                                    .t_body_strong()
                                    .child(format!("{percent}%")),
                            )
                            .child(
                                Button::new("font-size-increase")
                                    .ghost()
                                    .small()
                                    .label("+")
                                    .tooltip("Increase font size")
                                    .disabled(percent >= (MAX_ZOOM * 100.) as u32)
                                    .on_click(move |_, window, cx| {
                                        if let Some(percent) =
                                            crate::zoom::change(ZoomCommand::In, window, cx)
                                        {
                                            let _ = increase_store.update(cx, |store, cx| {
                                                store.set_font_size(percent, cx)
                                            });
                                        }
                                    }),
                            )
                            .child(
                                Button::new("font-size-reset")
                                    .ghost()
                                    .small()
                                    .label("Reset")
                                    .disabled(percent == 100)
                                    .on_click(move |_, window, cx| {
                                        if let Some(percent) =
                                            crate::zoom::change(ZoomCommand::Reset, window, cx)
                                        {
                                            let _ = reset_store.update(cx, |store, cx| {
                                                store.set_font_size(percent, cx)
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
    }

    fn diagnostics_tab(&self, p: &Palette) -> Div {
        let store = self.store.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().t_body_strong().child("Session diagnostics"))
            .child(
                div()
                    .t_label()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(p.ink_muted)
                    .child(
                        "Open a dedicated debugging window with the session and fork tree, \
                         tool outcomes, token usage, context size, edited files and the complete \
                         structured event log. Logs stay on this machine and are kept for 30 days.",
                    ),
            )
            .child(
                div().child(
                    Button::new("open-diagnostics")
                        .primary()
                        .icon("watch")
                        .label("Open diagnostics window")
                        .on_click(move |_, _, cx| {
                            crate::views::diagnostics::open(store.clone(), cx);
                        }),
                ),
            )
    }

    fn lanes_tab(&self, window: &Window, p: &Palette, cx: &mut Context<Self>) -> Div {
        let text = self.days.read(cx).value().to_string();
        let valid = valid_days(&text).is_some();
        let focused = self.days.read(cx).focus_handle(cx).is_focused(window);
        let busy = self.days_busy;
        let save = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .child(div().t_body_strong().child("Lane cleanup"))
                    .child(
                        div()
                            .mt(px(4.))
                            .t_label()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_muted)
                            .child(
                                "Cleanup offers worktree lanes after this many days without \
                                 session activity. Open sessions and lanes that need branch \
                                 recovery are never included.",
                            ),
                    ),
            )
            .child(ds::text_field(
                "Idle lane threshold (days)",
                Input::new(&self.days)
                    .appearance(false)
                    .disabled(busy)
                    .t_body()
                    .px(px(0.)),
                focused,
                (!valid).then(|| "Enter a whole number from 1 to 365.".into()),
                valid.then(|| "Default: 4 days.".into()),
                p,
            ))
            .child(
                ds::overlay_footer(p).child(
                    div().ml_auto().child(
                        Button::new("save-lanes")
                            .primary()
                            .label(if busy {
                                "Saving\u{2026}"
                            } else {
                                "Save lane settings"
                            })
                            .disabled(busy || !valid)
                            .on_click(move |_, _, cx| {
                                let _ = save.update(cx, |this, cx| this.save_days(cx));
                            }),
                    ),
                ),
            )
    }
}

impl Settings {
    fn notifications_tab(&self, p: &Palette, cx: &mut Context<Self>) -> Div {
        let notifications = self.store.read(cx).state.preferences.notifications;
        // Each switch is persisted independently, so quick successive clicks
        // cannot overwrite one another with a stale preferences snapshot.
        let toggle = |field: NotificationField| {
            let store = self.store.clone();
            move |checked: bool, _: &mut Window, cx: &mut App| {
                store.update(cx, |store, cx| store.set_notification(field, checked, cx));
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .child(div().t_body_strong().child("System notifications"))
                    .child(
                        div()
                            .mt(px(4.))
                            .t_label()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.ink_muted)
                            .child(
                                "macOS shows a notification when a session in another \
                                 window or another app needs you. The session you are \
                                 looking at never notifies, and nothing is sent while \
                                 notifications are off.",
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(ds::checkbox(
                        "notify-enabled",
                        notifications.enabled,
                        "Show notifications",
                        Some(
                            "Turn every notification off without changing the switches below."
                                .into(),
                        ),
                        false,
                        p,
                        toggle(NotificationField::Enabled),
                    ))
                    .child(ds::checkbox(
                        "notify-finished",
                        notifications.finished,
                        "A run finishes",
                        Some("Tell me when a session reaches Done.".into()),
                        !notifications.enabled,
                        p,
                        toggle(NotificationField::Finished),
                    ))
                    .child(ds::checkbox(
                        "notify-needs-attention",
                        notifications.needs_attention,
                        "A session needs me",
                        Some(
                            "Tell me when a session reaches Waiting: a question or an approval."
                                .into(),
                        ),
                        !notifications.enabled,
                        p,
                        toggle(NotificationField::NeedsAttention),
                    ))
                    // The kinds of event come first; the sound is what they
                    // are accompanied by, so it follows them.
                    .child(ds::checkbox(
                        "notify-sound",
                        notifications.sound,
                        "Play a sound",
                        Some("Play Panoptes' alert sound with the notification.".into()),
                        !notifications.enabled,
                        p,
                        toggle(NotificationField::Sound),
                    )),
            )
            // Hearing it is the quickest way to know whether to turn it off.
            // Off when nothing would be heard anyway, and while a preview is
            // still playing, so clicks cannot stack sounds.
            .child(
                div().child(
                    Button::new("notify-preview-sound")
                        .ghost()
                        .small()
                        .icon("play")
                        .label("Play the sound")
                        .disabled(!notifications.enabled || !notifications.sound)
                        .on_click(|_, _, _| crate::notify::play_sound()),
                ),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let this = cx.entity().downgrade();
        let models = available_models(self.store.read(cx));
        let profile_count = self.store.read(cx).state.preferences.profiles.len();
        let on_layer_close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.close(cx)).unwrap_or(false)
            }
        };
        let header_close: ds::CloseHandler = {
            let this = this.clone();
            std::rc::Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| this.close(cx));
            })
        };
        let tab = self.tab;
        let tabs = ds::tabs(
            vec![
                Tab {
                    id: "appearance",
                    label: "Appearance".into(),
                    icon: Some("settings"),
                    count: None,
                },
                Tab {
                    id: "profiles",
                    label: "Profiles".into(),
                    icon: Some("agent"),
                    count: Some(profile_count),
                },
                Tab {
                    id: "models",
                    label: "Models".into(),
                    icon: Some("circle-dot"),
                    count: None,
                },
                Tab {
                    id: "lanes",
                    label: "Lanes".into(),
                    icon: Some("git-branch"),
                    count: None,
                },
                Tab {
                    id: "notifications",
                    label: "Notifications".into(),
                    icon: Some("circle-check"),
                    count: None,
                },
                Tab {
                    id: "diagnostics",
                    label: "Diagnostics".into(),
                    icon: Some("watch"),
                    count: None,
                },
            ],
            match tab {
                SettingsTab::Appearance => "appearance",
                SettingsTab::Profiles => "profiles",
                SettingsTab::Models => "models",
                SettingsTab::Lanes => "lanes",
                SettingsTab::Notifications => "notifications",
                SettingsTab::Diagnostics => "diagnostics",
            },
            &p,
            {
                let this = this.clone();
                move |id, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.tab = match id {
                            "profiles" => SettingsTab::Profiles,
                            "models" => SettingsTab::Models,
                            "lanes" => SettingsTab::Lanes,
                            "notifications" => SettingsTab::Notifications,
                            "diagnostics" => SettingsTab::Diagnostics,
                            _ => SettingsTab::Appearance,
                        };
                        this.editor = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    });
                }
            },
        );
        let body = match tab {
            SettingsTab::Appearance => self.appearance_tab(&p),
            SettingsTab::Profiles => self.profiles_tab(&models, window, &p, cx),
            SettingsTab::Models => self.models_tab(&models, &p, cx),
            SettingsTab::Lanes => self.lanes_tab(window, &p, cx),
            SettingsTab::Notifications => self.notifications_tab(&p, cx),
            SettingsTab::Diagnostics => self.diagnostics_tab(&p),
        };
        let panel = ds::overlay_panel(680., window, &p)
            .track_focus(&self.focus)
            .child(ds::overlay_header(
                div().t_heading().child("Settings"),
                Some(header_close),
            ))
            .child(div().flex_none().px(px(16.)).pb(px(8.)).child(tabs))
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(16.))
                    .pb(px(16.))
                    .child(body),
            );
        ds::modal_layer(window, on_layer_close, panel, &p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn a_profile_summary_names_what_it_sets() {
        let profile = AgentProfile {
            id: 1,
            name: "Careful".into(),
            model_id: None,
            thinking_level: Some("high".into()),
            instructions: Some("Run tests".into()),
        };
        assert_eq!(
            profile_summary(&profile),
            "Harness default model \u{b7} Thinking: high \u{b7} Additional instructions"
        );
    }

    #[::core::prelude::v1::test]
    fn idle_days_are_whole_numbers_from_one_to_a_year() {
        assert_eq!(valid_days(" 4 "), Some(4));
        assert_eq!(valid_days("365"), Some(365));
        assert_eq!(valid_days("0"), None);
        assert_eq!(valid_days("366"), None);
        assert_eq!(valid_days("2.5"), None);
        assert_eq!(valid_days(""), None);
    }
}
