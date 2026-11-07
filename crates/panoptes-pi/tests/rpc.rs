//! The adapter against a scripted stand-in for `pi --mode rpc`
//! (`fixtures/fake_pi.py`), so the process and protocol handling are tested
//! without a real Pi or a model.

use std::path::PathBuf;
use std::time::Duration;

use panoptes_harness::{
    AgentSession, Answer, Block, ContextUsage, Harness, HarnessError, HarnessErrorKind,
    HarnessEvent, ImageAttachment, PromptMode, PromptOutcome, QuestionKind, Role, StartSession,
    ToolKind, ToolOutput, ToolStatus,
};
use panoptes_pi::{PiHarness, PiInstall};
use tokio::sync::mpsc::UnboundedReceiver;

fn fake_harness() -> PiHarness {
    let install = PiInstall {
        program: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_pi.py"),
        version: "1.0.0".to_string(),
        path_env: std::env::var_os("PATH").unwrap_or_default(),
    };
    PiHarness::with_sessions_dir(install, PathBuf::from("/nonexistent"))
}

async fn start() -> (Box<dyn AgentSession>, UnboundedReceiver<HarnessEvent>) {
    start_with(StartSession::new(std::env::temp_dir())).await
}

async fn start_with(
    request: StartSession,
) -> (Box<dyn AgentSession>, UnboundedReceiver<HarnessEvent>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let session = fake_harness().start_session(request, tx).await.unwrap();
    (session, rx)
}

