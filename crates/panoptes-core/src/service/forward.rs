use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use panoptes_harness::HarnessEvent;
use serde_json::json;
use tokio::sync::Mutex;

use super::registry::{EarlyExit, OpenSession};
use super::status::{Tracker, now_ms};
use super::store::SharedStore;
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
}

/// What a forwarder needs to notice its session editing other lanes.
pub(super) struct WriteWatch {
    pub(super) writers: LaneWriters,
    pub(super) store: SharedStore,
    /// The session's own lane, never counted as another lane.
    pub(super) own: Option<i64>,
    pub(super) cwd: PathBuf,
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
            {
                // Holding the tracker while emitting keeps statuses in
                // order with the ones `answer` sends.
                let mut tracker = self.status.lock().unwrap();
                let changed = tracker.apply(&event, now_ms());
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
        }
    }
}
