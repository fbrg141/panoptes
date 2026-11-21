use super::super::*;
use super::fake::*;
use crate::service::key::panoptes_id;
use std::time::Duration;

#[tokio::test]
async fn lane_changes_and_file_hunks_come_from_the_lane_tree() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    std::fs::write(
        Path::new(&lane.worktrees[0].path).join("untracked.txt"),
        "review me\n",
    )
    .unwrap();
    let changes = f.service.lane_changes(lane.id).await.unwrap();
    assert_eq!(changes.files.len(), 1);
    assert_eq!(changes.files[0].path, "untracked.txt");
    let range = TreeRange {
        from: changes.from,
        to: changes.to,
    };
    let hunks = f
        .service
        .file_hunks(lane.id, "untracked.txt".into(), range)
        .await
        .unwrap();
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].lines[0].text, "review me");
}

#[tokio::test]
async fn a_lane_session_has_its_key_before_the_harness_starts() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    assert_eq!(opened.key, "panoptes:1");
    settle().await;
    {
        // Nothing is held back: even what the harness says while it
        // starts carries the Panoptes key.
        let events = f.events.lock().unwrap();
        assert!(events.iter().all(|e| e.key == opened.key), "{events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e.event, SessionUpdate::Harness(HarnessEvent::Raw { .. })))
        );
    }
    let listing = f.service.list().await;
    let session = listing
        .sessions
        .iter()
        .find(|s| s.key == opened.key)
        .unwrap();
    assert_eq!(session.id, Some(1));
    assert_eq!(session.lane_id, Some(lane.id));
    assert_eq!(session.project_id, Some(lane.project_id));
    assert_eq!(session.cwd, lane.worktrees[0].path);
    assert_eq!(session.title, "New session");
    assert_eq!(session.harness_ref.as_ref().unwrap().id, "new-1");
    assert!(session.open);

    // Its conversation, opened by the harness key, is the same session.
    let again = f
        .service
        .open("fake", "fake:new-1", "new-1".into(), "/tmp".into())
        .await
        .unwrap();
    assert_eq!(again.key, opened.key);
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_new_session_keeps_profile_instructions_across_reopen() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    f.service
        .create_with(lane.id, "fake", Some("Prefer small changes.".into()))
        .await
        .unwrap();
    assert_eq!(
        f.harness.requests.lock().unwrap()[0]
            .instructions
            .as_deref(),
        Some("Prefer small changes.")
    );

    let opened = f.service.list().await.sessions[0].clone();
    f.service.close(&opened.key).await;
    f.service
        .open("fake", &opened.key, String::new(), String::new())
        .await
        .unwrap();
    assert_eq!(
        f.harness.requests.lock().unwrap()[1]
            .instructions
            .as_deref(),
        Some("Prefer small changes.")
    );
}

#[tokio::test]
async fn an_open_raw_session_must_stop_before_adoption() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    let error = f
        .service
        .adopt("fake", "fake:s1", "s1".into(), "/tmp".into(), Some(lane.id))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert!(error.message.contains("Stop this session"));
}

#[tokio::test]
async fn a_lane_keeps_session_history_with_only_one_live_process() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let key = f.service.create(lane.id, "fake").await.unwrap().key;
    let error = f.service.create(lane.id, "fake").await.unwrap_err();
    assert!(error.message.contains("already open"));
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);

    f.service.set_archived(&key, true).await.unwrap();
    f.service.create(lane.id, "fake").await.unwrap();
    f.service
        .adopt(
            "fake",
            "fake:s2",
            "s2".into(),
            lane.worktrees[0].path.clone(),
            Some(lane.id),
        )
        .await
        .unwrap();
    let listing = f.service.list().await;
    assert_eq!(listing.navigation[0].sessions.len(), 2);
    assert_eq!(listing.navigation[0].archived_sessions.len(), 1);
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_allow_in_a_lane_is_remembered_for_its_next_session() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::Approval {
        approval: panoptes_harness::Approval {
            id: "a1".into(),
            tool_call_id: "c1".into(),
            title: "Allow?".into(),
            message: None,
            subject: Some("execute: rm -rf build".into()),
            expires_at: None,
        },
    })
    .unwrap();
    settle().await;
    f.service
        .answer(&first.key, "a1".into(), Answer::Decision { allow: true })
        .await
        .unwrap();

    // The allow is Panoptes' now, kept for the lane: Pi's own memory goes
    // with its process.
    let store = Store::open(f._dir.path()).unwrap();
    assert_eq!(
        store
            .allowed_actions(&crate::store::lane_scope(lane.id))
            .unwrap(),
        ["execute: rm -rf build"]
    );
    drop(store);

    // A session started in the lane later starts already knowing it.
    f.service.replace_lane_session(lane.id).await.unwrap();
    let requests = f.harness.requests.lock().unwrap().clone();
    assert_eq!(
        requests.last().unwrap().allowed,
        ["execute: rm -rf build".to_string()]
    );
}

