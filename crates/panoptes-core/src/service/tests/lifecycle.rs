use super::super::*;
use super::fake::*;
use panoptes_harness::{HarnessError, SessionSettings};
use std::time::Duration;

#[tokio::test]
async fn concurrent_opens_start_one_process() {
    let f = fixture(FakeHarness::default());
    let open = |service: Arc<Service>| async move {
        service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
    };
    let (a, b) = tokio::join!(open(f.service.clone()), open(f.service.clone()));
    assert_eq!(a.unwrap().key, "fake:s1");
    assert_eq!(b.unwrap().key, "fake:s1");
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_reopen_waits_for_the_close_before_it() {
    let f = fixture(FakeHarness::default());
    let service = f.service.clone();
    service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    let (_, reopened) = tokio::join!(
        service.close("fake:s1"),
        service.open("fake", "fake:s1", "s1".into(), "/tmp".into())
    );
    reopened.unwrap();
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 2);
    assert_eq!(f.harness.live.most.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_close_during_a_start_stops_that_process() {
    let f = fixture(FakeHarness::default());
    let service = f.service.clone();
    let opening = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .open("fake", "fake:s1", "s1".into(), "/tmp".into())
                .await
        }
    });
    // The harness is still starting the process.
    tokio::time::sleep(Duration::from_millis(10)).await;
    service.close("fake:s1").await;
    opening.await.unwrap().unwrap();
    assert!(!service.list().await.sessions[0].open);
    assert_eq!(f.harness.live.now.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_starts_never_overlap() {
    let f = fixture(FakeHarness {
        dies_on_start: true,
        ..FakeHarness::default()
    });
    let open = |service: Arc<Service>| async move {
        service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
            .await
    };
    let s = f.service.clone();
    let (a, b, c) = tokio::join!(open(s.clone()), open(s.clone()), open(s.clone()));
    assert!(a.is_err() && b.is_err() && c.is_err());
    assert_eq!(f.harness.live.most.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn adopting_waits_for_an_open_of_the_same_conversation() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let service = f.service.clone();
    let opening = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .open("fake", "fake:s1", "s1".into(), "/tmp".into())
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    // The open got there first, so adoption is refused rather than
    // leaving its process running beside the lane's.
    let adopted = service
        .adopt("fake", "fake:s1", "s1".into(), "/tmp".into(), Some(lane.id))
        .await;
    opening.await.unwrap().unwrap();
    assert!(adopted.is_err());
}

#[tokio::test]
async fn a_lane_cannot_be_archived_while_its_session_starts() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let service = f.service.clone();
    let creating = tokio::spawn({
        let service = service.clone();
        async move { service.create(lane.id, "fake").await }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(service.archive_lane(lane.id, None).await.is_err());
    assert!(service.delete_lane(lane.id).await.is_err());
    creating.await.unwrap().unwrap();
}

/// A registered repository with one main-checkout lane, which needs no
/// worktree on disk.
#[tokio::test]
async fn closing_a_session_does_not_report_an_exit() {
    let f = fixture(FakeHarness::default());
    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    f.service.close("fake:s1").await;
    settle().await;
    assert_eq!(exits(&f), 0);
    assert!(!f.service.list().await.sessions[0].open);
}

#[tokio::test]
async fn a_process_that_dies_while_starting_is_not_left_open() {
    let f = fixture(FakeHarness {
        dies_on_start: true,
        ..FakeHarness::default()
    });
    let error = f
        .service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Exited);
    assert!(error.message.contains("no model configured"), "{error}");
    assert!(!f.service.list().await.sessions[0].open);
    let error = f.service.history("fake:s1").await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotOpen);
}

#[tokio::test]
async fn a_crash_is_reported_and_closes_the_session() {
    let f = fixture(FakeHarness::default());
    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    assert!(f.service.list().await.sessions[0].open);

    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::Exited {
        code: Some(1),
        stderr_tail: "boom".into(),
    })
    .unwrap();
    settle().await;
    assert_eq!(exits(&f), 1);
    assert_eq!(statuses(&f).last(), Some(&SessionStatus::Failed));
    assert!(!f.service.list().await.sessions[0].open);
    let error = f.service.history("fake:s1").await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::NotOpen);
}

#[tokio::test]
async fn a_run_goes_from_running_through_waiting_to_done() {
    let f = fixture(FakeHarness::default());
    let sink = open_s1(&f).await;
    settle().await;
    assert_eq!(statuses(&f), [SessionStatus::Idle]);

    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(question("q1", None)).unwrap();
    settle().await;
    f.service
        .answer("fake:s1", "q1".into(), Answer::Cancelled)
        .await
        .unwrap();
    sink.send(assistant(None)).unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    settle().await;
    assert_eq!(
        statuses(&f),
        [
            SessionStatus::Idle,
            SessionStatus::Running,
            SessionStatus::Waiting,
            SessionStatus::Running,
            SessionStatus::Done,
        ]
    );
}

