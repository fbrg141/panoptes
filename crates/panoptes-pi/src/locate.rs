//! Finding the `pi` executable and the folders it uses.
//!
//! Apps launched from Finder get a minimal `PATH`, so the user's shell setup
//! cannot be relied on. A bundled Pi is preferred when the app ships one;
//! otherwise the usual install locations are searched.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        self.version
            .split('.')
            .next()
            .and_then(|major| major.parse::<u32>().ok())
            .is_some_and(|major| SUPPORTED_MAJOR_VERSIONS.contains(&major))
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
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

    let output = Command::new(&program)
        .arg("--version")
        .env("PATH", &path_env)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("Could not run {}", program.display()))?;
    if !output.status.success() {
        bail!("`{} --version` failed", program.display());
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

    Ok(PiInstall { program, version, path_env })
}

/// Pi's config folder, `~/.pi/agent` unless overridden.
pub fn agent_dir() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".pi/agent"))
}

/// Where Pi stores sessions. A `sessionDir` set in Pi's settings file is not
/// read yet; only the environment override and the default are honoured.
pub fn sessions_dir() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_SESSION_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| agent_dir().join("sessions"))
}
