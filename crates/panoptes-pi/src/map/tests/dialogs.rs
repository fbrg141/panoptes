use super::*;

#[test]
fn dialogs_become_questions_with_a_deadline() {
    let events = extension_ui(
        &json!({"type":"extension_ui_request","id":"q1","method":"select","title":"Allow?","options":["Allow","Block"],"timeout":10000}),
        1_000,
    );
    let [HarnessEvent::Question { question }] = events.as_slice() else {
        panic!("expected one question, got {events:?}");
    };
    assert_eq!(question.id, "q1");
    assert_eq!(question.expires_at, Some(11_000));
    assert_eq!(
        question.kind,
        QuestionKind::Select {
            options: vec!["Allow".into(), "Block".into()]
        }
    );
}

#[test]
fn tagged_permission_dialogs_are_approvals() {
    let (mut mapper, shared) = started();
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(
        &json!({"id":"q1","method":"select","title":"Dangerous command:\n\n  rm -rf x\n\nAllow?","options":["No","Yes"],"timeout":1000}),
        5,
    );
    let [HarnessEvent::Approval { approval }] = events.as_slice() else {
        panic!("expected an approval, got {events:?}");
    };
    assert_eq!(
        (
            approval.id.as_str(),
            approval.tool_call_id.as_str(),
            approval.expires_at
        ),
        ("q1", "c1", Some(1_005))
    );
    assert_eq!(
        shared.approvals.lock().unwrap()["q1"],
        Some(("Yes".to_string(), "No".to_string()))
    );

    // The tag names the call, which need not be the last one started.
    mapper.map(
        json!({"type":"tool_execution_start","toolCallId":"c2","toolName":"bash","args":{}}),
        0,
    );
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(&json!({"id":"q2","method":"confirm","title":"Run it?"}), 0);
    assert!(
        matches!(&events[..], [HarnessEvent::Approval { approval }] if approval.id == "q2" && approval.tool_call_id == "c1")
    );
    assert_eq!(shared.approvals.lock().unwrap()["q2"], None);
}

#[test]
fn untagged_dialogs_stay_questions() {
    // A tool asking its own yes/no question as soon as it starts.
    let (mut mapper, shared) = started();
    let events = mapper.ui_request(&json!({"id":"q1","method":"confirm","title":"Tabs?"}), 0);
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));
    let events = mapper.ui_request(
        &json!({"id":"q2","method":"select","title":"Sure?","options":["Yes","No"]}),
        0,
    );
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));
    assert!(shared.approvals.lock().unwrap().is_empty());

    // A tag covers only the dialog right after it.
    tag(&mut mapper, "c1");
    mapper.ui_request(&json!({"id":"q3","method":"confirm","title":"Run?"}), 0);
    let events = mapper.ui_request(&json!({"id":"q4","method":"confirm","title":"Tabs?"}), 0);
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));

    // A tag whose dialog never came (an aborted hook) is dropped by the
    // next event.
    tag(&mut mapper, "c1");
    mapper.map(
        json!({"type":"tool_execution_update","toolCallId":"c1","partialResult":{"content":[]}}),
        1,
    );
    let events = mapper.ui_request(&json!({"id":"q5","method":"confirm","title":"Tabs?"}), 0);
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));

    // A tagged dialog that is not an allow/reject choice.
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(
        &json!({"id":"q6","method":"select","title":"Pick","options":["Fast","Slow"]}),
        0,
    );
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(&json!({"id":"q7","method":"input","title":"Name?"}), 0);
    assert!(matches!(&events[..], [HarnessEvent::Question { .. }]));
}

#[test]
fn other_status_lines_pass_through() {
    let events = EventMapper::default().ui_request(
        &json!({"id":"s","method":"setStatus","statusKey":"footer","statusText":"hi"}),
        0,
    );
    assert!(matches!(&events[..], [HarnessEvent::Raw { .. }]));
}

#[test]
fn notify_becomes_a_notice() {
    let events = extension_ui(
        &json!({"type":"extension_ui_request","id":"n","method":"notify","message":"Blocked","notifyType":"warning"}),
        0,
    );
    assert_eq!(
        events,
        vec![HarnessEvent::Notice {
            level: NoticeLevel::Warning,
            text: "Blocked".into()
        }]
    );
}