#[tokio::test]
async fn replacing_a_lane_session_starts_a_fresh_record_in_the_same_worktree() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    let second = f.service.replace_lane_session(lane.id).await.unwrap();

    assert_ne!(first.key, second.key);
    let requests = f.harness.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.resume.is_none()));
    assert!(
        requests
            .iter()
            .all(|request| request.cwd == lane.worktrees[0].path)
    );
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
    drop(requests);
    let owned = f
        .service
        .list()
        .await
        .sessions
        .into_iter()
        .filter(|session| session.lane_id == Some(lane.id))
        .collect::<Vec<_>>();
    assert_eq!(owned.len(), 2);
    assert_eq!(owned.iter().filter(|session| session.open).count(), 1);
}

#[tokio::test]
async fn resuming_a_lane_session_uses_exactly_that_records_locator() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    let first_id = panoptes_id(&first.key).unwrap();
    let second = f.service.replace_lane_session(lane.id).await.unwrap();

    let resumed = f
        .service
        .resume_lane_session(lane.id, first_id)
        .await
        .unwrap();
    assert_eq!(resumed.key, first.key);
    assert_ne!(resumed.key, second.key);
    let requests = f.harness.requests.lock().unwrap().clone();
    assert_eq!(requests.last().unwrap().resume.as_deref(), Some("new-1"));
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn previewing_an_earlier_session_keeps_the_live_one_until_resume() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    let first_id = panoptes_id(&first.key).unwrap();
    let second = f.service.replace_lane_session(lane.id).await.unwrap();

    let preview = f.service.preview(&first.key).await.unwrap();
    assert!(preview.preview);
    assert_eq!(preview.history, first.history);
    assert!(f.service.history(&second.key).await.is_ok());
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 2);
    assert_eq!(f.harness.read_only(), [false, false, true]);

    let resumed = f
        .service
        .resume_lane_session(lane.id, first_id)
        .await
        .unwrap();
    assert!(!resumed.preview);
    assert!(f.service.history(&second.key).await.is_err());
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
    assert_eq!(f.harness.read_only(), [false, false, true, false]);
}

#[tokio::test]
async fn a_preview_does_not_hold_its_lane() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    let second = f.service.replace_lane_session(lane.id).await.unwrap();
    f.service.preview(&first.key).await.unwrap();

    // Stopping the live session and opening it again keeps the preview.
    f.service.close(&second.key).await;
    let reopened = f
        .service
        .open("fake", &second.key, String::new(), String::new())
        .await
        .unwrap();
    assert!(!reopened.preview);
    assert!(f.service.history(&first.key).await.is_ok());

    // Archiving needs only the live session closed; the preview goes too.
    f.service.close(&second.key).await;
    f.service.archive_lane(lane.id, None).await.unwrap();
    assert!(f.service.history(&first.key).await.is_err());
}

