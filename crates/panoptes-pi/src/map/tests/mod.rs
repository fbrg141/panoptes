use super::content::MAX_OUTPUT;
use super::tools::shell_output;
use super::*;
use panoptes_harness::{FileEdit, StopReason, ToolImage, ToolKind, ToolOutput, Usage};
use serde_json::json;

mod dialogs;
mod history;
mod live;
mod models;
mod tools;

/// An assistant message as Pi writes it, with usage and price.
fn priced(stop: &str, timestamp: i64) -> Value {
    json!({"role":"assistant","content":[{"type":"text","text":"Hi"}],
        "provider":"anthropic","model":"claude-sonnet-4-5",
        "usage":{"input":1200,"output":340,"cacheRead":9000,"cacheWrite":50,"totalTokens":10590,
            "cost":{"input":0.0036,"output":0.0051,"cacheRead":0.0027,"cacheWrite":0.0002,"total":0.0116}},
        "stopReason":stop,"timestamp":timestamp})
}

fn result(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

/// One whole message, as Pi streams it.
fn stream(mapper: &mut EventMapper, message: Value) -> String {
    let mut events = mapper.map(
        json!({"type":"message_start","message":{"role":message["role"],"content":[]}}),
        0,
    );
    events.extend(mapper.map(json!({"type":"message_end","message":message}), 0));
    let Some(HarnessEvent::MessageCompleted { message }) = events.pop() else {
        panic!("no completed message")
    };
    message.id
}

/// A bash call from Pi's events: message, start, update, end.
fn live_call(mapper: &mut EventMapper) -> Vec<HarnessEvent> {
    let call = json!({"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"false"}});
    let mut events = Vec::new();
    for (event, now) in [
        (
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
            0,
        ),
        (
            json!({"type":"message_end","message":{"role":"assistant","content":[call],"stopReason":"toolUse"}}),
            0,
        ),
        (
            json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"false"}}),
            1_000,
        ),
        (
            json!({"type":"tool_execution_update","toolCallId":"c1","toolName":"bash","partialResult":result("out")}),
            1_500,
        ),
        (
            json!({"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":result("out\n\nCommand exited with code 1"),"isError":true}),
            3_000,
        ),
        (
            json!({"type":"message_start","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[],"isError":true}}),
            3_000,
        ),
        (
            json!({"type":"message_end","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[],"isError":true}}),
            3_000,
        ),
    ] {
        events.extend(mapper.map(event, now));
    }
    events
}

/// A mapper with call `c1` started and its approvals map.
fn started() -> (EventMapper, Shared) {
    let shared = Shared::default();
    let mut mapper = EventMapper::new(shared.clone());
    mapper.map(
        json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"rm -rf x"}}),
        0,
    );
    (mapper, shared)
}

/// The status line `approvals.js` sends before a hook's dialog.
fn tag(mapper: &mut EventMapper, call: &str) {
    let events = mapper.ui_request(
        &json!({"id":"s","method":"setStatus","statusKey":"panoptes.approval","statusText":call}),
        0,
    );
    assert!(events.is_empty(), "the tag is not shown: {events:?}");
}
