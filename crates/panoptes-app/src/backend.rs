//! The bridge between GPUI and `panoptes-core`: a tokio runtime that runs
//! every [`Service`] call, and a channel that carries session events back
//! to the UI thread.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use panoptes_core::eventlog::EventLog;
use panoptes_core::service::{Service, ServiceError, SessionEvent};
use panoptes_core::store::{Preferences, Store};
use panoptes_harness::Harness;
use panoptes_pi::PiHarness;
use tokio::sync::{mpsc, oneshot};

/// The app's bundle identifier, which names its data folder. It is the one
/// the Tauri app used, so an update keeps the existing store and lanes.
const IDENTIFIER: &str = "dev.panoptes.app";

pub struct Backend {
    runtime: tokio::runtime::Runtime,
    service: Arc<Service>,
    /// Harnesses that could not be loaded, and why.
    pub harness_errors: Vec<String>,
    /// Set when the Pi found is a version Panoptes does not support.
    pub unsupported: Option<String>,
    /// Where the event log is written.
    pub logs_dir: PathBuf,
}

pub struct Started {
    pub backend: Arc<Backend>,
    pub events: mpsc::UnboundedReceiver<SessionEvent>,
    /// Loaded before the first frame so the saved appearance applies at once.
    pub preferences: Preferences,
}

impl Backend {
    /// Opens the store and loads the harnesses. A store that can't be read
    /// stops the app with an error, never a silent fresh start.
    pub fn start() -> anyhow::Result<Started> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("panoptes-core")
            .build()
            .context("Could not start the backend runtime")?;
        let paths = Paths::resolve()?;
        let store = Store::open(&paths.data_dir)?;
        let preferences = store.preferences()?;

        let mut harnesses: Vec<Arc<dyn Harness>> = Vec::new();
        let mut harness_errors = Vec::new();
        let mut harness_info = Vec::new();
        let mut unsupported = None;
        match panoptes_pi::locate(bundled_pi().as_deref()) {
            Ok(install) => {
                harness_info.push(serde_json::json!({"id": "pi", "version": install.version}));
                if !install.is_supported() {
                    unsupported = Some(format!(
                        "Pi {} is not a supported version. Expect problems.",
                        install.version
                    ));
                }
                harnesses.push(Arc::new(PiHarness::new(install)));
            }
            Err(error) => harness_errors.push(format!("{error:#}")),
        }

        let (sender, events) = mpsc::unbounded_channel();
        let service = {
            let _runtime = runtime.enter();
            Service::new(
                harnesses,
                store,
                paths.lanes_root,
                Arc::new(move |event| {
                    let _ = sender.send(event);
                }),
            )
        };
        let logs_dir = paths.data_dir.join("logs");
        let log = EventLog::open(&logs_dir);
        log.record(
            "appStarted",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "shell": "gpui",
                "harnesses": harness_info,
                "harnessErrors": harness_errors,
            }),
        );
        let service = Arc::new(service.with_event_log(log));
        Ok(Started {
            backend: Arc::new(Self {
                runtime,
                service,
                harness_errors,
                unsupported,
                logs_dir,
            }),
            events,
            preferences,
        })
    }

    /// Runs a service call on the backend runtime. The returned future can
    /// be awaited on any executor, GPUI's included.
    pub fn call<F, Fut, T>(
        &self,
        call: F,
    ) -> impl Future<Output = anyhow::Result<T>> + use<F, Fut, T>
    where
        F: FnOnce(Arc<Service>) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (answer, answered) = oneshot::channel();
        let work = call(self.service.clone());
        self.runtime.spawn(async move {
            let _ = answer.send(work.await);
        });
        async move {
            answered
                .await
                .context("The backend stopped before answering")
        }
    }

    /// Runs a fallible service call, recording its error in the event log
    /// under `command`.
    pub fn run<F, Fut, T>(
        &self,
        command: &'static str,
        call: F,
    ) -> impl Future<Output = Result<T, ServiceError>> + use<F, Fut, T>
    where
        F: FnOnce(Arc<Service>) -> Fut,
        Fut: Future<Output = Result<T, ServiceError>> + Send + 'static,
        T: Send + 'static,
    {
        let service = self.service.clone();
        let answer = self.call(call);
        async move {
            let result = match answer.await {
                Ok(result) => result,
                Err(error) => Err(ServiceError::other(format!("{error:#}"))),
            };
            if let Err(error) = &result {
                service.record_error(command, error);
            }
            result
        }
    }

    /// Lets every agent process shut down cleanly. Blocks the caller.
    pub fn close_all(&self) {
        let service = self.service.clone();
        self.runtime
            .block_on(async move { service.close_all().await });
    }
}

struct Paths {
    data_dir: PathBuf,
    lanes_root: PathBuf,
}

impl Paths {
    fn resolve() -> anyhow::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("Could not find the home folder")?;
        let data_dir = app_data_dir(&home);
        let panoptes_home = home.join("Panoptes");
        // Debug builds keep their own store and lanes, so a schema step can be
        // reworked freely until it ships.
        Ok(if cfg!(debug_assertions) {
            Self {
                data_dir: data_dir.join("dev"),
                lanes_root: panoptes_home.join("dev").join("lanes"),
            }
        } else {
            Self {
                data_dir,
                lanes_root: panoptes_home.join("lanes"),
            }
        })
    }
}

/// The app data folder for [`IDENTIFIER`], where the Tauri app kept it too.
fn app_data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support").join(IDENTIFIER)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join(IDENTIFIER)
    }
}

/// A Pi bundled into the app's resources wins over an installed one.
fn bundled_pi() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // <App>.app/Contents/MacOS/<exe> -> <App>.app/Contents/Resources
    let resources = exe.parent()?.parent()?.join("Resources");
    Some(resources.join("pi/bin/pi"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_is_the_one_earlier_versions_used() {
        let dir = app_data_dir(Path::new("/Users/someone"));
        if cfg!(target_os = "macos") {
            assert_eq!(
                dir,
                Path::new("/Users/someone/Library/Application Support/dev.panoptes.app")
            );
        } else {
            assert!(dir.ends_with("dev.panoptes.app"));
        }
    }
}
