use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use panoptes_harness::{HarnessEvent, Role, SessionStatus};
use serde_json::json;
use tokio::sync::Mutex;

use super::registry::{EarlyExit, OpenSession};
use super::status::{Tracker, now_ms};
use super::store::SharedStore;
use super::turns::TurnCoordinator;
use super::writers::LaneWriters;
use super::{Emit, LaneWritesChanged, SessionEvent, SessionUpdate, StatusChanged};
use crate::eventlog::EventLog;
use crate::writes;

/// Forwards one process's events, tagged with its session's key, and works
/// out the session's status.
pub(super) struct Forwarder {
    pub(super) key: String,
    pub(super) generation: u64,
    pub(super) open: Arc<Mutex<HashMap<String, OpenSession>>>,
    pub(super) status: Tracker,
    pub(super) emit: Emit,
    pub(super) writes: WriteWatch,
    /// Set if the process ended before `start` had listed the session as
    /// open, with what it last wrote to stderr.
    pub(super) early_exit: EarlyExit,
    pub(super) log: EventLog,
    pub(super) turns: TurnCoordinator,
}

/// What a forwarder needs to notice its session editing other lanes.
pub(super) struct WriteWatch {
    pub(super) writers: LaneWriters,
    pub(super) store: SharedStore,
    /// The session's own lane, never counted as another lane.
    pub(super) own: Option<i64>,
    pub(super) cwd: PathBuf,
}

/// Records neutral diagnostics without retaining prompts, replies, tool
/// arguments or file contents. Repeated tool updates are folded by the viewer.
fn record_harness_event(log: &EventLog, key: &str, event: &HarnessEvent) {
    match event {
        HarnessEvent::RunStarted => log.record("runStarted", json!({"key": key})),
        HarnessEvent::RunSettled { aborted } => {
            log.record("runSettled", json!({"key": key, "aborted": aborted}))
        }
        HarnessEvent::ToolCallUpdated { call } => log.record(
            "toolCall",
            json!({
                "key": key,
                "id": call.id,
                "name": call.name,
                "kind": call.kind,
                "status": call.status,
                "startedAt": call.started_at,
                "endedAt": call.ended_at,
                "editedPaths": writes::edited_paths(call),
            }),
        ),
        HarnessEvent::MessageCompleted { message } if message.role == Role::Assistant => {
            log.record(
                "modelCall",
                json!({
                    "key": key,
                    "messageId": message.id,
                    "usage": message.usage,
                    "failed": message.error.is_some(),
                    "stopped": message.stopped,
                }),
            );
        }
        _ => {}
    }
}

impl Forwarder {
    /// Notes the other lanes a finished edit touched, and forgets the
    /// session's edits once its process ends.
    async fn watch(&self, event: &HarnessEvent) {
        match event {
            HarnessEvent::ToolCallUpdated { call } => {
                let paths = writes::edited_paths(call);
                if paths.is_empty() {
                    return;
                }
                let store = self.writes.store.clone();
                let (own, cwd) = (self.writes.own, self.writes.cwd.clone());
                let touched = tokio::task::spawn_blocking(move || {
                    let lanes = store.lock().lanes()?;
                    anyhow::Ok(writes::lanes_touched(&paths, &cwd, own, &lanes))
                })
                .await;
                let Ok(Ok(touched)) = touched else {
                    return;
                };
                let new: Vec<i64> = touched
                    .into_iter()
                    .filter(|lane| self.writes.writers.record(*lane, &self.key))
                    .collect();
                for lane_id in new {
                    (self.emit)(SessionEvent {
                        key: self.key.clone(),
                        event: SessionUpdate::LaneWrites(LaneWritesChanged { lane_id }),
                    });
                }
            }
            HarnessEvent::Exited { .. } => {
                self.writes.writers.forget(&self.key);
            }
            _ => {}
        }
    }