/// Collects events until one matches `done`, failing after a few seconds.
async fn until(
    rx: &mut UnboundedReceiver<HarnessEvent>,
    done: impl Fn(&HarnessEvent) -> bool,
) -> Vec<HarnessEvent> {
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = rx.recv().await {
            let finished = done(&event);
            events.push(event);
            if finished {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out; events so far: {events:?}"));
    events
}

fn text_of(events: &[HarnessEvent]) -> String {
    events
        .iter()
        .find_map(|event| match event {
            HarnessEvent::MessageCompleted { message } if message.role == Role::Assistant => {
                match &message.blocks[0] {
                    Block::Text { text } => Some(text.clone()),
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("an assistant message")
}

#[tokio::test]
async fn a_read_only_session_gets_only_reading_tools() {
    for (read_only, expected) in [(false, "tools=all"), (true, "tools=read,grep,find,ls")] {
        let mut request = StartSession::new(std::env::temp_dir());
        request.read_only = read_only;
        let (session, mut rx) = start_with(request).await;
        session
            .prompt("tools".into(), Vec::new(), PromptMode::Start)
            .await
            .unwrap();
        let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
        assert_eq!(text_of(&events), expected);
        session.shutdown().await;
    }
}

#[tokio::test]
async fn a_prompt_streams_and_settles() {
    let (session, mut rx) = start().await;
    assert_eq!(session.harness_ref().id, "fake-session");

    let outcome = session
        .prompt("hi".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    assert_eq!(outcome, PromptOutcome::Started);
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;

    assert!(matches!(events[0], HarnessEvent::RunStarted));
    // The U+2028 inside the text must not have split the record.
    assert_eq!(text_of(&events), "echo: hi\u{2028}done");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HarnessEvent::BlockDelta { .. }))
    );
    session.shutdown().await;
}

#[tokio::test]
async fn history_keeps_the_ids_messages_streamed_under() {
    let (session, mut rx) = start().await;
    assert!(session.history().await.unwrap().is_empty());

    let mut streamed = Vec::new();
    for prompt in ["one", "two"] {
        session
            .prompt(prompt.into(), Vec::new(), PromptMode::Start)
            .await
            .unwrap();
        let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
        streamed.extend(events.into_iter().filter_map(|event| match event {
            HarnessEvent::MessageCompleted { message } => Some(message.id),
            _ => None,
        }));
        // Fetched incrementally: each reload adds only the new entries.
        let ids: Vec<_> = session
            .history()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, streamed);
    }
    assert_eq!(streamed.len(), 4);
    session.shutdown().await;
}

#[tokio::test]
async fn queued_prompts_keep_their_ids() {
    let (session, mut rx) = start().await;
    session
        .prompt("queue".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    let queues: Vec<Vec<(String, String, PromptMode)>> = events
        .into_iter()
        .filter_map(|event| match event {
            HarnessEvent::QueueChanged { queued } => {
                Some(queued.into_iter().map(|q| (q.id, q.text, q.mode)).collect())
            }
            _ => None,
        })
        .collect();
    let [first, second, third] = queues.as_slice() else {
        panic!("expected three queue changes, got {queues:?}");
    };
    assert_eq!(first.len(), 2);
    assert_ne!(first[0].0, first[1].0);
    // "b" is still waiting under the same id; "c" is new.
    assert_eq!(second[0], first[1]);
    assert_eq!(
        (second[1].1.as_str(), second[1].2),
        ("c", PromptMode::FollowUp)
    );
    assert!(!first.iter().any(|q| q.0 == second[1].0));
    assert!(third.is_empty());
    session.shutdown().await;
}

#[tokio::test]
async fn a_tool_call_is_one_value_live_and_in_history() {
    let (session, mut rx) = start().await;
    session
        .prompt("tool".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    let updates: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            HarnessEvent::ToolCallUpdated { call } => Some(call.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(
        updates.iter().map(|c| c.status).collect::<Vec<_>>(),
        [ToolStatus::Running, ToolStatus::Running, ToolStatus::Failed]
    );
    let live = updates[2];
    assert_eq!(live.kind, ToolKind::Execute);
    assert_eq!(
        live.output,
        Some(ToolOutput::Shell {
            text: "building\nbroken".into(),
            exit_code: Some(3)
        })
    );
    assert!(live.started_at.is_some() && live.ended_at >= live.started_at);
    assert!(
        events.iter().all(|e| !matches!(
            e,
            HarnessEvent::MessageCompleted { message } if message.blocks.is_empty()
        )),
        "the tool result is not sent as a message"
    );

    let history = session.history().await.unwrap();
    assert_eq!(
        history.iter().map(|m| m.role).collect::<Vec<_>>(),
        [Role::User, Role::Assistant, Role::Assistant]
    );
    assert_eq!(
        history[1].blocks,
        vec![Block::ToolCall(Box::new(live.clone()))]
    );
    session.shutdown().await;
}

#[tokio::test]
async fn approvals_are_tied_to_their_call_and_answered_in_kind() {
    let (session, mut rx) = start().await;
    session
        .prompt("gate".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::Approval { .. })).await;
    let Some(HarnessEvent::Approval { approval }) = events.last() else {
        unreachable!()
    };
    assert_eq!(approval.tool_call_id, "call-g");
    session
        .answer(approval.id.clone(), Answer::Decision { allow: false })
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    // The select is answered with its own reject option.
    assert_eq!(text_of(&events), "gate=No");
    session.shutdown().await;
}

#[tokio::test]
async fn a_tools_own_question_is_not_an_approval() {
    let (session, mut rx) = start().await;
    session
        .prompt("tool-ask".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();
    let events = until(&mut rx, |e| {
        matches!(
            e,
            HarnessEvent::Question { .. } | HarnessEvent::Approval { .. }
        )
    })
    .await;
    let Some(HarnessEvent::Question { question }) = events.last() else {
        panic!("expected a question, got {:?}", events.last());
    };
    session
        .answer(
            question.id.clone(),
            Answer::Value {
                value: "Yes".into(),
            },
        )
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    assert_eq!(text_of(&events), "tabs=Yes");
    session.shutdown().await;
}

#[tokio::test]
async fn questions_are_answered_through_the_session() {
    let (session, mut rx) = start().await;
    session
        .prompt("ask".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap();

    let events = until(&mut rx, |e| matches!(e, HarnessEvent::Question { .. })).await;
    let Some(HarnessEvent::Question { question }) = events.last() else {
        unreachable!()
    };
    assert_eq!(question.kind, QuestionKind::Confirm);
    assert_eq!(question.title, "Proceed?");

    session
        .answer(question.id.clone(), Answer::Confirmed { confirmed: true })
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    assert_eq!(text_of(&events), "confirmed=True");
    session.shutdown().await;
}

#[tokio::test]
async fn a_crash_reports_the_exit_code_and_stderr() {
    let (session, mut rx) = start().await;
    let error = session
        .prompt("crash".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exited"), "{error}");
    assert_eq!(
        HarnessError::kind_of(&error),
        Some(HarnessErrorKind::Exited)
    );

    let events = until(&mut rx, |e| matches!(e, HarnessEvent::Exited { .. })).await;
    let Some(HarnessEvent::Exited { code, stderr_tail }) = events.last() else {
        unreachable!()
    };
    assert_eq!(*code, Some(7));
    assert!(stderr_tail.contains("fake pi crashed"));
    let error = session
        .prompt("again".into(), Vec::new(), PromptMode::Start)
        .await
        .unwrap_err();
    assert_eq!(
        HarnessError::kind_of(&error),
        Some(HarnessErrorKind::Exited)
    );
}

#[tokio::test]
async fn shutdown_ends_the_process_cleanly() {
    let (session, mut rx) = start().await;
    session.shutdown().await;
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::Exited { .. })).await;
    assert!(matches!(
        events.last(),
        Some(HarnessEvent::Exited { code: Some(0), .. })
    ));
}

#[tokio::test]
async fn commands_after_shutdown_fail_fast() {
    let (session, _rx) = start().await;
    session.abort().await.unwrap();
    session.shutdown().await;
    let started = std::time::Instant::now();
    let error = session.abort().await.unwrap_err();
    assert_eq!(
        HarnessError::kind_of(&error),
        Some(HarnessErrorKind::Exited)
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn images_travel_with_the_prompt() {
    let (session, mut rx) = start().await;
    let image = ImageAttachment {
        mime_type: "image/png".into(),
        data: "iVBORw0K".into(),
    };
    session
        .prompt("look".into(), vec![image], PromptMode::Start)
        .await
        .unwrap();
    let events = until(&mut rx, |e| matches!(e, HarnessEvent::RunSettled { .. })).await;
    assert_eq!(text_of(&events), "echo: look\u{2028}done [1 image/png]");
    // History keeps the image itself, so it can be shown again.
    let history = session.history().await.unwrap();
    assert_eq!(
        history[0].blocks[1],
        Block::Image {
            mime_type: "image/png".into(),
            data: Some("iVBORw0K".into())
        }
    );
    session.shutdown().await;
}

#[tokio::test]
async fn model_and_thinking_level_can_be_changed() {
    let (session, _rx) = start().await;
    let settings = session.settings().await.unwrap();
    assert_eq!(settings.model.unwrap().id, "fake/small");
    assert_eq!(settings.thinking_level.as_deref(), Some("medium"));
    assert_eq!(settings.thinking_levels, ["off", "low", "medium", "high"]);
    assert_eq!(
        settings.context,
        Some(ContextUsage {
            tokens: Some(1200),
            window: 32_000
        })
    );

    let models = session.models().await.unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["fake/small", "fake/vendor/large"]);
    assert!(!models[0].images);
    assert!(models[1].images);

    session.set_model("fake/vendor/large".into()).await.unwrap();
    session.set_thinking_level("high".into()).await.unwrap();
    let settings = session.settings().await.unwrap();
    assert_eq!(settings.model.unwrap().name, "Large");
    assert_eq!(settings.thinking_level.as_deref(), Some("high"));
    assert_eq!(settings.context.unwrap().window, 200_000);

    // Pi refuses the first; the adapter refuses the second before asking.
    for model in ["fake/missing", "no-provider"] {
        let error = session.set_model(model.into()).await.unwrap_err();
        assert_eq!(
            HarnessError::kind_of(&error),
            Some(HarnessErrorKind::Rejected),
            "{error}"
        );
    }
    session.shutdown().await;
}