#[tokio::test]
async fn an_archived_lane_session_cannot_be_resumed() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let first = f.service.create(lane.id, "fake").await.unwrap();
    f.service.replace_lane_session(lane.id).await.unwrap();
    f.service.set_archived(&first.key, true).await.unwrap();

    let error = f
        .service
        .resume_lane_session(lane.id, panoptes_id(&first.key).unwrap())
        .await
        .unwrap_err();
    assert!(error.message.contains("archived"));
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 2);
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn changing_lane_sessions_is_blocked_until_the_current_one_is_idle() {
    for event in [
        HarnessEvent::RunStarted,
        HarnessEvent::QueueChanged {
            queued: vec![panoptes_harness::QueuedPrompt {
                id: "q1".into(),
                text: "later".into(),
                mode: PromptMode::FollowUp,
            }],
        },
        question("q1", None),
    ] {
        let f = fixture(FakeHarness::default());
        let (_code, lane) = lane_of(&f).await;
        f.service.create(lane.id, "fake").await.unwrap();
        let sink = f.harness.sinks.lock().unwrap()[0].clone();
        sink.send(event).unwrap();
        settle().await;
        assert!(f.service.replace_lane_session(lane.id).await.is_err());
        assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
        assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn an_archived_lane_reopens_its_existing_session_read_only() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let key = f.service.create(lane.id, "fake").await.unwrap().key;
    f.service.close(&key).await;
    f.service.archive_lane(lane.id, None).await.unwrap();
    assert!(f.service.create(lane.id, "fake").await.is_err());

    f.service
        .open("fake", &key, String::new(), String::new())
        .await
        .unwrap();
    assert_eq!(f.harness.read_only(), [false, true]);
}

#[tokio::test]
async fn a_detached_lane_waits_for_recovery_before_its_store_record_changes() {
    let code = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let lane = f
        .service
        .create_lane(project.id, "Detached work".into(), false)
        .await
        .unwrap();
    let path = PathBuf::from(&lane.worktrees[0].path);
    git(&path, &["checkout", "--quiet", "--detach"]);
    std::fs::write(path.join("work.txt"), "keep me").unwrap();
    git(&path, &["add", "work.txt"]);
    git(&path, &["commit", "--quiet", "-m", "detached work"]);

    let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
    let ArchiveLaneOutcome::DetachedHead { detached_head, .. } = outcome else {
        panic!("a detached HEAD should need recovery");
    };
    assert_eq!(detached_head.unreachable_commits.len(), 1);
    assert_eq!(f.service.list().await.lanes[0].state, LaneState::Active);
    assert!(path.exists());

    f.service.create_lane_branch(lane.id).await.unwrap();
    let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
    assert!(matches!(outcome, ArchiveLaneOutcome::Archived));
    assert_eq!(f.service.list().await.lanes[0].state, LaneState::Archived);
    assert!(!path.exists());
}

/// A project with two worktree lanes, and a session started in the first.
#[tokio::test]
async fn failed_lane_setup_leaves_no_worktree_or_lane_record() {
    let code = tempfile::tempdir().unwrap();
    let repo = code.path().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let mut settings = project.settings.clone();
    settings.setup = Some("exit 9".into());
    f.service
        .update_project(project.id, settings)
        .await
        .unwrap();

    let error = f
        .service
        .create_lane(project.id, "Broken setup".into(), false)
        .await
        .unwrap_err();
    assert!(error.message.contains("project setup failed"), "{error}");
    assert!(f.service.list().await.lanes.is_empty());
    assert!(!f._dir.path().join("lanes/broken-setup").exists());
}

#[tokio::test]
async fn cancelling_lane_setup_rolls_back_the_partial_worktree() {
    let code = tempfile::tempdir().unwrap();
    let repo = code.path().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let mut settings = project.settings.clone();
    settings.setup = Some("touch started; exec sleep 30".into());
    f.service
        .update_project(project.id, settings)
        .await
        .unwrap();
    let lane_path = f._dir.path().join("lanes/slow-setup/web");
    let started = lane_path.join("started");

    let creating = tokio::spawn({
        let service = f.service.clone();
        async move {
            service
                .create_lane(project.id, "Slow setup".into(), false)
                .await
        }
    });
    for _ in 0..200 {
        if started.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(started.exists(), "setup did not start");
    f.service.cancel_lane_creation(project.id);
    let error = tokio::time::timeout(Duration::from_secs(5), creating)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert!(f.service.list().await.lanes.is_empty());
    assert!(!lane_path.exists());
}

#[tokio::test]
async fn main_checkout_lanes_are_marked_and_never_remove_the_checkout() {
    let code = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(code.path()).unwrap().join("web");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "--initial-branch", "trunk"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "start"],
    );
    let f = fixture(FakeHarness::default());
    let project = f
        .service
        .add_project(repo.to_string_lossy().into_owned())
        .await
        .unwrap();
    let lane = f
        .service
        .create_lane(project.id, "Quick question".into(), true)
        .await
        .unwrap();
    assert!(lane.worktrees[0].main_checkout);
    assert_eq!(lane.worktrees[0].path, project.repo);
    let error = f
        .service
        .create_lane(project.id, "Another main".into(), true)
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("already has an active main-checkout")
    );

    std::fs::write(repo.join("uncommitted.txt"), "keep me").unwrap();
    let outcome = f.service.archive_lane(lane.id, None).await.unwrap();
    assert!(matches!(outcome, ArchiveLaneOutcome::Archived));
    assert!(repo.join("uncommitted.txt").exists());
    assert_eq!(f.service.list().await.lanes[0].state, LaneState::Archived);

    let again = f
        .service
        .create_lane(project.id, "Quick question".into(), true)
        .await
        .unwrap();
    assert_eq!(again.name, "quick-question-2");
    let error = f.service.restore_lane(lane.id).await.unwrap_err();
    assert!(
        error
            .message
            .contains("already has an active main-checkout"),
        "{error}"
    );
}

