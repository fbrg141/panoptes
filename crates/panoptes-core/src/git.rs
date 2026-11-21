//! Bounded, blocking process execution for Git calls made on worker threads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

/// Long enough for ordinary repository operations, short enough that a broken
/// credential helper or stuck subprocess cannot hold a lane open forever.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// The variables that tie Git to one repository, as listed by
/// `git rev-parse --local-env-vars`. Git itself clears these before it runs a
/// command in another repository (a submodule, say).
const REPOSITORY_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// A `git` command that ignores the repository variables Panoptes inherited,
/// so `-C <path>` decides which repository it acts on. Variables the caller
/// sets afterwards, such as a private `GIT_INDEX_FILE`, still apply. Every
/// Git call goes through here, the tests' fixture helpers included, because
/// `cargo test` run from a Git hook inherits the hook's `GIT_DIR`.
pub fn command() -> Command {
    let mut command = Command::new("git");
    for name in REPOSITORY_VARS {
        command.env_remove(name);
    }
    command
}

/// Removes the repository variables Panoptes inherited from whatever started
/// it (a Git hook, a terminal inside `git rebase -x`, an editor's Git panel).
/// Every child inherits Panoptes' environment, and Git obeys `GIT_DIR` over
/// `-C`, so a leaked one would send lane, checkpoint and diff commands, and
/// any agent's own Git calls, to that repository instead of the lane's.
/// [`command`] covers Panoptes' own Git calls; this also covers the agents,
/// setup scripts and shells it starts.
///
/// # Safety
///
/// Changes the process environment, so it must run before any other thread
/// starts; the app shell calls it first in `run`.
pub unsafe fn forget_inherited_repository() {
    for name in REPOSITORY_VARS {
        // SAFETY: the caller guarantees no other thread is running.
        unsafe { std::env::remove_var(name) };
    }
}

/// Runs a command to completion, draining both pipes while it runs and killing
/// it at `timeout`. Optional input is written on its own thread for the same
/// reason: a large batch request must not deadlock against a full stdout pipe.
pub fn output(command: &mut Command, input: Option<&[u8]>) -> anyhow::Result<Output> {
    output_with_timeout(command, input, TIMEOUT)
}

pub(crate) fn output_with_timeout(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
) -> anyhow::Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A dedicated group also covers helpers and hooks that outlive the
        // immediate Git process while holding one of its pipes open.
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not run {program}"))?;
    let mut stdin = child.stdin.take().context("child stdin")?;
    let input = input.map(<[u8]>::to_vec);
    let writer = std::thread::spawn(move || {
        if let Some(input) = input {
            let _ = stdin.write_all(&input);
        }
        // Closing stdin tells batch commands that the request is complete.
    });
    let mut stdout = child.stdout.take().context("child stdout")?;
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut stderr = child.stderr.take().context("child stderr")?;
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            stop_group(&mut child);
            break status;
        }
        if started.elapsed() >= timeout {
            stop_group(&mut child);
            let _ = child.wait();
            let _ = writer.join();
            let _ = out.join();
            let _ = err.join();
            bail!(
                "{program} did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("{program} input writer failed"))?;
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("{program} output reader failed"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("{program} error reader failed"))??;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn stop_group(child: &mut std::process::Child) {
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
}

// Lane operations: helpers that run git in a worktree with the longer
// lane timeout.

/// Large worktrees can legitimately take a while to snapshot or archive.
/// Keep those operations bounded without applying the diff reader's 30 s limit.
pub(crate) const LANE_GIT_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) fn git_path(cwd: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(run(cwd, &["rev-parse", "--git-path", name])?);
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

pub(crate) fn ref_exists(repo: &Path, reference: &str) -> anyhow::Result<bool> {
    let output = git(repo, &["show-ref", "--verify", "--quiet", reference])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(command_failed("git show-ref", &output)),
    }
}

pub(crate) fn run(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    checked_stdout("git", git(cwd, args)?)
}

pub(crate) fn run_index(cwd: &Path, index: &Path, args: &[&str]) -> anyhow::Result<String> {
    let mut command = command();
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_INDEX_FILE", index);
    let output = output_with_timeout(&mut command, None, LANE_GIT_TIMEOUT)
        .with_context(|| format!("Could not run git {}", args.join(" ")))?;
    checked_stdout("git", output)
}

pub(crate) fn git(cwd: &Path, args: &[&str]) -> anyhow::Result<Output> {
    let mut command = command();
    command.arg("-C").arg(cwd).args(args);
    output_with_timeout(&mut command, None, LANE_GIT_TIMEOUT)
        .with_context(|| format!("Could not run git {}", args.join(" ")))
}

pub(crate) fn checked_stdout(description: &str, output: Output) -> anyhow::Result<String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(command_failed(description, &output))
    }
}

