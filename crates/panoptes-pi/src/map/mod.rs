//! Translating Pi's messages and RPC events into the neutral harness types.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// What the live event stream learns that a session's history needs too,
/// shared between the [`EventMapper`] and the session.
#[derive(Default)]
pub struct SessionState {
    /// Set by the permission extension's startup status.
    pub permission_guard: AtomicBool,
    /// When each tool call started and ended, as seen live. Pi's history has
    /// no such times, so history reloads borrow them from here and give the
    /// same values the live events did.
    pub tool_times: Mutex<HashMap<String, (i64, Option<i64>)>>,
    /// How to answer each open approval: `None` for a confirm dialog, or the
    /// select options that mean allow and reject.
    pub approvals: Mutex<HashMap<String, Option<(String, String)>>>,
    /// The ids messages streamed under, by [`message_key`], in the order
    /// they ended. Pi writes the message object it streamed to the session
    /// file unchanged, so history finds its entries here and keeps the ids
    /// the UI already has. Two messages can share a key (same role in the
    /// same millisecond); history hands their ids out in the same order.
    pub live_ids: Mutex<HashMap<String, Vec<String>>>,
    /// When each message that streamed ended, by its live id, so history
    /// gives the end time the live event did.
    pub ended_at: Mutex<HashMap<String, i64>>,
}

pub type Shared = Arc<SessionState>;
mod content;
mod dialogs;
mod history;
mod live;
mod message;
mod models;
mod tools;

pub use content::user_text;
pub use history::history;
pub use live::EventMapper;
pub use models::{context_usage, model, model_ref};

#[cfg(test)]
mod tests;
