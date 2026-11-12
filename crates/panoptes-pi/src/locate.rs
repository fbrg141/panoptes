//! Finding the `pi` executable and the folders it uses.
//!
//! Apps launched from Finder get a minimal `PATH`, so the user's shell setup
//! cannot be relied on. A bundled Pi is preferred when the app ships one;
//! otherwise the usual install locations are searched.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

/// Pi major versions whose RPC protocol this adapter is written against.
pub const SUPPORTED_MAJOR_VERSIONS: &[u32] = &[1];

#[derive(Debug, Clone)]
pub struct PiInstall {
    pub program: PathBuf,
    pub version: String,
    /// `PATH` to give Pi's process. Pi's launcher needs `node` on it.
    pub path_env: OsString,
}

impl PiInstall {
    pub fn is_supported(&self) -> bool {
        major_version(&self.version).is_some_and(|major| SUPPORTED_MAJOR_VERSIONS.contains(&major))
    }
}

/// The major version in what `pi --version` printed, which may be a bare
/// `1.2.3`, a `v1.2.3`, or have a name in front of it (`pi 1.2.3`).
fn major_version(output: &str) -> Option<u32> {
    output.split_whitespace().find_map(|word| {
        let word = word.strip_prefix('v').unwrap_or(word);
        let (major, rest) = word.split_once('.')?;
        rest.starts_with(|c: char| c.is_ascii_digit())
            .then(|| major.parse().ok())?
    })
}

/// How long `pi --version` gets. It runs while the app starts, so a Pi that
/// hangs must not hang Panoptes with it.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs `program --version` and returns what it printed, giving up after
/// [`VERSION_TIMEOUT`].
fn version_of(program: &Path, path_env: &OsString) -> anyhow::Result<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Could not run {}", program.display()))?;
    let mut stdout = child.stdout.take().context("pi stdout")?;
    // A version is a few bytes, so reading it after the exit cannot block
    // on a full pipe.
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > VERSION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "`{} --version` did not answer within {} seconds",
                program.display(),
                VERSION_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        bail!("`{} --version` failed", program.display());
    }
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .with_context(|| format!("Could not read the version of {}", program.display()))?;
    Ok(output.trim().to_string())
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The search path for Pi and for the processes it starts: the inherited
/// `PATH` plus the places Homebrew, npm and Pi's installer put binaries.
fn search_path() -> OsString {
    let home = home();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for extra in [
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Finds Pi, in order: `$PANOPTES_PI`, a copy bundled with the app, then the
/// search path.
pub fn locate(bundled: Option<&Path>) -> anyhow::Result<PiInstall> {
    let path_env = search_path();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = std::env::var_os("PANOPTES_PI") {
        candidates.push(PathBuf::from(explicit));
    }
    if let Some(bundled) = bundled {
        candidates.push(bundled.to_path_buf());
    }
    candidates.extend(std::env::split_paths(&path_env).map(|dir| dir.join("pi")));

    let program = candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .context("Could not find `pi`. Install Pi, or set PANOPTES_PI to its path.")?;

    let version = version_of(&program, &path_env)?;

    Ok(PiInstall {
        program,
        version,
        path_env,
    })
}

/// Pi's config folder, `~/.pi/agent` unless overridden.
pub fn agent_dir() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".pi/agent"))
}

/// Where Pi stores sessions: `$PI_CODING_AGENT_SESSION_DIR`, else the
/// `sessionDir` in Pi's global settings, else `<agent dir>/sessions`.
///
/// A relative `sessionDir` resolves against each session's working folder,
/// so there is no single folder to list; it falls back to the default.
pub fn sessions_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_SESSION_DIR") {
        return PathBuf::from(dir);
    }
    let agent_dir = agent_dir();
    configured_sessions_dir(&agent_dir.join("settings.json"), &home())
        .unwrap_or_else(|| agent_dir.join("sessions"))
}

fn configured_sessions_dir(settings: &Path, home: &Path) -> Option<PathBuf> {
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(settings).ok()?).ok()?;
    let dir = settings["sessionDir"].as_str()?;
    if let Some(rest) = dir.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    let dir = PathBuf::from(dir);
    dir.is_absolute().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    fn install(version: &str) -> PiInstall {
        PiInstall {
            program: PathBuf::from("pi"),
            version: version.into(),
            path_env: OsString::new(),
        }
    }

    #[test]
    fn the_version_is_read_however_pi_prints_it() {
        for supported in [
            "1.0.0",
            "1.12.3-beta.1",
            "v1.2.3",
            "pi 1.4.0",
            "pi v1.4.0\n",
        ] {
            assert!(install(supported).is_supported(), "{supported}");
        }
        for unsupported in ["2.0.0", "v0.9.1", "pi 2.1.0", "", "unknown", "1"] {
            assert!(!install(unsupported).is_supported(), "{unsupported}");
        }
    }

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("pi");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_version_command_is_run_and_its_failure_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path_env = std::env::var_os("PATH").unwrap_or_default();
        let pi = script(dir.path(), "echo 1.2.3");
        assert_eq!(version_of(&pi, &path_env).unwrap(), "1.2.3");
        let pi = script(dir.path(), "exit 3");
        let error = version_of(&pi, &path_env).unwrap_err();
        assert!(error.to_string().contains("failed"), "{error}");
    }

    #[test]
    fn reads_absolute_and_home_relative_session_dirs() {
        let home = Path::new("/Users/me");
        let (_dir, path) = settings(r#"{"sessionDir": "/data/pi"}"#);
        assert_eq!(
            configured_sessions_dir(&path, home),
            Some("/data/pi".into())
        );
        let (_dir, path) = settings(r#"{"sessionDir": "~/pi-sessions"}"#);
        assert_eq!(
            configured_sessions_dir(&path, home),
            Some("/Users/me/pi-sessions".into())
        );
    }

    #[test]
    fn ignores_relative_or_missing_session_dirs() {
        let home = Path::new("/Users/me");
        let (_dir, path) = settings(r#"{"sessionDir": ".pi/sessions"}"#);
        assert_eq!(configured_sessions_dir(&path, home), None);
        let (_dir, path) = settings(r#"{"theme": "dark"}"#);
        assert_eq!(configured_sessions_dir(&path, home), None);
        assert_eq!(
            configured_sessions_dir(Path::new("/nonexistent"), home),
            None
        );
    }

    #[test]
    fn supported_versions_follow_the_major() {
        let install = |version: &str| PiInstall {
            program: PathBuf::from("pi"),
            version: version.to_string(),
            path_env: OsString::new(),
        };
        assert!(install("1.4.0").is_supported());
        assert!(!install("2.0.0").is_supported());
        assert!(!install("garbage").is_supported());
    }
}
