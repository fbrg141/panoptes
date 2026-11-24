use super::*;
use crate::map::state::{ApprovalReply, Codemode};

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
        shared.approvals()["q1"],
        ApprovalReply {
            options: Some(("Yes".to_string(), "No".to_string())),
            subject: Some("execute: rm -rf x".to_string()),
        }
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
    assert_eq!(
        shared.approvals()["q2"],
        ApprovalReply {
            options: None,
            subject: Some("execute: rm -rf x".to_string()),
        }
    );
}

#[test]
fn a_permission_dialog_allows_its_lasting_option() {
    let (mut mapper, shared) = started();
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(
        &json!({"id":"q1","method":"select","title":"Permission Required",
            "options":["Allow Once","Allow Always","Reject","Reject with Reason"]}),
        0,
    );
    let [HarnessEvent::Approval { approval }] = events.as_slice() else {
        panic!("expected an approval, got {events:?}");
    };
    assert_eq!(approval.subject.as_deref(), Some("execute: rm -rf x"));
    // One Allow has to be the lasting one, or the prompt comes back.
    assert_eq!(
        shared.approvals()["q1"].options,
        Some(("Allow Always".to_string(), "Reject".to_string()))
    );
}

#[test]
fn an_allow_already_given_settles_the_dialog_itself() {
    let (mut mapper, shared) = started();
    shared.allow("execute: rm -rf x");
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(
        &json!({"id":"q1","method":"select","title":"Permission Required",
            "options":["Allow Once","Allow Always","Reject","Reject with Reason"]}),
        0,
    );
    assert!(
        events.is_empty(),
        "an allowed repeat is not shown: {events:?}"
    );
    assert!(shared.approvals().is_empty());
    assert_eq!(
        mapper.take_auto_answers(),
        vec![json!({"type":"extension_ui_response","id":"q1","value":"Allow Always"})]
    );

    // Only the same action is covered: another command still asks.
    mapper.map(
        json!({"type":"tool_execution_start","toolCallId":"c2","toolName":"bash","args":{"command":"rm -rf dist"}}),
        1,
    );
    tag(&mut mapper, "c2");
    let events = mapper.ui_request(&json!({"id":"q2","method":"confirm","title":"Allow?"}), 2);
    assert!(matches!(&events[..], [HarnessEvent::Approval { .. }]));
}

#[test]
fn an_allow_covers_the_whole_command_not_just_its_first_line() {
    let shared = Shared::default();
    shared.allow("execute: git push");
    let mut mapper = EventMapper::new(shared.clone());
    // Shown as `git push`, but it runs more than the user allowed.
    mapper.map(
        json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"bash",
            "args":{"command":"git push\nrm -rf build"}}),
        0,
    );
    tag(&mut mapper, "c1");
    let events = mapper.ui_request(&json!({"id":"q1","method":"confirm","title":"Allow?"}), 0);
    let [HarnessEvent::Approval { approval }] = events.as_slice() else {
        panic!("a longer command must still ask, got {events:?}");
    };
    assert_eq!(
        approval.subject.as_deref(),
        Some("execute: git push\nrm -rf build")
    );
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
    assert!(shared.approvals().is_empty());

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
fn codemode_status_updates_session_settings_without_a_raw_event() {
    let mut mapper = EventMapper::default();
    let events = mapper.ui_request(
        &json!({"id":"s","method":"setStatus","statusKey":"panoptes.codemode","statusText":"on"}),
        0,
    );
    assert!(events.is_empty());
    assert_eq!(
        mapper.shared.codemode(),
        Codemode::Available { enabled: true }
    );
}

#[test]
fn codemode_is_unavailable_without_the_tool() {
    let mut mapper = EventMapper::default();
    let events = mapper.ui_request(
        &json!({"id":"s","method":"setStatus","statusKey":"panoptes.codemode","statusText":"unavailable"}),
        0,
    );
    assert!(events.is_empty());
    assert_eq!(mapper.shared.codemode(), Codemode::Unavailable);
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
