use super::*;

#[test]
fn pins_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.set_pinned("pi:a", true).unwrap();
    store.set_pinned("pi:b", true).unwrap();
    store.set_pinned("pi:b", true).unwrap();
    store.set_pinned("pi:a", false).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
}

#[test]
fn profiles_and_model_preferences_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(!store.preferences().unwrap().permission_warning_dismissed);
    store.dismiss_permission_warning().unwrap();
    let input = ProfileInput {
        name: "  Careful  ".into(),
        model_id: Some(" anthropic/claude-sonnet ".into()),
        thinking_level: Some(" high ".into()),
        instructions: Some(" Run the tests. ".into()),
    };
    store.save_profile(None, &input).unwrap();
    store.set_model_favorite("openai/gpt", true).unwrap();
    store
        .set_model_favorite("anthropic/claude-sonnet", true)
        .unwrap();
    store.set_model_hidden("local/qwen", true).unwrap();
    store.set_model_favorite("openai/gpt", false).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let preferences = store.preferences().unwrap();
    assert_eq!(preferences.profiles.len(), 1);
    assert_eq!(preferences.profiles[0].name, "Careful");
    assert_eq!(
        preferences.profiles[0].model_id.as_deref(),
        Some("anthropic/claude-sonnet")
    );
    assert_eq!(
        preferences.profiles[0].thinking_level.as_deref(),
        Some("high")
    );
    assert_eq!(
        preferences.profiles[0].instructions.as_deref(),
        Some("Run the tests.")
    );
    assert_eq!(preferences.favorite_models, ["anthropic/claude-sonnet"]);
    assert_eq!(preferences.hidden_models, ["local/qwen"]);
    assert_eq!(preferences.cleanup_idle_days, 4);
    store.set_cleanup_idle_days(9).unwrap();
    assert!(store.set_cleanup_idle_days(0).is_err());
    assert!(store.set_cleanup_idle_days(366).is_err());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.preferences().unwrap().cleanup_idle_days, 9);
    assert!(store.preferences().unwrap().permission_warning_dismissed);

    store
        .save_profile(
            Some(preferences.profiles[0].id),
            &ProfileInput {
                name: "Careful review".into(),
                model_id: None,
                thinking_level: None,
                instructions: None,
            },
        )
        .unwrap();
    store.delete_profile(preferences.profiles[0].id).unwrap();
    assert!(store.preferences().unwrap().profiles.is_empty());
    assert!(
        store
            .save_profile(
                None,
                &ProfileInput {
                    name: " ".into(),
                    model_id: None,
                    thinking_level: None,
                    instructions: None,
                },
            )
            .is_err()
    );
}
