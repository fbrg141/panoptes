use super::*;
use crate::store::turns::turn_refs;
use crate::store::turns::{SettleResult, SnapshotResult};
use crate::store::{Boundary, TurnOutcome};

fn fixture() -> (tempfile::TempDir, Store, crate::store::Lane, i64, i64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let project = store
        .add_project("/code/app", &settings("App", &[".env"]))
        .unwrap();
    let lane = store
        .add_lane(&NewLane {
            project_id: project.id,
            name: "one".into(),
            task: "Do one".into(),
            branch: "panoptes/one".into(),
            worktrees: vec![NewLaneWorktree {
                project_id: project.id,
                path: "/lanes/one/app".into(),
                main_checkout: false,
            }],
        })
        .unwrap();
    let session = store.add_session(lane.id, "pi").unwrap();
    (dir, store, lane, project.id, session.id)
}

fn before(project_id: i64, tree: &str, human: bool) -> SnapshotResult {
    SnapshotResult {
        project_id,
        tree: tree.into(),
        human_edits_before: human,
    }
}

fn after(project_id: i64, tree: &str, recovered: bool) -> SettleResult {
    SettleResult {
        project_id,
        tree: tree.into(),
        recovered,
    }
}

#[test]
fn turn_numbers_are_lane_wide_and_intent_names_all_worktrees() {
    let (_dir, store, lane, project, session) = fixture();
    let first = store
        .create_turn(
            lane.id,
            session,
            Some("m1"),
            "Fix login\nsecond line",
            false,
        )
        .unwrap();
    assert_eq!(first.turn, 1);
    assert_eq!(first.prompt, "Fix login");
    assert!(!first.queued);
    assert_eq!(first.session_id, session);
    assert_eq!(first.worktrees.len(), 1);
    let (before_ref, after_ref) = turn_refs(&lane, 1, project);
    assert_eq!(first.worktrees[0].before_ref, before_ref);
    assert_eq!(first.worktrees[0].after_ref, after_ref);
    assert_eq!(first.worktrees[0].boundary, Boundary::Unavailable);

    let second = store
        .create_turn(lane.id, session, None, "Add tests", true)
        .unwrap();
    assert_eq!(second.turn, 2);
    assert!(second.queued);
    assert_eq!(
        store
            .unfinished_turns()
            .unwrap()
            .iter()
            .map(|t| t.turn)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn before_and_after_snapshots_settle_and_detect_human_edits() {
    let (_dir, store, lane, project, session) = fixture();
    let turn = store
        .create_turn(lane.id, session, None, "Prompt", false)
        .unwrap()
        .turn;
    assert_eq!(store.previous_after(lane.id, turn, project).unwrap(), None);
    store
        .set_turn_before(
            lane.id,
            turn,
            &before(project, &"a".repeat(40), false),
            Boundary::Exact,
            None,
        )
        .unwrap();
    store
        .set_turn_after(lane.id, turn, &after(project, &"b".repeat(40), false), None)
        .unwrap();
    store.settle_turn(lane.id, turn, TurnOutcome::Done).unwrap();
    let complete = store.turn(lane.id, turn).unwrap();
    assert_eq!(complete.outcome, Some(TurnOutcome::Done));
    assert!(complete.settled_at.is_some());
    assert_eq!(complete.worktrees[0].boundary, Boundary::Exact);
    assert!(!complete.worktrees[0].human_edits_before);
    assert!(store.unfinished_turns().unwrap().is_empty());

    let next = store
        .create_turn(lane.id, session, None, "Prompt again", false)
        .unwrap()
        .turn;
    assert_eq!(
        store.previous_after(lane.id, next, project).unwrap(),
        Some((turn_refs(&lane, turn, project).1, "b".repeat(40)))
    );
    store
        .set_turn_before(
            lane.id,
            next,
            &before(project, &"c".repeat(40), true),
            Boundary::Observed,
            None,
        )
        .unwrap();
    store
        .set_turn_after(lane.id, next, &after(project, &"d".repeat(40), true), None)
        .unwrap();
    store
        .settle_turn(lane.id, next, TurnOutcome::Stopped)
        .unwrap();
    let recovered = store.turn(lane.id, next).unwrap();
    assert_eq!(recovered.outcome, Some(TurnOutcome::Stopped));
    assert_eq!(recovered.worktrees[0].boundary, Boundary::Observed);
    assert!(recovered.worktrees[0].human_edits_before);
    assert!(recovered.worktrees[0].recovered);
}

#[test]
fn a_failed_before_snapshot_keeps_its_boundary_unavailable() {
    let (_dir, store, lane, project, session) = fixture();
    let turn = store
        .create_turn(lane.id, session, None, "Prompt", false)
        .unwrap()
        .turn;
    store
        .fail_turn_before(lane.id, turn, project, "no snapshot")
        .unwrap();
    // The prompt still answers Started, but a boundary without a before tree
    // has nothing to compare against: promoting it would show a diff that is
    // empty rather than unknown.
    store
        .mark_turn_boundary(lane.id, turn, Boundary::Exact)
        .unwrap();
    let row = &store.turn(lane.id, turn).unwrap().worktrees[0];
    assert_eq!(row.boundary, Boundary::Unavailable);
    assert_eq!(row.error.as_deref(), Some("no snapshot"));
}

#[test]
fn an_abandoned_turn_settles_with_its_reason() {
    let (_dir, store, lane, project, session) = fixture();
    let turn = store
        .create_turn(lane.id, session, None, "Prompt", false)
        .unwrap()
        .turn;
    store
        .set_turn_before(
            lane.id,
            turn,
            &before(project, &"a".repeat(40), false),
            Boundary::Exact,
            None,
        )
        .unwrap();
    store
        .abandon_turn(lane.id, turn, "the lane lost its worktree")
        .unwrap();
    let abandoned = store.turn(lane.id, turn).unwrap();
    // Settled, so it is not retried at every launch, and the reason is kept.
    assert_eq!(abandoned.outcome, Some(TurnOutcome::Failed));
    assert_eq!(
        abandoned.worktrees[0].after_error.as_deref(),
        Some("the lane lost its worktree")
    );
    assert!(abandoned.worktrees[0].after_tree.is_none());
    assert!(store.unfinished_turns().unwrap().is_empty());
}

#[test]
fn settling_waits_for_every_worktree_and_after_is_idempotent() {
    let (_dir, store, _, project, _session) = fixture();
    let another = store
        .add_project("/code/api", &settings("Api", &[]))
        .unwrap();
    let two = store
        .add_lane(&NewLane {
            project_id: project,
            name: "two".into(),
            task: "Do two".into(),
            branch: "panoptes/two".into(),
            worktrees: vec![
                NewLaneWorktree {
                    project_id: project,
                    path: "/lanes/two/app".into(),
                    main_checkout: false,
                },
                NewLaneWorktree {
                    project_id: another.id,
                    path: "/lanes/two/api".into(),
                    main_checkout: true,
                },
            ],
        })
        .unwrap();
    let two_session = store.add_session(two.id, "pi").unwrap();
    let turn = store
        .create_turn(two.id, two_session.id, None, "Prompt", false)
        .unwrap()
        .turn;
    assert_eq!(store.turn(two.id, turn).unwrap().worktrees.len(), 2);
    store
        .set_turn_before(
            two.id,
            turn,
            &before(project, &"a".repeat(40), false),
            Boundary::Exact,
            None,
        )
        .unwrap();
    store
        .set_turn_before(
            two.id,
            turn,
            &before(another.id, &"e".repeat(40), false),
            Boundary::Exact,
            None,
        )
        .unwrap();
    store
        .set_turn_after(two.id, turn, &after(project, &"b".repeat(40), false), None)
        .unwrap();
    assert!(store.settle_turn(two.id, turn, TurnOutcome::Done).is_err());
    store
        .set_turn_after(
            two.id,
            turn,
            &after(another.id, &"f".repeat(40), false),
            None,
        )
        .unwrap();
    store
        .set_turn_after(
            two.id,
            turn,
            &after(project, &"late".repeat(10), false),
            None,
        )
        .unwrap();
    store.settle_turn(two.id, turn, TurnOutcome::Done).unwrap();
    assert_eq!(
        store.turn(two.id, turn).unwrap().worktrees[0].after_tree,
        Some("b".repeat(40))
    );
}

#[test]
fn discard_and_owned_refs_follow_turn_rows() {
    let (_dir, store, lane, project, session) = fixture();
    let turn = store
        .create_turn(lane.id, session, None, "Prompt", false)
        .unwrap()
        .turn;
    let (before_ref, after_ref) = turn_refs(&lane, turn, project);
    assert!(store.owned_turn_refs().unwrap().contains(&before_ref));
    assert!(store.owned_turn_refs().unwrap().contains(&after_ref));
    store.discard_turn(lane.id, turn).unwrap();
    assert!(store.turn(lane.id, turn).is_err());
    assert!(store.owned_turn_refs().unwrap().is_empty());
}

#[test]
fn lane_deletion_drops_turn_rows() {
    let (_dir, store, lane, project, session) = fixture();
    let turn = store
        .create_turn(lane.id, session, None, "Prompt", false)
        .unwrap()
        .turn;
    store
        .set_turn_before(
            lane.id,
            turn,
            &before(project, &"a".repeat(40), false),
            Boundary::Exact,
            None,
        )
        .unwrap();
    store.delete_lane(lane.id).unwrap();
    assert!(store.turn(lane.id, turn).is_err());
    assert!(store.unfinished_turns().unwrap().is_empty());
}
