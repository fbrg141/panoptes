use super::*;

#[test]
fn history_ends_a_message_when_pi_wrote_it() {
    let entries = vec![
        json!({"type":"message","id":"a","parentId":null,"timestamp":"2026-09-07T16:09:44.548Z","message":{"role":"user","content":"go","timestamp":1788797384548_i64}}),
        json!({"type":"message","id":"b","parentId":"a","timestamp":"2026-09-07T16:09:46.140Z","message":priced("stop", 1788797384611)}),
    ];
    let history = history(&entries, Some("b"), &SessionState::default());
    let usage = history[1].usage.as_ref().unwrap();
    assert_eq!(usage.ended_at, Some(1788797386140));
    assert_eq!(usage.stop_reason, Some(StopReason::Finished));
}

#[test]
fn history_keeps_the_end_time_seen_live() {
    let shared: Shared = Arc::default();
    let mut mapper = EventMapper::new(shared.clone());
    mapper.map(
        json!({"type":"message_start","message":priced("pending", 1000)}),
        1000,
    );
    mapper.map(
        json!({"type":"message_end","message":priced("stop", 1000)}),
        4500,
    );
    let entries = vec![
        json!({"type":"message","id":"b","parentId":null,"timestamp":"2026-09-07T16:09:46.140Z","message":priced("stop", 1000)}),
    ];
    let history = history(&entries, Some("b"), &shared);
    assert_eq!(history[0].id, "live-1");
    assert_eq!(history[0].usage.as_ref().unwrap().ended_at, Some(4500));
}

#[test]
fn reads_javascript_iso_times() {
    assert_eq!(iso_millis("1970-01-01T00:00:00.000Z"), Some(0));
    assert_eq!(
        iso_millis("2024-02-29T23:59:59.999Z"),
        Some(1_709_251_199_999)
    );
    assert_eq!(
        iso_millis("2026-09-07T16:09:46.140Z"),
        Some(1_788_797_386_140)
    );
    assert_eq!(iso_millis("2026-09-07T16:09:46"), None);
    assert_eq!(iso_millis("yesterday"), None);
}

#[test]
fn history_follows_the_active_branch() {
    let entries = vec![
        json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"one"}}),
        json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"two"}]}}),
        json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"abandoned"}]}}),
        json!({"type":"session_info","id":"d","parentId":"b","name":"x"}),
    ];
    let ids: Vec<_> = history(&entries, Some("d"), &SessionState::default())
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

#[test]
fn history_gives_the_same_call_as_the_live_events() {
    let shared = Shared::default();
    let mut mapper = EventMapper::new(shared.clone());
    let events = live_call(&mut mapper);
    let Some(HarnessEvent::ToolCallUpdated { call: live }) = events
        .iter()
        .rfind(|e| matches!(e, HarnessEvent::ToolCallUpdated { .. }))
    else {
        panic!("no tool call in {events:?}");
    };
    let entries = vec![
        json!({"type":"message","id":"a","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"false"}}],"stopReason":"toolUse"}}),
        json!({"type":"message","id":"b","parentId":"a","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[{"type":"text","text":"out\n\nCommand exited with code 1"}],"isError":true,"timestamp":9_999}}),
    ];
    let messages = history(&entries, Some("b"), &shared);
    assert_eq!(messages.len(), 1, "the result is not a message of its own");
    assert_eq!(messages[0].blocks, vec![Block::ToolCall(live.clone())]);

    // Without live times (a session opened later), the result's own
    // timestamp is the end.
    let cold = history(&entries, Some("b"), &SessionState::default());
    let Block::ToolCall(call) = &cold[0].blocks[0] else {
        unreachable!()
    };
    assert_eq!(
        (call.status, call.started_at, call.ended_at),
        (ToolStatus::Failed, None, Some(9_999))
    );
}