pub(crate) fn command_failed(description: &str, output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    anyhow::anyhow!("{description} failed ({}): {detail}", output.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Git's own list can grow; ours must keep up with the installed Git.
    #[test]
    fn repository_vars_cover_everything_git_lists() {
        let output = Command::new("git")
            .args(["rev-parse", "--local-env-vars"])
            .output()
            .unwrap();
        assert!(output.status.success());
        for name in String::from_utf8(output.stdout).unwrap().lines() {
            assert!(REPOSITORY_VARS.contains(&name), "{name} is not cleared");
        }
    }

    /// The real regression: started with `GIT_DIR` pointing at another
    /// repository, Panoptes' Git calls must still reach the lane's own. The
    /// test re-runs itself as a fresh process so it can change that process'
    /// environment before any thread starts.
    #[test]
    fn inherited_git_dir_does_not_redirect_git() {
        const CHILD: &str = "PANOPTES_TEST_INHERITED_GIT_DIR";
        let init = |dir: &std::path::Path| {
            let status = command()
                .args(["init", "--quiet"])
                .arg(dir)
                .status()
                .unwrap();
            assert!(status.success());
        };
        if let Some(lane) = std::env::var_os(CHILD) {
            let lane = std::path::PathBuf::from(lane);
            let git_dir = |command: &mut Command| {
                let result = output(
                    command
                        .arg("-C")
                        .arg(&lane)
                        .args(["rev-parse", "--absolute-git-dir"]),
                    None,
                )
                .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let found = String::from_utf8(result.stdout).unwrap();
                std::path::Path::new(found.trim()).canonicalize().unwrap()
            };
            let expected = lane.join(".git").canonicalize().unwrap();
            assert_eq!(git_dir(&mut command()), expected);
            // SAFETY: this process runs only this test, and the test harness
            // has no other thread that reads the environment.
            unsafe { forget_inherited_repository() };
            assert_eq!(git_dir(&mut Command::new("git")), expected);
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let (lane, other) = (root.path().join("lane"), root.path().join("other"));
        init(&lane);
        init(&other);
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git::tests::inherited_git_dir_does_not_redirect_git",
                "--test-threads=1",
            ])
            .env(CHILD, &lane)
            .env("GIT_DIR", other.join(".git"))
            .env("GIT_WORK_TREE", &other)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn reads_large_output_and_times_out_a_stuck_command() {
        let result = output(
            Command::new("sh").args(["-c", "printf '%0100000d' 0; printf '%0100000d' 0 >&2"]),
            None,
        )
        .unwrap();
        assert_eq!(result.stdout.len(), 100_000);
        assert_eq!(result.stderr.len(), 100_000);

        let started = Instant::now();
        let error = output_with_timeout(
            Command::new("sh").args(["-c", "sleep 2"]),
            None,
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(error.to_string().contains("did not finish"));

        let started = Instant::now();
        output(Command::new("sh").args(["-c", "sleep 2 &"]), None).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
