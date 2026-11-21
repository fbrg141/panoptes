//! The searchable picker behind the composer's model menu and the new lane and
//! settings forms' menus, with the model, thinking and profile lists it shows.

use std::collections::{HashMap, HashSet};

use panoptes_core::store::{AgentProfile, Preferences};
use panoptes_harness::ModelInfo;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
    /// A second, quieter line of identity: an id, a branch, a path.
    pub detail: Option<String>,
    /// Extra words a search should match besides the label and detail.
    pub keywords: Vec<String>,
    /// The value in use now; it is ticked and the list opens on it.
    pub current: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerGroup {
    /// A heading over the group; groups without one run on from the last.
    pub label: Option<String>,
    pub items: Vec<PickerItem>,
}

/// The groups narrowed to `query`. An item stays when its label, detail or
/// keywords contain every word typed; groups left empty drop out. Order is
/// kept, so the list does not jump about as you type.
pub fn filter_picker(groups: &[PickerGroup], query: &str) -> Vec<PickerGroup> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return groups.to_vec();
    }
    groups
        .iter()
        .map(|group| PickerGroup {
            label: group.label.clone(),
            items: group
                .items
                .iter()
                .filter(|item| {
                    let mut text = vec![item.label.as_str(), item.detail.as_deref().unwrap_or("")];
                    text.extend(item.keywords.iter().map(String::as_str));
                    let text = text.join(" ").to_lowercase();
                    words.iter().all(|word| text.contains(word))
                })
                .cloned()
                .collect(),
        })
        .filter(|group| !group.items.is_empty())
        .collect()
}

/// The items a keyboard can land on, in display order.
pub fn pickable(groups: &[PickerGroup]) -> Vec<&PickerItem> {
    groups
        .iter()
        .flat_map(|group| group.items.iter().filter(|item| !item.disabled))
        .collect()
}

/// Where the highlight starts: the current item if it is listed, else the first.
pub fn initial_pick(groups: &[PickerGroup]) -> usize {
    pickable(groups)
        .iter()
        .position(|item| item.current)
        .unwrap_or(0)
}

/// The picker's first row: leave the choice to the harness.
pub const DEFAULT_CHOICE: &str = "";

/// Borrow a populated list from the active session, or another open one.
pub fn borrowed_models<'a>(
    by_session: &'a HashMap<String, Vec<ModelInfo>>,
    active_key: Option<&str>,
) -> Option<&'a Vec<ModelInfo>> {
    active_key
        .and_then(|key| by_session.get(key))
        .filter(|list| !list.is_empty())
        .or_else(|| by_session.values().find(|list| !list.is_empty()))
}

/// A model's id without its provider prefix, which its group already names.
pub fn short_model_id(id: &str, provider: &str) -> String {
    id.strip_prefix(provider)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(id)
        .to_string()
}

/// Models by provider, after a row that keeps the harness's default.
/// Favorites come first; hidden models drop out unless in use.
pub fn model_groups(
    models: &[ModelInfo],
    model_id: &str,
    preferences: Option<&Preferences>,
) -> Vec<PickerGroup> {
    let mut groups = vec![PickerGroup {
        label: None,
        items: vec![PickerItem {
            id: DEFAULT_CHOICE.into(),
            label: "Harness default".into(),
            current: model_id == DEFAULT_CHOICE,
            ..Default::default()
        }],
    }];
    let hidden: HashSet<&str> = preferences
        .map(|p| p.hidden_models.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let visible: Vec<&ModelInfo> = models
        .iter()
        .filter(|m| !hidden.contains(m.id.as_str()) || m.id == model_id)
        .collect();
    let item = |model: &ModelInfo| PickerItem {
        id: model.id.clone(),
        label: model.name.clone(),
        detail: Some(short_model_id(&model.id, &model.provider)),
        keywords: vec![model.provider.clone(), model.id.clone()],
        current: model.id == model_id,
        disabled: false,
    };
    let favorites: Vec<&ModelInfo> = preferences
        .map(|p| p.favorite_models.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|id| visible.iter().find(|m| &m.id == id).copied())
        .collect();
    if !favorites.is_empty() {
        groups.push(PickerGroup {
            label: Some("Favorites".into()),
            items: favorites.iter().map(|m| item(m)).collect(),
        });
    }
    for model in visible {
        if favorites.iter().any(|f| f.id == model.id) {
            continue;
        }
        let label = Some(model.provider.clone());
        match groups.iter_mut().find(|g| g.label == label) {
            Some(group) => group.items.push(item(model)),
            None => groups.push(PickerGroup {
                label,
                items: vec![item(model)],
            }),
        }
    }
    groups
}

/// The model's default before its thinking levels.
pub fn thinking_groups(levels: &[String], level: &str) -> Vec<PickerGroup> {
    let mut items = vec![PickerItem {
        id: DEFAULT_CHOICE.into(),
        label: "Model default".into(),
        current: level == DEFAULT_CHOICE,
        ..Default::default()
    }];
    items.extend(levels.iter().map(|l| PickerItem {
        id: l.clone(),
        label: l.clone(),
        current: l == level,
        ..Default::default()
    }));
    vec![PickerGroup { label: None, items }]
}

/// A saved model missing from a loaded list is stale; an empty list proves
/// nothing.
pub fn missing_profile_model<'a>(
    profile: &'a AgentProfile,
    models: &[ModelInfo],
) -> Option<&'a str> {
    let id = profile.model_id.as_deref()?;
    (!models.is_empty() && !models.iter().any(|m| m.id == id)).then_some(id)
}

