use super::super::*;
use super::fake::*;
use std::time::Duration;

fn edit(id: &str, path: &str) -> HarnessEvent {
    HarnessEvent::ToolCallUpdated {
        call: Box::new(panoptes_harness::ToolCall {
            id: id.into(),
            name: "edit".into(),
            kind: panoptes_harness::ToolKind::Edit,
            subject: Some(path.into()),
            input: serde_json::Value::Null,
            status: panoptes_harness::ToolStatus::Done,
            output: None,
            images: Vec::new(),
            started_at: None,
            ended_at: None,
        }),
    }
}

fn entry(listing: &Listing, lane: i64) -> &LaneNavigation {
    listing
        .navigation
        .iter()
        .find(|entry| entry.lane_id == lane)
        .unwrap()
}

/// Lists until `done` holds, for what the forwarder records on its own.
async fn eventually(f: &Fixture, done: impl Fn(&Listing) -> bool) -> Listing {
    for _ in 0..100 {
        let listing = f.service.list().await;
        if done(&listing) {
            return listing;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never happened: {:?}", f.service.list().await.navigation);
}

#[tokio::test]
async fn a_session_editing_another_lane_holds_back_that_lanes_own_session() {
    let f = fixture(FakeHarness::default());
    let (_code, a, b, sink) = two_lanes(&f).await;
    let b_path = PathBuf::from(&b.worktrees[0].path);

    let listing = f.service.list().await;
    assert_eq!(
        entry(&listing, b.id).work,
        Some(LaneWork {
            changed_files: 0,
            commits_ahead: Some(0)
        })
    );
    assert!(entry(&listing, b.id).busy_writer.is_none());

    sink.send(HarnessEvent::RunStarted).unwrap();
    // Its own worktree is not another lane's.
    sink.send(edit("t1", "own.txt")).unwrap();
    std::fs::write(b_path.join("parser.rs"), "fn parse() {}\n").unwrap();
    sink.send(edit("t2", &b_path.join("parser.rs").to_string_lossy()))
        .unwrap();
    let listing = eventually(&f, |l| entry(l, b.id).busy_writer.is_some()).await;
    let writer = entry(&listing, b.id).busy_writer.clone().unwrap();
    assert_eq!(writer.session_key, "panoptes:1");
    assert_eq!(writer.title, "New session");
    assert!(entry(&listing, a.id).busy_writer.is_none());
    assert_eq!(entry(&listing, b.id).work.unwrap().changed_files, 1);
    assert!(
        f.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.event == SessionUpdate::LaneWrites(LaneWritesChanged { lane_id: b.id }))
    );

    let refused = f.service.create(b.id, "fake").await.unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Rejected);

    // Once the writer is Done, the lane can have its own session.
    sink.send(HarnessEvent::RunSettled { aborted: false })
        .unwrap();
    let listing = eventually(&f, |l| entry(l, b.id).busy_writer.is_none()).await;
    assert_eq!(entry(&listing, b.id).work.unwrap().changed_files, 1);
    assert_eq!(
        f.service.create(b.id, "fake").await.unwrap().key,
        "panoptes:2"
    );
}

#[tokio::test]
async fn a_writer_that_stops_no_longer_holds_a_lane_back() {
    let f = fixture(FakeHarness::default());
    let (_code, _a, b, sink) = two_lanes(&f).await;
    let b_path = PathBuf::from(&b.worktrees[0].path);
    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(edit("t1", &b_path.join("x.rs").to_string_lossy()))
        .unwrap();
    eventually(&f, |l| entry(l, b.id).busy_writer.is_some()).await;
    f.service.close("panoptes:1").await;
    eventually(&f, |l| entry(l, b.id).busy_writer.is_none()).await;
    assert!(f.service.registry.lane_writers().is_empty());
}
