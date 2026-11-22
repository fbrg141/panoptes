use std::collections::HashMap;
use std::sync::Arc;

use panoptes_harness::{HarnessEvent, Role, SessionStatus};

/// Works out a session's status from its events. Waiting wins over
/// Running, so a session that needs the user always says so.
#[derive(Debug)]
pub(super) struct StatusTracker {
    status: SessionStatus,
    running: bool,
    /// Open questions and approvals, with when the harness stops waiting
    /// (Unix ms).
    questions: HashMap<String, Option<i64>>,
    /// Steering or follow-up prompts waiting in the harness.
    queued: bool,
    /// The events that asked them, in order, so a reopened window can be
    /// shown them again. Only those still in `questions` are open.
    asked: Vec<HarnessEvent>,
    /// The latest assistant message in this run reported an error.
    errored: bool,
    /// How the last run ended: Idle before any, then Done or Failed.
    settled: SessionStatus,
}

impl Default for StatusTracker {
    fn default() -> Self {
        Self {
            status: SessionStatus::Idle,
            running: false,
            questions: HashMap::new(),
            queued: false,
            asked: Vec::new(),
            errored: false,
            settled: SessionStatus::Idle,
        }
    }
}

impl StatusTracker {
    pub(super) fn is_idle(&self) -> bool {
        !self.running && !self.queued && self.questions.is_empty()
    }

    pub(super) fn status(&self) -> SessionStatus {
        self.status
    }

    /// Folds in one event; returns the new status if it changed.
    pub(super) fn apply(&mut self, event: &HarnessEvent, now: i64) -> Option<SessionStatus> {
        match event {
            HarnessEvent::RunStarted => {
                self.running = true;
                self.errored = false;
            }
            HarnessEvent::MessageCompleted { message } if message.role == Role::Assistant => {
                // A retried provider error is cleared by the reply that follows.
                self.errored = message.error.is_some();
            }
            HarnessEvent::Question { question } => {
                self.questions
                    .insert(question.id.clone(), question.expires_at);
                self.asked.push(event.clone());
            }
            HarnessEvent::Approval { approval } => {
                self.questions
                    .insert(approval.id.clone(), approval.expires_at);
                self.asked.push(event.clone());
            }
            HarnessEvent::QueueChanged { queued } => {
                self.queued = !queued.is_empty();
            }
            HarnessEvent::RunSettled { .. } => {
                self.running = false;
                self.questions.clear();
                self.settled = if self.errored {
                    SessionStatus::Failed
                } else {
                    SessionStatus::Done
                };
            }
            HarnessEvent::Exited { .. } => {
                self.running = false;
                self.queued = false;
                self.questions.clear();
                self.settled = SessionStatus::Failed;
            }
            _ => {}
        }
        self.update(now)
    }

    /// What the approval `question_id` lets the agent go ahead with, if it
    /// is about a call the harness could name.
    pub(super) fn approval_subject(&self, question_id: &str) -> Option<String> {
        self.asked.iter().find_map(|event| match event {
            HarnessEvent::Approval { approval } if approval.id == question_id => {
                approval.subject.clone()
            }
            _ => None,
        })
    }

    pub(super) fn answered(&mut self, question_id: &str, now: i64) -> Option<SessionStatus> {
        self.questions.remove(question_id);
        self.update(now)
    }

    /// What a window that opens the session now needs to catch up: its
    /// status, and the events that put it there (the run's start and the
    /// questions and approvals still waiting).
    pub(super) fn snapshot(&mut self, now: i64) -> (SessionStatus, Vec<HarnessEvent>) {
        self.update(now);
        let mut replay = Vec::new();
        if self.running {
            replay.push(HarnessEvent::RunStarted);
        }
        replay.extend(self.asked.iter().cloned());
        (self.status, replay)
    }

    pub(super) fn update(&mut self, now: i64) -> Option<SessionStatus> {
        // The harness picks a default once a question expires.
        self.questions
            .retain(|_, expires| expires.is_none_or(|at| at > now));
        let questions = &self.questions;
        self.asked.retain(|event| {
            let id = match event {
                HarnessEvent::Question { question } => &question.id,
                HarnessEvent::Approval { approval } => &approval.id,
                _ => return false,
            };
            questions.contains_key(id)
        });
        let status = if !self.questions.is_empty() {
            SessionStatus::Waiting
        } else if self.running {
            SessionStatus::Running
        } else {
            self.settled
        };
        (status != self.status).then(|| {
            self.status = status;
            status
        })
    }
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) type Tracker = Arc<std::sync::Mutex<StatusTracker>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, expires_at: Option<i64>) -> HarnessEvent {
        HarnessEvent::Question {
            question: panoptes_harness::Question {
                id: id.into(),
                title: "Proceed?".into(),
                message: None,
                kind: panoptes_harness::QuestionKind::Confirm,
                expires_at,
            },
        }
    }

    #[test]
    fn an_approval_waits_until_decided() {
        let mut tracker = StatusTracker::default();
        tracker.apply(&HarnessEvent::RunStarted, 0);
        let approval = HarnessEvent::Approval {
            approval: panoptes_harness::Approval {
                id: "a1".into(),
                tool_call_id: "c1".into(),
                title: "Allow?".into(),
                message: None,
                subject: Some("execute: rm -rf x".into()),
                expires_at: None,
            },
        };
        assert_eq!(tracker.apply(&approval, 10), Some(SessionStatus::Waiting));
        assert_eq!(
            tracker.approval_subject("a1"),
            Some("execute: rm -rf x".to_string())
        );
        assert_eq!(tracker.answered("a1", 20), Some(SessionStatus::Running));
    }

    #[test]
    fn a_snapshot_replays_the_run_and_what_still_waits() {
        let mut tracker = StatusTracker::default();
        assert_eq!(tracker.snapshot(0), (SessionStatus::Idle, Vec::new()));
        tracker.apply(&HarnessEvent::RunStarted, 0);
        tracker.apply(&question("q1", None), 10);
        tracker.apply(&question("q2", Some(100)), 10);
        tracker.apply(&question("q3", None), 10);
        tracker.answered("q3", 20);
        // q2 has expired by now and q3 was answered: only q1 is shown again.
        let (status, replay) = tracker.snapshot(200);
        assert_eq!(status, SessionStatus::Waiting);
        assert_eq!(replay, vec![HarnessEvent::RunStarted, question("q1", None)]);

        tracker.apply(&HarnessEvent::RunSettled { aborted: false }, 300);
        assert_eq!(tracker.snapshot(300), (SessionStatus::Done, Vec::new()));
    }

    #[test]
    fn an_expired_question_stops_waiting() {
        let mut tracker = StatusTracker::default();
        tracker.apply(&HarnessEvent::RunStarted, 0);
        assert_eq!(
            tracker.apply(&question("q1", Some(100)), 50),
            Some(SessionStatus::Waiting)
        );
        let notice = HarnessEvent::Notice {
            level: panoptes_harness::NoticeLevel::Info,
            text: "still going".into(),
        };
        assert_eq!(tracker.apply(&notice, 150), Some(SessionStatus::Running));
    }
}
