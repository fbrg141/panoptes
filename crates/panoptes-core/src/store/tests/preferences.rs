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
    let initial = store.preferences().unwrap();
    assert!(!initial.permission_warning_dismissed);
    assert_eq!(initial.notifications, NotificationPreferences::default());
    assert_eq!(initial.dark_mode, None);
    assert_eq!(initial.font_size, DEFAULT_FONT_SIZE);
    store.dismiss_permission_warning().unwrap();
    store
        .set_notification(NotificationField::Finished, false)
        .unwrap();
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
    store.set_dark_mode(true).unwrap();
    store.set_font_size(120).unwrap();
    assert!(store.set_font_size(95).is_err());
    assert!(store.set_font_size(MIN_FONT_SIZE - 10).is_err());
    assert!(store.set_font_size(MAX_FONT_SIZE + 10).is_err());
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    let reopened = store.preferences().unwrap();
    assert_eq!(reopened.cleanup_idle_days, 9);
    assert!(reopened.permission_warning_dismissed);
    assert_eq!(reopened.dark_mode, Some(true));
    assert_eq!(reopened.font_size, 120);

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

#[test]
fn notification_settings_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .set_notification(NotificationField::Enabled, false)
        .unwrap();
    store
        .set_notification(NotificationField::Finished, false)
        .unwrap();
    store
        .set_notification(NotificationField::NeedsAttention, true)
        .unwrap();
    store
        .set_notification(NotificationField::Sound, false)
        .unwrap();
    // Changing one switch must not restore a previous snapshot of another.
    assert_eq!(
        store.preferences().unwrap().notifications,
        NotificationPreferences {
            enabled: false,
            finished: false,
            needs_attention: true,
            sound: false,
        }
    );
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.preferences().unwrap().notifications,
        NotificationPreferences {
            enabled: false,
            finished: false,
            needs_attention: true,
            sound: false,
        }
    );
}

#[test]
fn the_notification_sound_defaults_on_and_can_be_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.preferences().unwrap().notifications.sound);
    store
        .set_notification(NotificationField::Sound, false)
        .unwrap();
    // Only the sound changed.
    assert_eq!(
        store.preferences().unwrap().notifications,
        NotificationPreferences {
            sound: false,
            ..NotificationPreferences::default()
        }
    );
}
