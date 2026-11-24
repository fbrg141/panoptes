use std::fs;
use std::path::{Path, PathBuf};

use panoptes_harness::{HarnessEvent, PromptMode, PromptOutcome, QueuedPrompt};

use super::fake::*;
use crate::git::ref_exists;
use crate::store::turns::{Boundary, Turn, TurnOutcome};

async fn wait_for_turn(f: &Fixture, lane_id: i64, turn: u32, ready: impl Fn(&Turn) -> bool) {
    for _ in 0..100 {
        settle().await;
        let current = f.service.store.lock().turn(lane_id, turn);
        if let Ok(current) = current
            && ready(&current)
        {
            return;
        }
    }
    panic!("turn {turn} did not reach the expected state");
}

async fn complete(f: &Fixture, lane_id: i64, turn: u32, key: &str, text: &str, repo: &Path) {
    assert_eq!(
        f.service
            .prompt(key, text.into(), Vec::new(), PromptMode::Start)
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    settle().await;
    fs::write(repo.join(format!("agent-{turn}.txt")), "agent\n").unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    wait_for_turn(f, lane_id, turn, |turn| {
        turn.outcome == Some(TurnOutcome::Done)
    })
    .await;
}

#[tokio::test]
async fn an_immediate_turn_keeps_exact_before_and_after_trees() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    let repo = PathBuf::from(&lane.worktrees[0].path);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let index = fs::read(repo.join(".git/index")).unwrap();
    let stash = git(&repo, &["stash", "list"]);
    let config = git(&repo, &["config", "--local", "--list"]);

    assert_eq!(
        f.service
            .prompt(
                &opened.key,
                "Fix login\nmore detail".into(),
                Vec::new(),
                PromptMode::Start,
            )
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    let turn = f.service.store.lock().turn(lane.id, 1).unwrap();
    assert_eq!(turn.prompt, "Fix login");
    assert_eq!(turn.worktrees[0].boundary, Boundary::Exact);
    let before = turn.worktrees[0].before_tree.clone().unwrap();
    let before_ref = turn.worktrees[0].before_ref.clone();
    let after_ref = turn.worktrees[0].after_ref.clone();
    assert!(ref_exists(&repo, &before_ref).unwrap());

    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    settle().await;
    fs::write(repo.join("agent.txt"), "agent\n").unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.outcome == Some(TurnOutcome::Done)
    })
    .await;

    let turn = f.service.store.lock().turn(lane.id, 1).unwrap();
    let after = turn.worktrees[0].after_tree.clone().unwrap();
    assert_ne!(before, after);
    assert!(!turn.worktrees[0].recovered);
    assert!(ref_exists(&repo, &after_ref).unwrap());
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(git(&repo, &["stash", "list"]), stash);
    assert_eq!(git(&repo, &["config", "--local", "--list"]), config);
}

#[tokio::test]
async fn a_queued_turn_refreshes_its_boundary_on_run_started() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    let repo = PathBuf::from(&lane.worktrees[0].path);
    assert_eq!(
        f.service
            .prompt(
                &opened.key,
                "queue".into(),
                Vec::new(),
                PromptMode::FollowUp,
            )
            .await
            .unwrap(),
        PromptOutcome::Queued
    );
    let queued = f.service.store.lock().turn(lane.id, 1).unwrap();
    assert!(queued.queued);
    assert_eq!(queued.worktrees[0].boundary, Boundary::Unavailable);

    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::QueueChanged {
        queued: vec![QueuedPrompt {
            id: "q1".into(),
            text: "queue".into(),
            mode: PromptMode::FollowUp,
        }],
    })
    .unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.message_id.as_deref() == Some("q1")
    })
    .await;
    sink.send(HarnessEvent::RunStarted).unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.worktrees[0].boundary == Boundary::Observed
    })
    .await;
    fs::write(repo.join("agent.txt"), "agent\n").unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.outcome == Some(TurnOutcome::Done)
    })
    .await;
    let turn = f.service.store.lock().turn(lane.id, 1).unwrap();
    assert!(turn.worktrees[0].after_tree.is_some());
}

