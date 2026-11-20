//! The Panoptes backend, with no UI dependency: [`service::Service`] over
//! open sessions and the store, plus lanes, changes, cleanup and the event
//! log. The app (`panoptes-app`) loads the harnesses, builds a `Service` and
//! calls it directly.

pub mod changes;
pub mod cleanup;
pub mod eventlog;
pub mod files;
pub mod git;
pub mod lanes;
pub mod projects;
pub mod service;
pub mod shell_path;
pub mod store;
pub mod writes;
