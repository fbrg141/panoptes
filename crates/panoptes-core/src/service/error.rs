use panoptes_harness::{HarnessError, HarnessErrorKind};
use serde::Serialize;

pub type ServiceResult<T> = Result<T, ServiceError>;

/// An error as the UI receives it: a kind to choose the recovery from, and a
/// message to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The session has no process; open it again.
    NotOpen,
    /// The harness it needs is not installed or failed to load.
    Unavailable,
    /// The harness refused the command; its message says why.
    Rejected,
    /// The harness process ended; reopen the session to restart it.
    Exited,
    /// The harness did not answer in time.
    TimedOut,
    /// A lane creation was cancelled; its partial worktree was rolled back.
    Cancelled,
    /// Anything else (I/O, a harness error with no kind).
    Other,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<anyhow::Error> for ServiceError {
    fn from(error: anyhow::Error) -> Self {
        let kind = match HarnessError::kind_of(&error) {
            Some(HarnessErrorKind::Rejected) => ErrorKind::Rejected,
            Some(HarnessErrorKind::NotRunning) => ErrorKind::NotOpen,
            Some(HarnessErrorKind::Exited) => ErrorKind::Exited,
            Some(HarnessErrorKind::TimedOut) => ErrorKind::TimedOut,
            None => ErrorKind::Other,
        };
        Self::new(kind, format!("{error:#}"))
    }
}

pub(super) fn err(error: anyhow::Error) -> ServiceError {
    error.into()
}

pub(super) fn not_open() -> ServiceError {
    ServiceError::new(ErrorKind::NotOpen, "This session is not open")
}

pub(super) fn only_in_a_lane(what: &str) -> ServiceError {
    ServiceError::new(
        ErrorKind::Rejected,
        format!("Only a session in a lane can be {what}"),
    )
}
