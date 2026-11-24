use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

/// How to answer one open approval, and what an allow covers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ApprovalReply {
    /// `None` for a confirm dialog, otherwise the select options that mean
    /// allow (the lasting one, if the dialog offers a choice) and reject.
    pub options: Option<(String, String)>,
    /// The subject the allow is remembered under, when the call names one.
    pub subject: Option<String>,
}

/// What the controls extension reports about the `codemode` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codemode {
    /// No status line yet: the session has not finished starting.
    Unknown,
    /// Pi has the tool; `enabled` says whether it is active.
    Available { enabled: bool },
    /// Pi has no `codemode` tool, so the switch would do nothing.
    Unavailable,
}

const CODEMODE_OFF: u8 = 1;
const CODEMODE_ON: u8 = 2;
const CODEMODE_UNAVAILABLE: u8 = 3;

/// What the live event stream learns that a session's history needs too,
/// shared between the [`EventMapper`](super::EventMapper) and the session.
///
/// The fields are private so the contract "history gets the same ids and
/// times as the live events" lives in one place: the mapper records through
/// these methods and history reads through [`SessionState::history_view`].
#[derive(Default)]
pub struct SessionState {
    /// Set by the permission extension's startup status.
    permission_guard: AtomicBool,
    /// What Panoptes' controls extension reports about codemode.
    codemode: AtomicU8,
    /// When each tool call started and ended, as seen live. Pi's history has
    /// no such times, so history reloads borrow them from here and give the
    /// same values the live events did.
    tool_times: Mutex<HashMap<String, (i64, Option<i64>)>>,
    /// How to answer each open approval.
    approvals: Mutex<HashMap<String, ApprovalReply>>,
    /// Subjects ([`panoptes_harness::Approval::subject`]) the user has
    /// allowed for this session. A matching approval is settled without
    /// asking again.
    allowed: Mutex<HashSet<String>>,
    /// Responses to dialogs that were settled from `allowed`, waiting to be
    /// written back to the harness.
    auto_answers: Mutex<Vec<Value>>,
    /// The ids messages streamed under, by
    /// [`message_key`](super::message::message_key), in the order they
    /// ended. Pi writes the message object it streamed to the session file
    /// unchanged, so history finds its entries here and keeps the ids the UI
    /// already has. Two messages can share a key (same role in the same
    /// millisecond); history hands their ids out in the same order.
    live_ids: Mutex<HashMap<String, Vec<String>>>,
    /// When each message that streamed ended, by its live id, so history
    /// gives the end time the live event did.
    ended_at: Mutex<HashMap<String, i64>>,
}

pub type Shared = Arc<SessionState>;

impl SessionState {
    pub(super) fn set_permission_guard(&self) {
        self.permission_guard.store(true, Ordering::Relaxed);
    }

    pub(crate) fn has_permission_guard(&self) -> bool {
        self.permission_guard.load(Ordering::Relaxed)
    }

    /// Records what `panoptes.codemode` says: `on`, `off`, or nothing else,
    /// which means the tool is not there.
    pub(super) fn set_codemode(&self, status: Option<&str>) {
        let value = match status {
            Some("on") => CODEMODE_ON,
            Some("off") => CODEMODE_OFF,
            _ => CODEMODE_UNAVAILABLE,
        };
        self.codemode.store(value, Ordering::Relaxed);
    }

    pub(crate) fn codemode(&self) -> Codemode {
        match self.codemode.load(Ordering::Relaxed) {
            CODEMODE_ON => Codemode::Available { enabled: true },
            CODEMODE_OFF => Codemode::Available { enabled: false },
            CODEMODE_UNAVAILABLE => Codemode::Unavailable,
            _ => Codemode::Unknown,
        }
    }

    pub(super) fn record_tool_time(&self, call_id: &str, started: i64, ended: Option<i64>) {
        self.tool_times
            .lock()
            .unwrap()
            .insert(call_id.to_string(), (started, ended));
    }

    pub(super) fn record_approval(&self, question_id: &str, reply: ApprovalReply) {
        self.approvals
            .lock()
            .unwrap()
            .insert(question_id.to_string(), reply);
    }

    /// Removes and returns how to answer `question_id`, if it is an approval.
    pub(crate) fn take_approval(&self, question_id: &str) -> Option<ApprovalReply> {
        self.approvals.lock().unwrap().remove(question_id)
    }

    pub(super) fn clear_approvals(&self) {
        self.approvals.lock().unwrap().clear();
    }

    /// Whether the user already allowed `subject` in this session.
    pub(super) fn is_allowed(&self, subject: &str) -> bool {
        self.allowed.lock().unwrap().contains(subject)
    }

    /// Remembers that the user allowed `subject` in this session.
    pub(crate) fn allow(&self, subject: &str) {
        self.allowed.lock().unwrap().insert(subject.to_string());
    }

    /// Keeps a dialog response for the process to write back, because the
    /// user's allow already settled it.
    pub(super) fn remember_auto_answer(&self, response: Value) {
        self.auto_answers.lock().unwrap().push(response);
    }

    /// The responses to dialogs settled without the user.
    pub(crate) fn take_auto_answers(&self) -> Vec<Value> {
        self.auto_answers.lock().unwrap().drain(..).collect()
    }

    /// Notes that a message with `key` streamed under `live_id`.
    pub(super) fn record_live_id(&self, key: String, live_id: String) {
        self.live_ids
            .lock()
            .unwrap()
            .entry(key)
            .or_default()
            .push(live_id);
    }

    pub(super) fn record_end(&self, live_id: String, now: i64) {
        self.ended_at.lock().unwrap().insert(live_id, now);
    }

    /// A consistent read of everything history borrows from the live
    /// stream, held until the view is dropped.
    pub(super) fn history_view(&self) -> HistoryView<'_> {
        HistoryView {
            tool_times: self.tool_times.lock().unwrap(),
            live_ids: self.live_ids.lock().unwrap(),
            ended_at: self.ended_at.lock().unwrap(),
        }
    }

    #[cfg(test)]
    pub(super) fn approvals(&self) -> MutexGuard<'_, HashMap<String, ApprovalReply>> {
        self.approvals.lock().unwrap()
    }
}

/// What history reads from [`SessionState`], under its locks.
pub(super) struct HistoryView<'a> {
    tool_times: MutexGuard<'a, HashMap<String, (i64, Option<i64>)>>,
    live_ids: MutexGuard<'a, HashMap<String, Vec<String>>>,
    ended_at: MutexGuard<'a, HashMap<String, i64>>,
}

impl HistoryView<'_> {
    /// When the call started and ended, as seen live.
    pub(super) fn tool_time(&self, call_id: &str) -> Option<(i64, Option<i64>)> {
        self.tool_times.get(call_id).copied()
    }

    /// The live id of the `nth` streamed message with `key`.
    pub(super) fn live_id(&self, key: &str, nth: usize) -> Option<&String> {
        self.live_ids.get(key).and_then(|ids| ids.get(nth))
    }

    /// When the message streamed under `live_id` ended, as seen live.
    pub(super) fn ended_at(&self, live_id: &str) -> Option<i64> {
        self.ended_at.get(live_id).copied()
    }
}
