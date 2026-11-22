use super::*;

#[test]
fn sessions_get_an_id_first_and_their_harness_ref_later() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let lane = lane_in(&store);
    let session = store
        .add_session_configured(lane.id, "pi", Some("  Run the tests.  "))
        .unwrap();
    assert_eq!(session.lane_id, lane.id);
    assert_eq!(session.harness_id, None);
    assert_eq!(store.session_for("pi", "abc").unwrap(), None);

    store
        .set_harness_ref(session.id, "abc", "/s/abc.jsonl")
        .unwrap();
    store
        .set_session_title(session.id, Some("  Fix it "))
        .unwrap();
    store.set_session_pinned(session.id, true).unwrap();
    store
        .set_session_tags(session.id, &["ui".into(), " ".into(), "bug".into()])
        .unwrap();
    // Recorded sessions survive a reopen, archived or not.
    store.set_session_archived(session.id, true).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    let session = store.session(session.id).unwrap();
    assert_eq!(session.harness_id.as_deref(), Some("abc"));
    assert_eq!(session.locator.as_deref(), Some("/s/abc.jsonl"));
    assert_eq!(session.title.as_deref(), Some("Fix it"));
    assert_eq!(session.instructions.as_deref(), Some("Run the tests."));
    assert_eq!(session.tags, ["bug", "ui"]);
    assert!(session.pinned && session.archived);
    assert_eq!(store.session_for("pi", "abc").unwrap(), Some(session.id));
    // Archiving the session leaves its lane alone.
    assert_eq!(store.lane(lane.id).unwrap().state, LaneState::Active);
    // One harness conversation, one Panoptes session, even across lanes.
    let other_lane = another_lane_in(&store, &lane);
    let other = store.add_session(other_lane.id, "pi").unwrap();
    assert!(
        store
            .set_harness_ref(other.id, "abc", "/s/abc.jsonl")
            .is_err()
    );
}

#[test]
fn adoption_persists_the_harness_ref_and_moves_its_legacy_pin() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let lane = lane_in(&store);
    store.set_pinned("pi:abc", true).unwrap();
    let adopted = store
        .adopt_session(lane.id, "pi", "abc", "/s/abc.jsonl")
        .unwrap();
    assert_eq!(adopted.lane_id, lane.id);
    assert_eq!(adopted.harness_id.as_deref(), Some("abc"));
    assert!(adopted.pinned);
    assert!(store.pinned().unwrap().is_empty());
    assert!(
        store
            .adopt_session(lane.id, "pi", "abc", "/s/abc.jsonl")
            .is_err()
    );
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.session_for("pi", "abc").unwrap(), Some(adopted.id));
    assert!(store.session(adopted.id).unwrap().pinned);
}

#[test]
fn only_unstarted_sessions_are_discarded_and_archived_lanes_take_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let lane = lane_in(&store);
    let started = store.add_session(lane.id, "pi").unwrap();
    store.set_harness_ref(started.id, "abc", "").unwrap();
    let other_lane = another_lane_in(&store, &lane);
    let unstarted = store.add_session(other_lane.id, "pi").unwrap();
    store.discard_session(started.id).unwrap();
    store.discard_session(unstarted.id).unwrap();
    let left: Vec<i64> = store.sessions().unwrap().iter().map(|s| s.id).collect();
    assert_eq!(left, [started.id]);
    assert_eq!(store.session(started.id).unwrap().locator, None);

    store.archive_lane(lane.id, None).unwrap();
    assert!(store.add_session(lane.id, "pi").is_err());
    assert!(store.set_session_pinned(999, true).is_err());
    assert!(store.set_session_tags(999, &[]).is_err());
}

#[test]
fn an_allow_is_remembered_per_scope_and_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .allow_action(&lane_scope(3), "execute: rm -rf build")
        .unwrap();
    store
        .allow_action(&lane_scope(3), "execute: rm -rf build")
        .unwrap();
    store
        .allow_action(&lane_scope(3), "edit: src/main.rs")
        .unwrap();
    store
        .allow_action(&session_scope("pi:abc"), "execute: git push")
        .unwrap();

    assert_eq!(
        store.allowed_actions(&lane_scope(3)).unwrap(),
        ["execute: rm -rf build", "edit: src/main.rs"]
    );
    assert_eq!(
        store.allowed_actions(&session_scope("pi:abc")).unwrap(),
        ["execute: git push"]
    );
    // Another lane inherits nothing.
    assert!(store.allowed_actions(&lane_scope(4)).unwrap().is_empty());
}
