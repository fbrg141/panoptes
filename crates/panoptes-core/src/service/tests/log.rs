use super::super::*;
use super::fake::*;
use crate::service::error::not_open;

/// Every event line in the fixture's log, in order.
fn logged(f: &Fixture) -> Vec<serde_json::Value> {
    let dir = f._dir.path().join("logs");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn a_session_leaves_a_record_without_its_words() {
    let f = fixture(FakeHarness::default());
    f.service
        .open("fake", "fake:s1", "s1".into(), "/tmp".into())
        .await
        .unwrap();
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::RunStarted).unwrap();
    sink.send(question("q1", None)).unwrap();
    settle().await;
    f.service
        .answer(
            "fake:s1",
            "q1".into(),
            Answer::Confirmed { confirmed: true },
        )
        .await
        .unwrap();
    f.service.close("fake:s1").await;
    f.service.record_error("send_prompt", &not_open());

    let lines = logged(&f);
    let events: Vec<&str> = lines
        .iter()
        .map(|line| line["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        events,
        [
            "sessionStarted",
            "status",
            "asked",
            "status",
            "answered",
            "status",
            "sessionClosed",
            "error"
        ]
    );
    assert_eq!(lines[0]["key"], "fake:s1");
    assert_eq!(lines[0]["resumed"], true);
    assert_eq!(lines[2]["kind"], "question");
    assert_eq!(lines[7]["kind"], "notOpen");
    // What the question said is not kept.
    let text = serde_json::to_string(&lines).unwrap();
    assert!(!text.contains("Proceed?"), "{text}");
    assert!(lines.iter().all(|line| line["at"].is_i64()));
}

#[tokio::test]
async fn lane_changes_and_crashes_are_recorded() {
    let f = fixture(FakeHarness::default());
    let (_code, lane) = lane_of(&f).await;
    let key = f.service.create(lane.id, "fake").await.unwrap().key;
    let sink = f.harness.sinks.lock().unwrap()[0].clone();
    sink.send(HarnessEvent::Exited {
        code: Some(2),
        stderr_tail: "boom\n".into(),
    })
    .unwrap();
    settle().await;
    f.service.archive_lane(lane.id, None).await.unwrap();
    f.service.restore_lane(lane.id).await.unwrap();
    f.service.delete_lane(lane.id).await.unwrap();

    let lines = logged(&f);
    let find = |event: &str| {
        lines
            .iter()
            .find(|line| line["event"] == event)
            .unwrap_or_else(|| panic!("no {event} in {lines:?}"))
    };
    assert_eq!(find("laneCreated")["lane"], lane.id);
    assert_eq!(find("sessionStarted")["key"], key);
    assert_eq!(find("sessionExited")["code"], 2);
    assert_eq!(find("sessionExited")["stderrTail"], "boom\n");
    assert_eq!(find("laneArchived")["lane"], lane.id);
    assert_eq!(find("laneRestored")["lane"], lane.id);
    assert_eq!(find("laneDeleted")["lane"], lane.id);
}
