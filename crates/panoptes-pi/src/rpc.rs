//! One `pi --mode rpc` child process and its JSONL protocol.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use panoptes_harness::{EventSink, HarnessEvent, NoticeLevel};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::locate::PiInstall;
use crate::map::EventMapper;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_TAIL: usize = 4096;

type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

pub struct RpcProcess {
    stdin: std::sync::Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    pending: Pending,
    next_id: AtomicU64,
    child: Mutex<Child>,
}

impl RpcProcess {
    pub fn spawn(
        install: &PiInstall,
        cwd: &Path,
        session_file: Option<&str>,
        events: EventSink,
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

        // Writer: owns stdin, so dropping the sender closes it, which is
        // Pi's signal for an orderly shutdown.
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(record) = stdin_rx.recv().await {
                if stdin.write_all(&record).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        let stderr_tail = Arc::new(std::sync::Mutex::new(String::new()));
        {
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
            });
        }

        let pending: Pending = Arc::default();
        {
            let pending = pending.clone();
            let stdin_tx = stdin_tx.clone();
            tokio::spawn(async move {
                // Split strictly on LF bytes: U+2028/U+2029 are valid inside
                // JSON strings and must not end a record.
                let mut stdout = BufReader::new(stdout);
                let mut line = Vec::new();
                let mut mapper = EventMapper::default();
                loop {
                    line.clear();
                    match stdout.read_until(b'\n', &mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let record = line.strip_suffix(b"\n").unwrap_or(&line);
                    let record = record.strip_suffix(b"\r").unwrap_or(record);
                    let Ok(record) = serde_json::from_slice::<Value>(record) else {
                        continue;
                    };
                    match record["type"].as_str() {
                        Some("response") => {
                            if let Some(id) = record["id"].as_str() {
                                if let Some(waiter) = pending.lock().await.remove(id) {
                                    let _ = waiter.send(record);
                                }
                            }
                        }
                        Some("extension_ui_request") => {
                            handle_extension_ui(&record, &stdin_tx, &events);
                        }
                        _ => {
                            for event in mapper.map(record) {
                                let _ = events.send(event);
                            }
                        }
                    }
                }
                // Stdout closed: the process is gone or going. Fail every
                // outstanding command by dropping its waiter.
                pending.lock().await.clear();
                let tail = stderr_tail.lock().unwrap().clone();
                let _ = events.send(HarnessEvent::Exited { code: None, stderr_tail: tail });
            });
        }

        Ok(Self {
            stdin: std::sync::Mutex::new(Some(stdin_tx)),
            pending,
            next_id: AtomicU64::new(1),
            child: Mutex::new(child),
        })
    }

    /// Sends a command and waits for its response. Returns `data` on success.
    pub async fn call(&self, mut command: Value) -> anyhow::Result<Value> {
        let id = format!("panoptes-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        command["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let mut record = serde_json::to_vec(&command)?;
        record.push(b'\n');
        let sent = self
            .stdin
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|stdin| stdin.send(record).is_ok());
        if !sent {
            self.pending.lock().await.remove(&id);
            bail!("Pi is not running");
        }

        let response = match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => bail!("Pi exited before answering `{}`", command["type"]),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                bail!("Pi did not answer `{}` in time", command["type"]);
            }
        };
        if response["success"] == true {
            Ok(response["data"].clone())
        } else {
            Err(anyhow!(
                "{}",
                response["error"].as_str().unwrap_or("Pi rejected the command")
            ))
        }
    }

    /// Closes stdin and gives Pi a moment to exit before killing it.
    pub async fn shutdown(&self) {
        self.stdin.lock().unwrap().take();
        let mut child = self.child.lock().await;
        if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    }
}

/// Extension dialogs are not supported yet: they are cancelled straight away
/// so Pi never waits on them, and the user is told what was skipped.
fn handle_extension_ui(
    request: &Value,
    stdin: &mpsc::UnboundedSender<Vec<u8>>,
    events: &EventSink,
) {
    let method = request["method"].as_str().unwrap_or_default();
    match method {
        "select" | "confirm" | "input" | "editor" => {
            let response = json!({"type": "extension_ui_response", "id": request["id"], "cancelled": true});
            if let Ok(mut record) = serde_json::to_vec(&response) {
                record.push(b'\n');
                let _ = stdin.send(record);
            }
            let title = request["title"].as_str().unwrap_or(method);
            let _ = events.send(HarnessEvent::Notice {
                level: NoticeLevel::Warning,
                text: format!("An extension asked for input ({title}); dialogs are not supported yet, so it was cancelled."),
            });
        }
        "notify" => {
            let level = match request["notifyType"].as_str() {
                Some("error") => NoticeLevel::Error,
                Some("warning") => NoticeLevel::Warning,
                _ => NoticeLevel::Info,
            };
            let text = request["message"].as_str().unwrap_or_default().to_string();
            let _ = events.send(HarnessEvent::Notice { level, text });
        }
        _ => {
            let _ = events.send(HarnessEvent::Raw { payload: request.clone() });
        }
    }
}
