//! The app's session registry: which harnesses are loaded, which sessions
//! are open, and Panoptes' own store. Independent of the UI so it can be
//! tested with a fake harness.

use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;

use panoptes_harness::Harness;
use serde_json::json;

use crate::eventlog::EventLog;
use crate::projects::Location;
use crate::store::Store;

#[cfg(test)]
use crate::changes::TreeRange;
#[cfg(test)]
use crate::lanes::{ArchiveStrategy, LaneWork};
#[cfg(test)]
use crate::store::{Lane, LaneState};
#[cfg(test)]
use panoptes_harness::{
    AgentSession, Answer, DiscoveredSession, HarnessEvent, HarnessRef, ImageAttachment, Message,
    PromptMode, PromptOutcome, SessionStatus, StartSession,
};

mod changes;
mod cleanup;
mod commands;
mod error;
mod events;
mod forward;
mod key;
mod lanes;
mod listing;
mod preferences;
mod projects;
mod registry;
mod sessions;
mod status;
mod store;
mod turns;
mod writers;

pub use error::{ErrorKind, ServiceError, ServiceResult};
pub use events::{Emit, LaneWritesChanged, SessionEvent, SessionUpdate, StatusChanged};
pub use key::{harness_key, panoptes_key};
pub use lanes::{ArchiveLaneOutcome, DeleteLaneOutcome};
pub use listing::{BusyWriter, LaneNavigation, Listing, Session};
pub use sessions::{AdoptionOutcome, OpenedSession};

use registry::Registry;
#[cfg(test)]
use status::now_ms;
use store::SharedStore;

pub struct Service {
    /// Open and starting processes, lifecycle locks and cross-lane writers.
    registry: Registry,
    harnesses: HashMap<String, Arc<dyn Harness>>,
    /// Shared with the blocking threads that run every store call.
    store: SharedStore,
    /// Central folder for lane worktrees, normally `~/Panoptes/lanes`.
    lanes_root: PathBuf,
    /// Cancellation flags for setup commands, keyed by their project.
    lane_cancellations: Arc<std::sync::Mutex<HashMap<i64, Arc<AtomicBool>>>>,
    /// Lanes with an archive, restore or delete in progress.
    busy_lanes: Arc<std::sync::Mutex<HashSet<i64>>>,
    /// Panoptes' own record of what happened; disabled unless given one.
    log: EventLog,
    /// Durable, lane-level before/after boundaries for writable runs.
    turns: turns::TurnCoordinator,
    /// Where each working directory sits in git, so each is looked up once.
    locations: std::sync::Mutex<HashMap<String, Location>>,
    emit: Emit,
}

impl Service {
    pub fn new(
        harnesses: Vec<Arc<dyn Harness>>,
        store: Store,
        lanes_root: PathBuf,
        emit: Emit,
    ) -> Self {
        let shared_store = SharedStore::new(store);
        Self {
            registry: Registry::new(),
            harnesses: harnesses
                .into_iter()
                .map(|harness| (harness.id().to_string(), harness))
                .collect(),
            store: shared_store.clone(),
            turns: turns::TurnCoordinator::new(shared_store),
            lanes_root,
            lane_cancellations: Arc::default(),
            busy_lanes: Arc::default(),
            log: EventLog::default(),
            locations: std::sync::Mutex::default(),
            emit,
        }
    }

    /// Records what happens from now on into `log`.
    pub fn with_event_log(mut self, log: EventLog) -> Self {
        self.log = log;
        self
    }

    /// Records an error a command returned to the UI.
    pub fn record_error(&self, command: &str, error: &ServiceError) {
        self.log.record(
            "error",
            json!({"command": command, "kind": error.kind, "message": error.message}),
        );
    }

    fn harness(&self, id: &str) -> ServiceResult<Arc<dyn Harness>> {
        self.harnesses.get(id).cloned().ok_or_else(|| {
            ServiceError::new(
                ErrorKind::Unavailable,
                format!("The {id} harness is not available"),
            )
        })
    }
}

#[cfg(test)]
mod tests;