#[tokio::test]
async fn a_project_whose_repository_moved_can_still_be_cleared_away() {
    let f = fixture(FakeHarness::default());
    let (code, lane) = lane_of(&f).await;
    let repo = PathBuf::from(&lane.worktrees[0].path);
    std::fs::rename(&repo, code.path().join("moved")).unwrap();

    f.service.list().await;
    f.service.delete_lane(lane.id).await.unwrap();
    f.service.remove_project(lane.project_id).await.unwrap();
    assert!(f.service.list().await.projects.is_empty());
}

#[tokio::test]
async fn closing_a_lane_closes_idle_sessions_but_refuses_running_ones() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let opened = f.service.create(lane.id, "fake").await.unwrap();
    let sink = f.harness.sinks.lock().unwrap()[0].clone();

    sink.send(HarnessEvent::RunStarted).unwrap();
    settle().await;
    let error = f.service.close_lane_sessions(lane.id).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert!(error.message.contains("is Running"), "{error}");
    sink.send(question("q1", None)).unwrap();
    settle().await;
    let error = f.service.close_lane_sessions(lane.id).await.unwrap_err();
    assert!(error.message.contains("is Waiting"), "{error}");
    assert_eq!(exits(&f), 0, "a busy session is never stopped");

    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    settle().await;
    let closed = f.service.close_lane_sessions(lane.id).await.unwrap();
    assert_eq!(closed, std::slice::from_ref(&opened.key));
    let listing = f.service.list().await;
    let session = listing.sessions.iter().find(|s| s.key == opened.key);
    assert!(session.is_some_and(|s| !s.open));
    assert!(
        f.service
            .close_lane_sessions(lane.id)
            .await
            .unwrap()
            .is_empty()
    );
    f.service.delete_lane(lane.id).await.unwrap();
}

#[tokio::test]
async fn deleting_a_lane_requires_a_closed_session_and_frees_the_project() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let repo = PathBuf::from(&lane.worktrees[0].path);
    std::fs::write(repo.join("keep.txt"), "main checkout work").unwrap();
    let opened = f.service.create(lane.id, "fake").await.unwrap();

    let error = f.service.delete_lane(lane.id).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert!(
        error.message.contains("Close this lane's session"),
        "{error}"
    );
    assert_eq!(f.service.list().await.lanes.len(), 1);

    f.service.close(&opened.key).await;
    f.service.delete_lane(lane.id).await.unwrap();
    let listing = f.service.list().await;
    assert!(listing.lanes.is_empty());
    assert!(
        listing
            .sessions
            .iter()
            .all(|session| session.key != opened.key && session.lane_id.is_none())
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("keep.txt")).unwrap(),
        "main checkout work"
    );

    f.service.remove_project(lane.project_id).await.unwrap();
    assert!(f.service.list().await.projects.is_empty());
}
