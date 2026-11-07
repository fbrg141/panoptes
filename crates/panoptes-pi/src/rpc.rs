//! One `pi --mode rpc` child process and its JSONL protocol.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use panoptes_harness::{EventSink, HarnessError, HarnessEvent};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot, watch};

use crate::locate::PiInstall;
use crate::map::{self, EventMapper, ToolTimes};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
/// How long Pi gets to exit after stdin closes before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
const STDERR_TAIL: usize = 4096;

/// Commands waiting for their response, by id. `None` once Pi's stdout has
/// closed: no response can arrive after that, so new commands fail at once.
type Pending = Arc<Mutex<Option<HashMap<String, oneshot::Sender<Value>>>>>;

/// `None` while the process runs; `Some(code)` once it has exited, where
/// `code` is `None` if it was ended by a signal.
type ExitState = Option<Option<i32>>;

pub struct RpcProcess {
    /// Dropping the sender closes Pi's stdin, its signal to shut down.
    stdin: std::sync::Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    pending: Pending,
    next_id: AtomicU64,
    /// Sending (or dropping) this kills the process.
    kill: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    exit: watch::Receiver<ExitState>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn encode(record: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(record).expect("JSON values always serialize");
    bytes.push(b'\n');
    bytes
}

async fn wait_for_exit(exit: &mut watch::Receiver<ExitState>, limit: Duration) -> ExitState {
    let exited = tokio::time::timeout(limit, exit.wait_for(Option::is_some)).await;
    match exited {
        Ok(Ok(state)) => *state,
        _ => None,
    }
}

impl RpcProcess {
    pub fn spawn(
        install: &PiInstall,
        cwd: &Path,
        session_file: Option<&str>,
        events: EventSink,
        times: ToolTimes,
    ) -> anyhow::Result<Self> {
        let mut command = Command::new(&install.program);
        command.args(["--mode", "rpc"]);
        if let Some(file) = session_file {
            command.args(["--session", file]);
        }
        let mut child = command
            .current_dir(cwd)
            .env("PATH", &install.path_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("Could not start {}", install.program.display()))?;

        let mut stdin = child.stdin.take().context("pi stdin")?;
        let stdout = child.stdout.take().context("pi stdout")?;
        let mut stderr = child.stderr.take().context("pi stderr")?;

        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(record) = stdin_rx.recv().await {
                if stdin.write_all(&record).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = watch::channel::<ExitState>(None);
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = kill_rx => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            let _ = exit_tx.send(Some(status.ok().and_then(|status| status.code())));
        });

        let stderr_tail = Arc::new(std::sync::Mutex::new(String::new()));
        let stderr_done = {
            let stderr_tail = stderr_tail.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                while let Ok(read) = stderr.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    let mut tail = stderr_tail.lock().unwrap();
                    tail.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if tail.len() > STDERR_TAIL {
                        let mut cut = tail.len() - STDERR_TAIL;
                        while !tail.is_char_boundary(cut) {
                            cut += 1;
                        }
                        tail.drain(..cut);
                    }
                }
            })
        };

        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        {
            let pending = pending.clone();
            let mut exit = exit_rx.clone();
            tokio::spawn(async move {
                read_stdout(stdout, &pending, &events, times).await;
                // Stdout closed: Pi is gone or going. Dropping the waiters
                // fails every outstanding command.
                pending.lock().await.take();
                let code = wait_for_exit(&mut exit, SHUTDOWN_GRACE).await.flatten();
                let _ = tokio::time::timeout(Duration::from_secs(1), stderr_done).await;
                let stderr_tail = stderr_tail.lock().unwrap().clone();
                let _ = events.send(HarnessEvent::Exited { code, stderr_tail });
            });
        }

        Ok(Self {
            stdin: std::sync::Mutex::new(Some(stdin_tx)),
            pending,
            next_id: AtomicU64::new(1),
            kill: std::sync::Mutex::new(Some(kill_tx)),
            exit: exit_rx,
        })
    }

    async fn forget(&self, id: &str) {
        if let Some(pending) = self.pending.lock().await.as_mut() {
            pending.remove(id);
        }
    }

    /// Writes one record to Pi. Returns `false` once stdin is closed.
    fn write(&self, record: &Value) -> bool {
        self.stdin
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|stdin| stdin.send(encode(record)).is_ok())
    }

    /// Sends a command and waits for its response. Returns `data` on success.
    pub async fn call(&self, mut command: Value) -> anyhow::Result<Value> {
        let id = format!("panoptes-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let kind = command["type"].as_str().unwrap_or("command").to_string();
        command["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        match self.pending.lock().await.as_mut() {
            Some(pending) => pending.insert(id.clone(), tx),
            None => return Err(not_running()),
        };

        if !self.write(&command) {
            self.forget(&id).await;
            return Err(not_running());
        }

        let response = match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                return Err(
                    HarnessError::exited(format!("Pi exited before answering `{kind}`")).into(),
                );
            }
            Err(_) => {
                self.forget(&id).await;
                return Err(
                    HarnessError::timed_out(format!("Pi did not answer `{kind}` in time")).into(),
                );
            }
        };
        if response["success"] == true {
            Ok(response["data"].clone())
        } else {
            let message = response["error"]
                .as_str()
                .unwrap_or("Pi rejected the command");
            Err(HarnessError::rejected(message).into())
        }
    }

    /// Sends a record that Pi does not answer, such as an
    /// `extension_ui_response`.
    pub fn notify(&self, record: Value) -> anyhow::Result<()> {
        if self.write(&record) {
            Ok(())
        } else {
            Err(not_running())
        }
    }

    /// Closes stdin and gives Pi a moment to exit cleanly before killing it.
    pub async fn shutdown(&self) {
        self.stdin.lock().unwrap().take();
        let mut exit = self.exit.clone();
        if wait_for_exit(&mut exit, SHUTDOWN_GRACE).await.is_none() {
            if let Some(kill) = self.kill.lock().unwrap().take() {
                let _ = kill.send(());
            }
            wait_for_exit(&mut exit, SHUTDOWN_GRACE).await;
        }
    }
}

fn not_running() -> anyhow::Error {
    HarnessError::exited("Pi is not running").into()
}

async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    pending: &Pending,
    events: &EventSink,
    times: ToolTimes,
) {
    // Split strictly on LF bytes: U+2028 and U+2029 are valid inside JSON
    // strings and must not end a record.
    let mut stdout = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut mapper = EventMapper::new(times);
    loop {
        line.clear();
        match stdout.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let record = line.strip_suffix(b"\n").unwrap_or(&line);
        let record = record.strip_suffix(b"\r").unwrap_or(record);
        let Ok(record) = serde_json::from_slice::<Value>(record) else {
            continue;
        };
        let mapped = match record["type"].as_str() {
            Some("response") => {
                if let Some(id) = record["id"].as_str()
                    && let Some(waiter) = pending.lock().await.as_mut().and_then(|p| p.remove(id))
                {
                    let _ = waiter.send(record);
                }
                continue;
            }
            Some("extension_ui_request") => map::extension_ui(&record, now_ms()),
            _ => mapper.map(record, now_ms()),
        };
        for event in mapped {
            let _ = events.send(event);
        }
    }
}
