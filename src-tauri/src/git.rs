//! Bounded, blocking process execution for Git calls made on worker threads.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

/// Long enough for ordinary repository operations, short enough that a broken
/// credential helper or stuck subprocess cannot hold a lane open forever.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a command to completion, draining both pipes while it runs and killing
/// it at `timeout`. Optional input is written on its own thread for the same
/// reason: a large batch request must not deadlock against a full stdout pipe.
pub fn output(command: &mut Command, input: Option<&[u8]>) -> anyhow::Result<Output> {
    output_with_timeout(command, input, TIMEOUT)
}

fn output_with_timeout(
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

#[cfg(test)]
mod tests {
    use super::*;

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
