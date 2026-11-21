use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// How to answer an approval: `None` for a confirm dialog, or the select
/// options that mean allow and reject.
pub(crate) type ApprovalReply = Option<(String, String)>;

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
    /// When each tool call started and ended, as seen live. Pi's history has
    /// no such times, so history reloads borrow them from here and give the
    /// same values the live events did.
    tool_times: Mutex<HashMap<String, (i64, Option<i64>)>>,
    /// How to answer each open approval.
    approvals: Mutex<HashMap<String, ApprovalReply>>,
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
