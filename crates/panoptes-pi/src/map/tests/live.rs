use super::*;

#[test]
fn streams_text_under_one_live_id() {
    let mut mapper = EventMapper::default();
    let started = mapper.map(json!({"type":"message_start","message":{"role":"assistant","content":[],"stopReason":"pending"}}), 0);
    assert!(
        matches!(&started[0], HarnessEvent::MessageStarted { message_id, role: Role::Assistant } if message_id == "live-1")
    );
    let delta = mapper.map(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi"}}), 0);
    assert!(
        matches!(&delta[0], HarnessEvent::BlockDelta { message_id, delta, .. } if message_id == "live-1" && delta == "Hi")
    );
    let done = mapper.map(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"stopReason":"stop"}}), 0);
    assert!(
        matches!(&done[0], HarnessEvent::MessageCompleted { message } if message.id == "live-1" && message.error.is_none())
    );
}

#[test]
fn a_finished_message_carries_its_usage_and_end_time() {
    let mut mapper = EventMapper::default();
    mapper.map(
        json!({"type":"message_start","message":priced("pending", 1000)}),
        1000,
    );
    let done = mapper.map(
        json!({"type":"message_end","message":priced("length", 1000)}),
        4500,
    );
    let [HarnessEvent::MessageCompleted { message }] = done.as_slice() else {
        panic!("expected a completed message: {done:?}");
    };
    assert_eq!(
        message.usage,
        Some(Box::new(Usage {
            model: Some("claude-sonnet-4-5".into()),
            provider: Some("anthropic".into()),
            input_tokens: 1200,
            output_tokens: 340,
            cache_read_tokens: 9000,
            cache_write_tokens: 50,
            cost: Some(0.0116),
            stop_reason: Some(StopReason::MaxTokens),
            ended_at: Some(4500),
        }))
    );
}

#[test]
fn only_assistant_messages_have_usage() {
    let user = message(
        &json!({"role":"user","content":"hi","timestamp":1}),
        "u".into(),
        Some(2),
    );
    assert_eq!(user.unwrap().usage, None);
    // A message without usage still says why it stopped.
    let bare = message(
        &json!({"role":"assistant","content":[],"stopReason":"toolUse"}),
        "a".into(),
        None,
    )
    .unwrap();
    let usage = bare.usage.unwrap();
    assert_eq!(usage.stop_reason, Some(StopReason::ToolUse));
    assert_eq!(
        (usage.input_tokens, usage.cost, usage.model),
        (0, None, None)
    );
}

#[test]
fn messages_keep_their_ids_even_when_role_and_time_collide() {
    let shared = Shared::default();
    let mut mapper = EventMapper::new(shared.clone());
    let text = |text: &str| json!([{"type":"text","text":text}]);
    let messages = [
        // Two user messages in the same millisecond.
        json!({"role":"user","content":text("one"),"timestamp":5}),
        json!({"role":"user","content":text("two"),"timestamp":5}),
        // Messages Pi did not stamp at all.
        json!({"role":"assistant","content":text("three")}),
        json!({"role":"assistant","content":text("four")}),
    ];
    let live: Vec<String> = messages
        .iter()
        .map(|message| stream(&mut mapper, message.clone()))
        .collect();
    let entries: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(i, message)| {
            let parent = (i > 0).then(|| format!("e{}", i - 1));
            json!({"type":"message","id":format!("e{i}"),"parentId":parent,"message":message})
        })
        .collect();
    let reloaded: Vec<String> = history(&entries, Some("e3"), &shared)
        .into_iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(reloaded, live);
    // Reloading again gives the same answer.
    let again: Vec<String> = history(&entries, Some("e3"), &shared)
        .into_iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(again, live);
}

#[test]
fn a_call_with_no_end_is_closed_when_its_run_settles() {
    let (mut mapper, shared) = started();
    tag(&mut mapper, "c1");
    mapper.ui_request(&json!({"id":"q1","method":"confirm","title":"Run?"}), 0);
    assert_eq!(shared.approvals.lock().unwrap().len(), 1);

    let events = mapper.map(json!({"type":"agent_settled","aborted":true}), 4_000);
    let [
        HarnessEvent::ToolCallUpdated { call },
        HarnessEvent::RunSettled { aborted: true },
    ] = events.as_slice()
    else {
        panic!("unexpected events: {events:?}")
    };
    assert_eq!(
        (call.status, call.started_at, call.ended_at),
        (ToolStatus::Failed, Some(0), Some(4_000))
    );
    assert!(shared.approvals.lock().unwrap().is_empty());
    // A second settle has nothing left to close.
    assert_eq!(
        mapper
            .map(json!({"type":"agent_settled","aborted":false}), 5_000)
            .len(),
        1
    );

    // The history, which has no result for the call, agrees.
    let entries = vec![
        json!({"type":"message","id":"a","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"rm -rf x"}}],"stopReason":"aborted"}}),
    ];
    let messages = history(&entries, Some("a"), &shared);
    let Block::ToolCall(reloaded) = &messages[0].blocks[0] else {
        unreachable!()
    };
    assert_eq!(
        (reloaded.status, reloaded.ended_at),
        (ToolStatus::Failed, Some(4_000))
    );
}

#[test]
fn live_tool_events_carry_the_whole_call() {
    let mut mapper = EventMapper::default();
    let events = live_call(&mut mapper);
    let calls: Vec<&ToolCall> = events
        .iter()
        .filter_map(|event| match event {
            HarnessEvent::ToolCallUpdated { call } => Some(call.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls.iter().map(|c| c.status).collect::<Vec<_>>(),
        [ToolStatus::Running, ToolStatus::Running, ToolStatus::Failed]
    );
    assert!(calls.iter().all(|c| c.kind == ToolKind::Execute
        && c.subject.as_deref() == Some("false")
        && c.input["command"] == "false"));
    let last = calls[2];
    assert_eq!((last.started_at, last.ended_at), (Some(1_000), Some(3_000)));
    assert_eq!(
        last.output,
        Some(ToolOutput::Shell {
            text: "out".into(),
            exit_code: Some(1)
        })
    );
    // The result message is folded into the call, not sent on its own.
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, HarnessEvent::MessageCompleted { .. }))
            .count(),
        1
    );
}

#[test]
fn aborted_messages_are_stopped_not_failed() {
    let stopped = message(
        &json!({"role":"assistant","content":[],"stopReason":"aborted"}),
        "m".into(),
        None,
    )
    .unwrap();
    assert!(stopped.stopped && stopped.error.is_none());
}

#[test]
fn hides_system_messages() {
    assert!(message(&json!({"role":"system","content":""}), "s".into(), None).is_none());
}
