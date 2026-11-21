use std::sync::Arc;

use panoptes_harness::{HarnessEvent, SessionStatus};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub key: String,
    pub event: SessionUpdate,
}

/// What the UI hears about an open session: the harness's own events, plus
/// the status the service works out from them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SessionUpdate {
    Harness(HarnessEvent),
    Status(StatusChanged),
    LaneWrites(LaneWritesChanged),
}

/// Sent when an open session is first seen editing another lane's worktree,
/// so the sidebar can hold back that lane's own session straight away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "laneWritesChanged")]
pub struct LaneWritesChanged {
    pub lane_id: i64,
}

/// Sent when an open session's status changes, and once when it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename = "statusChanged")]
pub struct StatusChanged {
    pub status: SessionStatus,
}

pub type Emit = Arc<dyn Fn(SessionEvent) + Send + Sync>;
