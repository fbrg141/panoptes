//! The user's shell `PATH`, for an app started from Finder or the Dock.
//!
//! macOS gives such an app only `/usr/bin:/bin:/usr/sbin:/sbin`, so `pi`,
//! the `node` its launcher needs, and the tools a project's setup or the
//! agent runs (pnpm, cargo, …) would not be found. At startup the app asks
//! the user's login shell for its `PATH` and puts those folders first.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Marks the `PATH` in the shell's output, which may also hold whatever the
/// user's profile prints.
const MARKER: &str = "__PANOPTES_PATH__";

/// A slow profile must not hold up the window.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Prepends the login shell's `PATH` to this process's `PATH`. Leaves it
/// alone when the shell can't be run or says nothing usable.
pub fn apply() {
    let Some(shell_path) = login_shell_path() else {
        return;
    };
    let current = std::env::var_os("PATH").unwrap_or_default();
    if let Some(merged) = merge(&shell_path, &current) {
        // SAFETY: called first thing in `run`, before Tauri or the async
        // runtime start any thread that could read the environment.
        unsafe { std::env::set_var("PATH", merged) };
    }
}

fn login_shell_path() -> Option<String> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    // Interactive and login, so both .zprofile and .zshrc style setups count.
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARKER}%s{MARKER}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    extract(&String::from_utf8_lossy(&output.stdout))
}

/// The `PATH` between the markers, if there is a non-empty one.
fn extract(output: &str) -> Option<String> {
    let (_, rest) = output.split_once(MARKER)?;
    let (path, _) = rest.split_once(MARKER)?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// The shell's folders, then any of the current ones it lacks, without
/// duplicates or empty entries.
fn merge(shell_path: &str, current: &OsString) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(shell_path).chain(std::env::split_paths(current)) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_path_from_noisy_output() {
        let output = format!("Welcome!\n{MARKER}/opt/homebrew/bin:/usr/bin{MARKER}bye");
        assert_eq!(
            extract(&output),
            Some("/opt/homebrew/bin:/usr/bin".to_string())
        );
    }

    #[test]
    fn rejects_missing_or_empty_paths() {
        assert_eq!(extract("no markers here"), None);
        assert_eq!(extract(&format!("{MARKER}{MARKER}")), None);
        assert_eq!(extract(&format!("{MARKER}/usr/bin")), None);
    }

    #[test]
    fn puts_the_shell_first_and_keeps_the_rest_once() {
        let merged = merge(
            "/Users/me/.local/bin:/opt/homebrew/bin:/usr/bin",
            &OsString::from("/usr/bin:/bin::/usr/sbin"),
        )
        .unwrap();
        assert_eq!(
            merged,
            OsString::from("/Users/me/.local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin")
        );
    }

    #[test]
    fn reads_a_real_shell() {
        // `sh` stands in for the user's shell; it must report some PATH.
        let output = Command::new("sh")
            .args(["-c", &format!("printf '{MARKER}%s{MARKER}' \"$PATH\"")])
            .output()
            .unwrap();
        assert!(extract(&String::from_utf8_lossy(&output.stdout)).is_some());
    }
}
