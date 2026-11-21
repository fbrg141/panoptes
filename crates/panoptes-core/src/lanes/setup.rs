use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};

use super::checkpoint::TemporaryFile;
use crate::store::Project;

pub(super) fn run_setup(
    project: &Project,
    path: &Path,
    cancelled: Option<&AtomicBool>,
) -> anyhow::Result<()> {
    let Some(setup) = project.settings.setup.as_deref() else {
        return Ok(());
    };
    // Output goes to a file, not a pipe: a pipe nobody reads fills up after
    // about 64 KB and blocks the setup forever, and a background process
    // that inherits a pipe would keep it open after the shell exits.
    let log_path = std::env::temp_dir().join(format!(
        "panoptes-setup-{}-{}.log",
        std::process::id(),
        SETUP_LOGS.fetch_add(1, Ordering::Relaxed)
    ));
    let log = std::fs::File::create(&log_path).context("Could not create the setup log")?;
    let _log = TemporaryFile(log_path.clone());
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(setup)
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A dedicated group lets cancellation stop package managers and any
        // children they spawned, not just the shell.
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Could not run setup for {}", path.display()))?;
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            stop_setup(&mut child);
            bail!("Lane creation cancelled");
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let log = std::fs::read(&log_path).unwrap_or_default();
            let log = String::from_utf8_lossy(&log);
            bail!(
                "project setup failed ({status}): {}",
                tail(log.trim(), 4000)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(super) static SETUP_LOGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The last `max` bytes of `text`, on a character boundary.
pub(super) fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

pub(super) fn stop_setup(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: the child was started in its own process group. A negative
        // pid signals that group; it cannot name Panoptes' own group.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}