/// Reusable session setups, after a row that leaves every field unset.
pub fn profile_groups(profiles: &[AgentProfile], profile_id: Option<i64>) -> Vec<PickerGroup> {
    let mut items = vec![PickerItem {
        id: DEFAULT_CHOICE.into(),
        label: "No profile".into(),
        current: profile_id.is_none(),
        ..Default::default()
    }];
    items.extend(profiles.iter().map(|p| PickerItem {
        id: p.id.to_string(),
        label: p.name.clone(),
        detail: p.model_id.clone(),
        current: Some(p.id) == profile_id,
        ..Default::default()
    }));
    vec![PickerGroup { label: None, items }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, label: &str) -> PickerItem {
        PickerItem {
            id: id.into(),
            label: label.into(),
            ..Default::default()
        }
    }

    fn groups() -> Vec<PickerGroup> {
        vec![
            PickerGroup {
                label: Some("anthropic".into()),
                items: vec![
                    PickerItem {
                        detail: Some("claude-sonnet-4-5".into()),
                        keywords: vec!["anthropic".into()],
                        current: true,
                        ..item("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5")
                    },
                    PickerItem {
                        keywords: vec!["anthropic".into()],
                        ..item("anthropic/claude-haiku-4-5", "Claude Haiku 4.5")
                    },
                ],
            },
            PickerGroup {
                label: Some("local".into()),
                items: vec![
                    PickerItem {
                        disabled: true,
                        ..item("local/off", "Offline")
                    },
                    PickerItem {
                        detail: Some("qwen3-coder".into()),
                        keywords: vec!["local".into()],
                        ..item("local/qwen3-coder", "Qwen3 Coder")
                    },
                ],
            },
        ]
    }

    fn ids(groups: &[PickerGroup]) -> Vec<String> {
        pickable(groups).iter().map(|i| i.id.clone()).collect()
    }

    #[test]
    fn filters_by_every_word_keeping_order() {
        let groups = groups();
        assert_eq!(filter_picker(&groups, " "), groups);
        assert_eq!(
            ids(&filter_picker(&groups, "anthropic")),
            ["anthropic/claude-sonnet-4-5", "anthropic/claude-haiku-4-5"]
        );
        assert_eq!(
            ids(&filter_picker(&groups, "sonnet-4")),
            ["anthropic/claude-sonnet-4-5"]
        );
        assert_eq!(
            ids(&filter_picker(&groups, "claude haiku")),
            ["anthropic/claude-haiku-4-5"]
        );
        let labels: Vec<_> = filter_picker(&groups, "qwen")
            .into_iter()
            .map(|g| g.label)
            .collect();
        assert_eq!(labels, [Some("local".to_string())]);
        assert!(filter_picker(&groups, "nothing").is_empty());
    }

    #[test]
    fn skips_disabled_items_and_starts_on_the_current_one() {
        let groups = groups();
        assert!(!ids(&groups).contains(&"local/off".to_string()));
        assert_eq!(initial_pick(&groups), 0);
        let b = PickerItem {
            current: true,
            ..item("b", "B")
        };
        let two = [PickerGroup {
            label: None,
            items: vec![item("a", "A"), b],
        }];
        assert_eq!(initial_pick(&two), 1);
    }

    fn model(id: &str, name: &str, provider: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: name.into(),
            provider: provider.into(),
            context_window: None,
            images: true,
        }
    }

    fn preferences(favorites: &[&str], hidden: &[&str]) -> Preferences {
        Preferences {
            profiles: vec![],
            favorite_models: favorites.iter().map(|s| s.to_string()).collect(),
            hidden_models: hidden.iter().map(|s| s.to_string()).collect(),
            cleanup_idle_days: 14,
            permission_warning_dismissed: false,
        }
    }

    fn labels(groups: &[PickerGroup]) -> Vec<Option<&str>> {
        groups.iter().map(|g| g.label.as_deref()).collect()
    }

    #[test]
    fn groups_models_by_provider_after_the_default() {
        let groups = model_groups(
            &[
                model(
                    "anthropic/claude-sonnet-4-5",
                    "Claude Sonnet 4.5",
                    "anthropic",
                ),
                model("local/qwen", "Qwen", "local"),
                model(
                    "anthropic/claude-haiku-4-5",
                    "Claude Haiku 4.5",
                    "anthropic",
                ),
            ],
            "local/qwen",
            None,
        );
        assert_eq!(labels(&groups), [None, Some("anthropic"), Some("local")]);
        assert!(!groups[0].items[0].current);
        let details: Vec<_> = groups[1].items.iter().map(|i| i.detail.clone()).collect();
        assert_eq!(
            details,
            [
                Some("claude-sonnet-4-5".into()),
                Some("claude-haiku-4-5".into())
            ]
        );
        assert!(groups[2].items[0].current);
    }

    #[test]
    fn favorites_first_and_hidden_models_drop_out_unless_current() {
        let prefs = preferences(
            &["openai/gpt", "anthropic/claude-sonnet", "missing/model"],
            &["local/qwen"],
        );
        let groups = model_groups(
            &[
                model("anthropic/claude-sonnet", "Claude Sonnet", "anthropic"),
                model("openai/gpt", "GPT", "openai"),
                model("anthropic/claude-haiku", "Claude Haiku", "anthropic"),
                model("local/qwen", "Qwen", "local"),
            ],
            "anthropic/claude-haiku",
            Some(&prefs),
        );
        assert_eq!(
            labels(&groups),
            [None, Some("Favorites"), Some("anthropic")]
        );
        assert_eq!(
            ids(&groups[1..2]),
            ["openai/gpt", "anthropic/claude-sonnet"]
        );
        assert!(groups[2].items[0].current);
        let prefs = preferences(&[], &["local/qwen"]);
        let groups = model_groups(
            &[
                model("local/qwen", "Qwen", "local"),
                model("openai/gpt", "GPT", "openai"),
            ],
            "local/qwen",
            Some(&prefs),
        );
        assert_eq!(labels(&groups), [None, Some("local"), Some("openai")]);
    }

    #[test]
    fn borrows_models_and_shortens_ids() {
        let small = model("p/small", "Small", "p");
        let map: HashMap<String, Vec<ModelInfo>> = [
            ("active".to_string(), vec![]),
            ("other".to_string(), vec![small.clone()]),
        ]
        .into();
        assert_eq!(borrowed_models(&map, Some("active")), Some(&vec![small]));
        assert_eq!(borrowed_models(&HashMap::new(), None), None);
        assert_eq!(short_model_id("openai/gpt-5", "openai"), "gpt-5");
        assert_eq!(short_model_id("gpt-5", "openai"), "gpt-5");
    }

    #[test]
    fn profiles_and_thinking_levels() {
        let profile = AgentProfile {
            id: 4,
            name: "Careful".into(),
            model_id: Some("removed/model".into()),
            thinking_level: Some("high".into()),
            instructions: None,
        };
        let groups = profile_groups(std::slice::from_ref(&profile), Some(4));
        assert_eq!(
            groups[0]
                .items
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["", "4"]
        );
        assert!(groups[0].items[1].current);
        assert_eq!(missing_profile_model(&profile, &[]), None);
        assert_eq!(
            missing_profile_model(&profile, &[model("openai/gpt", "GPT", "openai")]),
            Some("removed/model")
        );
        let levels: Vec<String> = ["low", "high"].map(String::from).into();
        let groups = thinking_groups(&levels, "high");
        assert_eq!(groups[0].items.len(), 3);
        assert!(groups[0].items[2].current);
    }
}