    pub(super) async fn run(self, mut events: tokio::sync::mpsc::UnboundedReceiver<HarnessEvent>) {
        while let Some(event) = events.recv().await {
            self.watch(&event).await;
            record_harness_event(&self.log, &self.key, &event);
            if matches!(event, HarnessEvent::RunSettled { .. }) {
                let session = self
                    .open
                    .lock()
                    .await
                    .get(&self.key)
                    .filter(|open| open.generation == self.generation)
                    .map(|open| open.session.clone());
                if let Some(session) = session
                    && let Ok(settings) = session.settings().await
                    && let Some(context) = settings.context
                {
                    self.log.record(
                        "context",
                        json!({
                            "key": self.key,
                            "tokens": context.tokens,
                            "window": context.window,
                            "model": settings.model,
                            "thinkingLevel": settings.thinking_level,
                        }),
                    );
                }
            }
            if let HarnessEvent::Exited { stderr_tail, .. } = &event {
                let mut open = self.open.lock().await;
                let current = open
                    .get(&self.key)
                    .is_some_and(|o| o.generation == self.generation);
                if !current {
                    // Closed on purpose or replaced: not news. If `start`
                    // has not listed the session yet, it reads this under
                    // the same lock and refuses to list a dead process.
                    *self.early_exit.lock().unwrap() = Some(stderr_tail.clone());
                    self.turns
                        .handle_event(&self.key, self.generation, &event, SessionStatus::Failed)
                        .await;
                    continue;
                }
                open.remove(&self.key);
                if let HarnessEvent::Exited { code, stderr_tail } = &event {
                    self.log.record(
                        "sessionExited",
                        json!({"key": self.key, "code": code, "stderrTail": stderr_tail}),
                    );
                }
            }
            match &event {
                HarnessEvent::Question { question } => self.log.record(
                    "asked",
                    json!({"key": self.key, "id": question.id, "kind": "question"}),
                ),
                HarnessEvent::Approval { approval } => self.log.record(
                    "asked",
                    json!({"key": self.key, "id": approval.id, "kind": "approval"}),
                ),
                _ => {}
            }
            let turn_event = turn_relevant(&event).then(|| event.clone());
            let turn_status;
            {
                // Holding the tracker while emitting keeps statuses in
                // order with the ones `answer` sends. Turn checkpoints run
                // after the event is forwarded but before the next event.
                let mut tracker = self.status.lock().unwrap();
                let changed = tracker.apply(&event, now_ms());
                turn_status = tracker.status();
                (self.emit)(SessionEvent {
                    key: self.key.clone(),
                    event: SessionUpdate::Harness(event),
                });
                if let Some(status) = changed {
                    self.log
                        .record("status", json!({"key": self.key, "status": status}));
                    (self.emit)(SessionEvent {
                        key: self.key.clone(),
                        event: SessionUpdate::Status(StatusChanged { status }),
                    });
                }
            }
            if let Some(event) = &turn_event {
                self.turns
                    .handle_event(&self.key, self.generation, event, turn_status)
                    .await;
            }
        }
    }
}

/// Which neutral events the turn coordinator needs; cloning a tool update
/// or text delta for every event would be needless work.
fn turn_relevant(event: &HarnessEvent) -> bool {
    matches!(
        event,
        HarnessEvent::RunStarted
            | HarnessEvent::RunSettled { .. }
            | HarnessEvent::Exited { .. }
            | HarnessEvent::QueueChanged { .. }
            | HarnessEvent::MessageStarted { .. }
            | HarnessEvent::MessageCompleted { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::{
        FileEdit, Message, StopReason, ToolCall, ToolKind, ToolOutput, ToolStatus, Usage,
    };

    #[test]
    fn diagnostics_keep_tool_results_and_usage_but_not_their_contents() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path());
        record_harness_event(
            &log,
            "fake:s1",
            &HarnessEvent::ToolCallUpdated {
                call: Box::new(ToolCall {
                    id: "tool-1".into(),
                    name: "edit".into(),
                    kind: ToolKind::Edit,
                    subject: Some("secret subject".into()),
                    input: json!({"secret": "arguments"}),
                    status: ToolStatus::Done,
                    output: Some(ToolOutput::Edits {
                        edits: vec![FileEdit {
                            path: "src/lib.rs".into(),
                            old_text: Some("secret old text".into()),
                            new_text: "secret new text".into(),
                        }],
                        text: Some("secret output".into()),
                    }),
                    images: Vec::new(),
                    started_at: Some(10),
                    ended_at: Some(25),
                }),
            },
        );
        record_harness_event(
            &log,
            "fake:s1",
            &HarnessEvent::MessageCompleted {
                message: Message {
                    id: "message-1".into(),
                    role: Role::Assistant,
                    blocks: Vec::new(),
                    timestamp: Some(25),
                    error: Some("secret provider error".into()),
                    stopped: false,
                    usage: Some(Box::new(Usage {
                        model: Some("model-a".into()),
                        provider: Some("provider-a".into()),
                        input_tokens: 10,
                        output_tokens: 4,
                        cache_read_tokens: 20,
                        cache_write_tokens: 2,
                        cost: Some(0.01),
                        stop_reason: Some(StopReason::Error),
                        ended_at: Some(25),
                    })),
                },
            },
        );
        let snapshot = crate::eventlog::read(dir.path());
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0]["editedPaths"], json!(["src/lib.rs"]));
        assert_eq!(snapshot.events[1]["usage"]["outputTokens"], 4);
        assert_eq!(snapshot.events[1]["failed"], true);
        let recorded = serde_json::to_string(&snapshot.events).unwrap();
        assert!(!recorded.contains("secret"), "{recorded}");
    }
}