#[tokio::test]
async fn a_run_that_ends_on_an_error_fails() {
    let f = fixture(FakeHarness::default());
    let sink = open_s1(&f).await;
    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(assistant(Some("overloaded"))).unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    settle().await;
    assert_eq!(statuses(&f).last(), Some(&SessionStatus::Failed));

    // The next run starts clean, and a retried error does not count.
    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(assistant(Some("overloaded"))).unwrap();
    sink.send(assistant(None)).unwrap();
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    settle().await;
    assert_eq!(
        statuses(&f)[3..],
        [SessionStatus::Running, SessionStatus::Done]
    );
}

#[tokio::test]
async fn reopening_an_open_session_brings_back_its_status_and_questions() {
    let f = fixture(FakeHarness::default());
    let open = || {
        f.service
            .open("fake", "fake:s1", "s1".into(), "/tmp".into())
    };
    let first = open().await.unwrap();
    assert_eq!(first.status, SessionStatus::Idle);
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(question("q1", None)).unwrap();
    settle().await;

    // A reloaded window opens it again and gets the same process.
    let again = open().await.unwrap();
    assert_eq!(f.harness.starts.load(Ordering::SeqCst), 1);
    assert_eq!(again.status, SessionStatus::Waiting);
    assert_eq!(
        again.replay,
        vec![HarnessEvent::RunStarted, question("q1", None)]
    );
}

#[test]
fn status_events_serialise_like_harness_events() {
    let update = SessionUpdate::Status(StatusChanged {
        status: SessionStatus::Waiting,
    });
    assert_eq!(
        serde_json::to_value(update).unwrap(),
        serde_json::json!({"type": "statusChanged", "status": "waiting"})
    );
    let update = SessionUpdate::Harness(HarnessEvent::RunStarted);
    assert_eq!(
        serde_json::to_value(update).unwrap(),
        serde_json::json!({"type": "runStarted"})
    );
}

#[test]
fn harness_errors_keep_their_kind() {
    let error: ServiceError = anyhow::Error::from(HarnessError::timed_out("slow"))
        .context("Could not prompt")
        .into();
    assert_eq!(error.kind, ErrorKind::TimedOut);
    assert_eq!(error.message, "Could not prompt: slow");
    let error: ServiceError = anyhow::Error::from(HarnessError::not_running("gone")).into();
    assert_eq!(error.kind, ErrorKind::NotOpen);
    let error: ServiceError = anyhow::anyhow!("disk full").into();
    assert_eq!(error.kind, ErrorKind::Other);
    assert_eq!(
        serde_json::to_value(ServiceError::new(ErrorKind::NotOpen, "x")).unwrap(),
        serde_json::json!({"kind": "notOpen", "message": "x"})
    );
}

#[tokio::test]
async fn a_failing_harness_is_reported_not_fatal() {
    let f = fixture(FakeHarness {
        fail_listing: true,
        ..Default::default()
    });
    let listing = f.service.list().await;
    assert!(listing.sessions.is_empty());
    assert_eq!(
        listing.errors,
        ["Could not list fake sessions: disk on fire"]
    );
}

#[tokio::test]
async fn unknown_harnesses_and_sessions_are_errors() {
    let f = fixture(FakeHarness::default());
    fn kind<T: std::fmt::Debug>(result: ServiceResult<T>) -> ErrorKind {
        result.unwrap_err().kind
    }
    assert_eq!(
        kind(f.service.create(1, "nope").await),
        ErrorKind::Unavailable
    );
    assert_eq!(
        kind(f.service.abort("fake:missing").await),
        ErrorKind::NotOpen
    );
    assert_eq!(
        kind(
            f.service
                .answer("fake:missing", "q".into(), Answer::Cancelled)
                .await
        ),
        ErrorKind::NotOpen
    );
}

#[tokio::test]
async fn a_harness_without_models_reports_no_settings() {
    let f = fixture(FakeHarness::default());
    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    let settings = f.service.settings("fake:s1").await.unwrap();
    assert_eq!(settings, SessionSettings::default());
    assert!(f.service.models("fake:s1").await.unwrap().is_empty());
    let error = f
        .service
        .set_thinking_level("fake:s1", "high".into())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ServiceError::new(ErrorKind::Rejected, "This harness has no thinking levels")
    );
}