#[tokio::test]
async fn handled_steer_failed_raw_and_preview_prompts_create_no_turn() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    assert_eq!(
        f.service
            .prompt(&first.key, "handled".into(), Vec::new(), PromptMode::Start,)
            .await
            .unwrap(),
        PromptOutcome::Handled
    );
    assert!(f.service.store.lock().turn(lane.id, 1).is_err());
    assert_eq!(
        f.service
            .prompt(&first.key, "steer".into(), Vec::new(), PromptMode::Steer,)
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    assert!(f.service.store.lock().turn(lane.id, 1).is_err());
    assert!(
        f.service
            .prompt(&first.key, "fail".into(), Vec::new(), PromptMode::Start,)
            .await
            .is_err()
    );
    assert!(f.service.store.lock().turn(lane.id, 1).is_err());

    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    assert_eq!(
        f.service
            .prompt("fake:s1", "raw".into(), Vec::new(), PromptMode::Start)
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    f.service.replace_lane_session(lane.id).await.unwrap();
    settle().await;
    f.service.preview(&first.key).await.unwrap();
    assert_eq!(
        f.service
            .prompt(&first.key, "preview".into(), Vec::new(), PromptMode::Start,)
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    assert!(
        f.service
            .store
            .lock()
            .unfinished_turns()
            .unwrap()
            .is_empty()
    );
    assert!(f.service.store.lock().turn(lane.id, 1).is_err());
}

#[tokio::test]
async fn an_exited_run_settles_its_turn_as_failed() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    f.service
        .prompt(&opened.key, "Work".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    settle().await;
    sink.send(HarnessEvent::Exited {
        code: Some(7),
        stderr_tail: "crashed".into(),
    })
    .unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.outcome == Some(TurnOutcome::Failed)
    })
    .await;
    let turn = f.service.store.lock().turn(lane.id, 1).unwrap();
    assert!(turn.worktrees[0].after_tree.is_some());
}

#[tokio::test]
async fn human_edits_between_turns_are_flagged() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    let repo = PathBuf::from(&lane.worktrees[0].path);
    complete(&f, lane.id, 1, &opened.key, "First", &repo).await;
    fs::write(repo.join("human.txt"), "human\n").unwrap();
    assert_eq!(
        f.service
            .prompt(&opened.key, "Second".into(), Vec::new(), PromptMode::Start,)
            .await
            .unwrap(),
        PromptOutcome::Started
    );
    let second = f.service.store.lock().turn(lane.id, 2).unwrap();
    assert_eq!(second.worktrees[0].boundary, Boundary::Exact);
    assert!(second.worktrees[0].human_edits_before);
}

#[tokio::test]
async fn startup_reconciles_an_interrupted_turn_and_removes_orphan_refs() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    let repo = PathBuf::from(&lane.worktrees[0].path);
    f.service
        .prompt(
            &opened.key,
            "Interrupted".into(),
            Vec::new(),
            PromptMode::Start,
        )
        .await
        .unwrap();
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    wait_for_turn(&f, lane.id, 1, |turn| {
        turn.worktrees[0].boundary == Boundary::Exact
    })
    .await;
    let orphan = format!("refs/panoptes/lanes/{}/turns/orphan", lane.name);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["update-ref", &orphan, &head]);
    assert!(ref_exists(&repo, &orphan).unwrap());

    f.service.reconcile_turns().await.unwrap();
    let turn = f.service.store.lock().turn(lane.id, 1).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Stopped));
    assert!(turn.worktrees[0].after_tree.is_some());
    assert!(turn.worktrees[0].recovered);
    assert!(!ref_exists(&repo, &orphan).unwrap());
    assert!(
        f.service
            .store
            .lock()
            .unfinished_turns()
            .unwrap()
            .is_empty()
    );
}
