//! Translating Pi's messages and RPC events into the neutral harness types.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use panoptes_harness::{
    Approval, Block, ContextUsage, DeltaKind, FileEdit, HarnessEvent, Message, ModelInfo,
    NoticeLevel, PromptMode, Question, QuestionKind, QueuedPrompt, Role, StopReason, ToolCall,
    ToolImage, ToolKind, ToolOutput, ToolStatus, Usage,
};
use serde_json::Value;

/// What the live event stream learns that a session's history needs too,
/// shared between the [`EventMapper`] and the session.
#[derive(Default)]
pub struct SessionState {
    /// Set by the permission extension's startup status.
    pub permission_guard: AtomicBool,
    /// When each tool call started and ended, as seen live. Pi's history has
    /// no such times, so history reloads borrow them from here and give the
    /// same values the live events did.
    pub tool_times: Mutex<HashMap<String, (i64, Option<i64>)>>,
    /// How to answer each open approval: `None` for a confirm dialog, or the
    /// select options that mean allow and reject.
    pub approvals: Mutex<HashMap<String, Option<(String, String)>>>,
    /// The ids messages streamed under, by [`message_key`], in the order
    /// they ended. Pi writes the message object it streamed to the session
    /// file unchanged, so history finds its entries here and keeps the ids
    /// the UI already has. Two messages can share a key (same role in the
    /// same millisecond); history hands their ids out in the same order.
    pub live_ids: Mutex<HashMap<String, Vec<String>>>,
    /// When each message that streamed ended, by its live id, so history
    /// gives the end time the live event did.
    pub ended_at: Mutex<HashMap<String, i64>>,
}

pub type Shared = Arc<SessionState>;

/// Select options read as "allow" and "reject" when a dialog offers exactly
/// one of each, compared without case.
const ALLOW: &[&str] = &[
    "yes", "allow", "approve", "ok", "run", "proceed", "continue",
];
const REJECT: &[&str] = &["no", "block", "deny", "reject", "cancel", "skip", "abort"];

/// The status key `approvals.js` tags `tool_call` hook dialogs with.
const APPROVAL_TAG: &str = "panoptes.approval";
/// The permission system publishes this status as soon as the session starts.
const PERMISSION_GUARD_STATUS: &str = "pi-permission-system";

/// Tool output beyond this many bytes is cut for display.
const MAX_OUTPUT: usize = 64 * 1024;

/// Text of a user message's `content`, which is a string or a block array.
pub fn user_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

/// Cuts long text for display, keeping its start.
fn truncate(mut text: String) -> String {
    if text.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… (output truncated)");
    }
    text
}

/// Cuts long command output for display, keeping its end: that is where a
/// failing command says what went wrong.
fn truncate_start(text: String) -> String {
    if text.len() <= MAX_OUTPUT {
        return text;
    }
    let mut cut = text.len() - MAX_OUTPUT;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    format!("… (earlier output truncated)\n{}", &text[cut..])
}

/// The images among a tool result's `content` blocks, such as a screenshot
/// or an image file `read` loaded. Kept whole: they are not text to cut.
fn result_images(result: &Value) -> Vec<ToolImage> {
    result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "image")
                .filter_map(|block| {
                    Some(ToolImage {
                        mime_type: block["mimeType"].as_str()?.to_string(),
                        data: block["data"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Joined text of a tool result's `content` blocks, in full.
fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn content_blocks(content: &Value) -> Vec<Block> {
    let Some(blocks) = content.as_array() else {
        return user_text(content)
            .map(|text| vec![Block::Text { text }])
            .unwrap_or_default();
    };
    blocks
        .iter()
        .filter_map(|block| match block["type"].as_str()? {
            "text" => Some(Block::Text {
                text: block["text"].as_str()?.to_string(),
            }),
            "thinking" => Some(Block::Thinking {
                text: block["thinking"].as_str().unwrap_or_default().to_string(),
            }),
            "toolCall" => Some(Block::ToolCall(Box::new(tool_call(
                block["id"].as_str()?,
                block["name"].as_str()?,
                block["arguments"].clone(),
            )))),
            "image" => Some(Block::Image {
                mime_type: block["mimeType"].as_str().unwrap_or_default().to_string(),
                data: block["data"].as_str().map(str::to_string),
            }),
            _ => None,
        })
        .collect()
}

/// A tool call with what Pi's built-in tools do spelled out neutrally.
/// Extension tools are `Other`, with their first string argument as the
/// subject. The call is pending until Pi reports it running.
fn tool_call(id: &str, name: &str, args: Value) -> ToolCall {
    let arg = |key: &str| args[key].as_str().map(str::to_string);
    let (kind, subject, edits) = match name {
        "read" | "ls" => (ToolKind::Read, arg("path"), Vec::new()),
        "grep" | "find" => (ToolKind::Search, arg("pattern"), Vec::new()),
        "bash" => (ToolKind::Execute, arg("command"), Vec::new()),
        "edit" => (ToolKind::Edit, arg("path"), edit_changes(&args)),
        "write" => {
            let edits = match (arg("path"), arg("content")) {
                (Some(path), Some(new_text)) => vec![FileEdit {
                    path,
                    old_text: None,
                    new_text,
                }],
                _ => Vec::new(),
            };
            (ToolKind::Edit, arg("path"), edits)
        }
        _ => {
            let first = args
                .as_object()
                .and_then(|o| o.values().find_map(Value::as_str))
                .map(str::to_string);
            (ToolKind::Other, first, Vec::new())
        }
    };
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        kind,
        subject: subject.map(|s| s.lines().next().unwrap_or_default().to_string()),
        input: args,
        status: ToolStatus::Pending,
        output: (!edits.is_empty()).then_some(ToolOutput::Edits { edits, text: None }),
        images: Vec::new(),
        started_at: None,
        ended_at: None,
    }
}

/// How Pi's `bash` tool reports a non-zero exit, as the last line of its
/// output.
const EXIT_PREFIX: &str = "Command exited with code ";

/// Puts a tool's (partial or final) output on its call. `finished` is
/// `Some(is_error)` once the tool has ended.
fn set_output(call: &mut ToolCall, result: &Value, finished: Option<bool>) {
    let text = result_text(result);
    call.images = result_images(result);
    call.status = match finished {
        None => ToolStatus::Running,
        Some(false) => ToolStatus::Done,
        Some(true) => ToolStatus::Failed,
    };
    call.output = match call.kind {
        // The exit code is read from the full text, before any cut.
        ToolKind::Execute => Some(shell_output(text, finished)),
        ToolKind::Edit => {
            let text = truncate(text);
            let edits = match call.output.take() {
                Some(ToolOutput::Edits { edits, .. }) => edits,
                _ => Vec::new(),
            };
            Some(ToolOutput::Edits {
                edits,
                text: (!text.is_empty()).then_some(text),
            })
        }
        _ => (!text.is_empty() || finished.is_some()).then_some(ToolOutput::Text {
            text: truncate(text),
        }),
    };
}

/// Shell output with its exit code. Pi's `bash` succeeds only on exit 0 and
/// otherwise ends its output with "Command exited with code N"; a command
/// that was stopped or timed out says so instead and has no code.
fn shell_output(text: String, finished: Option<bool>) -> ToolOutput {
    let (body, last) = match text.rsplit_once('\n') {
        Some((body, last)) => (body, last),
        None => ("", text.as_str()),
    };
    let reported = last
        .strip_prefix(EXIT_PREFIX)
        .and_then(|code| code.trim().parse::<i32>().ok());
    let (text, exit_code) = match (reported, finished) {
        (Some(code), Some(_)) => (body.trim_end_matches('\n').to_string(), Some(code)),
        (_, Some(false)) => (text, Some(0)),
        _ => (text, None),
    };
    ToolOutput::Shell {
        text: truncate_start(text),
        exit_code,
    }
}

/// The replacements of an `edit` call. Pi takes an `edits` array, but models
/// also send it as a JSON string, as a single object, or as the older
/// top-level `oldText`/`newText` pair.
fn edit_changes(args: &Value) -> Vec<FileEdit> {
    let Some(path) = args["path"].as_str() else {
        return Vec::new();
    };
    let edits = match &args["edits"] {
        Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let mut list = match edits {
        Value::Array(list) => list,
        Value::Object(_) => vec![edits],
        _ => Vec::new(),
    };
    if args["oldText"].is_string() {
        list.push(args.clone());
    }
    list.iter()
        .filter_map(|edit| {
            Some(FileEdit {
                path: path.to_string(),
                old_text: Some(edit["oldText"].as_str()?.to_string()),
                new_text: edit["newText"].as_str()?.to_string(),
            })
        })
        .collect()
}

/// What identifies a Pi message both live and in its session file: its role
/// and timestamp, or, for a message Pi did not stamp, its role and content.
fn message_key(message: &Value) -> Option<String> {
    let role = message["role"].as_str()?;
    Some(match message["timestamp"].as_i64() {
        Some(timestamp) => format!("{role}@{timestamp}"),
        None => format!("{role}#{}", message["content"]),
    })
}

/// What an assistant message's model call used: Pi records the model,
/// token counts, its own price and the stop reason on the message itself.
fn usage(value: &Value, ended_at: Option<i64>) -> Usage {
    let tokens = &value["usage"];
    let count = |key: &str| tokens[key].as_u64().unwrap_or(0);
    Usage {
        model: value["model"].as_str().map(str::to_string),
        provider: value["provider"].as_str().map(str::to_string),
        input_tokens: count("input"),
        output_tokens: count("output"),
        cache_read_tokens: count("cacheRead"),
        cache_write_tokens: count("cacheWrite"),
        cost: tokens["cost"]["total"].as_f64(),
        stop_reason: match value["stopReason"].as_str() {
            Some("stop") => Some(StopReason::Finished),
            Some("toolUse") => Some(StopReason::ToolUse),
            Some("length") => Some(StopReason::MaxTokens),
            Some("error") => Some(StopReason::Error),
            Some("aborted") => Some(StopReason::Aborted),
            _ => None,
        },
        ended_at,
    }
}

/// Unix milliseconds of an ISO 8601 UTC time as JavaScript writes it,
/// `2026-09-07T16:09:46.140Z`. Pi stamps each session entry this way when
/// it writes it, which for a message is when the message ended.
fn iso_millis(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.splitn(3, ':');
    let hours: i64 = time.next()?.parse().ok()?;
    let minutes: i64 = time.next()?.parse().ok()?;
    let seconds: f64 = time.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since 1970-01-01 in the proleptic Gregorian calendar.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((days * 86_400 + hours * 3_600 + minutes * 60) * 1000 + (seconds * 1000.0).round() as i64)
}

/// Converts one Pi `AgentMessage`. System messages (prompt and tool
/// loadout) are not part of the visible conversation, and tool results
/// belong to their calls; both return `None`. `ended_at` is when the
/// message finished, if known.
pub fn message(value: &Value, id: String, ended_at: Option<i64>) -> Option<Message> {
    let timestamp = value["timestamp"].as_i64();
    let stopped = value["stopReason"].as_str() == Some("aborted");
    let mut usage_of = None;
    let (role, blocks, error) = match value["role"].as_str()? {
        "system" => return None,
        "user" => (Role::User, content_blocks(&value["content"]), None),
        "assistant" => {
            let error = match value["stopReason"].as_str() {
                Some("error") => Some(
                    value["errorMessage"]
                        .as_str()
                        .unwrap_or("The model request failed")
                        .to_string(),
                ),
                _ => None,
            };
            usage_of = Some(Box::new(usage(value, ended_at)));
            (Role::Assistant, content_blocks(&value["content"]), error)
        }
        // Results are folded into their calls (see `history` and
        // `EventMapper`), so they are not messages of their own.
        "toolResult" => return None,
        "bashExecution" => {
            let command = value["command"].as_str().unwrap_or_default();
            let output = value["output"].as_str().unwrap_or_default();
            (
                Role::Other,
                vec![Block::Text {
                    text: truncate(format!("$ {command}\n{output}")),
                }],
                None,
            )
        }
        other => {
            let text = value["summary"]
                .as_str()
                .map(str::to_string)
                .or_else(|| user_text(&value["content"]))
                .unwrap_or_else(|| other.to_string());
            (Role::Other, vec![Block::Text { text }], None)
        }
    };
    Some(Message {
        id,
        role,
        blocks,
        timestamp,
        error,
        stopped,
        usage: usage_of,
    })
}

/// Turns the Pi event stream into harness events. Pi does not give
/// streaming messages an id, so each one gets a live id here; history
/// reloads later replace them with stable entry ids.
///
/// Tool calls are tracked from `tool_execution_start` to `_end`, so every
/// [`HarnessEvent::ToolCallUpdated`] carries the whole call.
#[derive(Default)]
pub struct EventMapper {
    counter: u64,
    current: Option<String>,
    calls: HashMap<String, ToolCall>,
    shared: Shared,
    /// Prompts waiting in Pi's queue, in delivery order.
    queued: Vec<QueuedPrompt>,
    /// The call named by the status line Panoptes' Pi extension sends just
    /// before a `tool_call` hook's dialog (see `approvals.js`).
    approval_for: Option<String>,
}

impl EventMapper {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            ..Self::default()
        }
    }

    /// Translates an `extension_ui_request` (see [`extension_ui`]). A
    /// confirm, or a select between one allow and one reject option, that
    /// directly follows an approval tag is that call's approval. Any other
    /// dialog, including one a tool asks while it runs, is a question.
    pub fn ui_request(&mut self, request: &Value, now: i64) -> Vec<HarnessEvent> {
        if request["method"] == "setStatus" && request["statusKey"] == PERMISSION_GUARD_STATUS {
            self.shared.permission_guard.store(true, Ordering::Relaxed);
            return Vec::new();
        }
        if request["method"] == "setStatus" && request["statusKey"] == APPROVAL_TAG {
            self.approval_for = request["statusText"].as_str().map(str::to_string);
            return Vec::new();
        }
        let mut events = extension_ui(request, now);
        let Some(tool_call_id) = self.approval_for.take() else {
            return events;
        };
        if let [HarnessEvent::Question { question }] = events.as_slice() {
            let reply = match &question.kind {
                QuestionKind::Confirm => Some(None),
                QuestionKind::Select { options } => allow_reject(options).map(Some),
                _ => None,
            };
            if let Some(reply) = reply {
                self.shared
                    .approvals
                    .lock()
                    .unwrap()
                    .insert(question.id.clone(), reply);
                events = vec![HarnessEvent::Approval {
                    approval: Approval {
                        id: question.id.clone(),
                        tool_call_id,
                        title: question.title.clone(),
                        message: question.message.clone(),
                        expires_at: question.expires_at,
                    },
                }];
            }
        }
        events
    }

    /// `now` is the time the event arrived, Unix milliseconds.
    pub fn map(&mut self, event: Value, now: i64) -> Vec<HarnessEvent> {
        self.approval_for = None;
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "agent_start" => vec![HarnessEvent::RunStarted],
            "agent_settled" => {
                let mut events = self.end_unfinished_calls(now);
                events.push(HarnessEvent::RunSettled {
                    aborted: event["aborted"].as_bool().unwrap_or(false),
                });
                events
            }
            "message_start" => {
                self.counter += 1;
                let id = format!("live-{}", self.counter);
                self.current = Some(id.clone());
                match message(&event["message"], id.clone(), None) {
                    Some(started) => {
                        vec![HarnessEvent::MessageStarted {
                            message_id: id,
                            role: started.role,
                        }]
                    }
                    None => Vec::new(),
                }
            }
            "message_update" => self.update(&event["assistantMessageEvent"]),
            "message_end" => {
                let id = self.current.take().unwrap_or_else(|| {
                    self.counter += 1;
                    format!("live-{}", self.counter)
                });
                if let Some(key) = message_key(&event["message"]) {
                    self.shared
                        .live_ids
                        .lock()
                        .unwrap()
                        .entry(key)
                        .or_default()
                        .push(id.clone());
                }
                self.shared.ended_at.lock().unwrap().insert(id.clone(), now);
                message(&event["message"], id, Some(now))
                    .map(|mut message| {
                        // A call that already started keeps what is known of it.
                        for block in &mut message.blocks {
                            if let Block::ToolCall(call) = block
                                && let Some(live) = self.calls.get(&call.id)
                            {
                                **call = live.clone();
                            }
                        }
                        vec![HarnessEvent::MessageCompleted { message }]
                    })
                    .unwrap_or_default()
            }
            "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                self.tool(kind, &event, now)
            }
            "queue_update" => self.queue(&event),
            "session_info_changed" => vec![HarnessEvent::TitleChanged {
                title: event["name"].as_str().map(str::to_string),
            }],
            "auto_retry_start" => vec![HarnessEvent::Notice {
                level: NoticeLevel::Warning,
                text: format!(
                    "Retrying ({}/{}): {}",
                    event["attempt"],
                    event["maxAttempts"],
                    event["errorMessage"].as_str().unwrap_or("request failed")
                ),
            }],
            "auto_retry_end" if event["success"] == false => vec![HarnessEvent::Notice {
                level: NoticeLevel::Error,
                text: event["finalError"]
                    .as_str()
                    .unwrap_or("Retries failed")
                    .to_string(),
            }],
            "compaction_start" => vec![HarnessEvent::Notice {
                level: NoticeLevel::Info,
                text: "Compacting context…".to_string(),
            }],
            "compaction_end" => match event["errorMessage"].as_str() {
                Some(error) => vec![HarnessEvent::Notice {
                    level: NoticeLevel::Error,
                    text: format!("Compaction failed: {error}"),
                }],
                None => Vec::new(),
            },
            "extension_error" => vec![HarnessEvent::Notice {
                level: NoticeLevel::Error,
                text: format!(
                    "Extension error in {}: {}",
                    event["extensionPath"].as_str().unwrap_or("an extension"),
                    event["error"].as_str().unwrap_or_default()
                ),
            }],
            // Lifecycle detail the neutral model does not need.
            "turn_start" | "turn_end" | "agent_end" | "auto_retry_end" => Vec::new(),
            _ => vec![HarnessEvent::Raw { payload: event }],
        }
    }

    /// A run that has settled runs nothing any more. A call Pi never
    /// reported the end of (it was aborted, or the event was lost) is closed
    /// as failed, so it does not show as running forever. Approvals nobody
    /// answered are moot for the same reason.
    fn end_unfinished_calls(&mut self, now: i64) -> Vec<HarnessEvent> {
        self.shared.approvals.lock().unwrap().clear();
        let mut calls: Vec<ToolCall> = self.calls.drain().map(|(_, call)| call).collect();
        calls.sort_by_key(|call| call.started_at);
        calls
            .into_iter()
            .map(|mut call| {
                call.status = ToolStatus::Failed;
                call.ended_at = Some(now);
                if let Some(started) = call.started_at {
                    self.shared
                        .tool_times
                        .lock()
                        .unwrap()
                        .insert(call.id.clone(), (started, call.ended_at));
                }
                HarnessEvent::ToolCallUpdated {
                    call: Box::new(call),
                }
            })
            .collect()
    }

    /// Pi lists its queue by text alone. Each prompt keeps the id it was
    /// first listed under until it leaves the queue, so the UI tracks
    /// prompts by id.
    fn queue(&mut self, event: &Value) -> Vec<HarnessEvent> {
        let mut previous = std::mem::take(&mut self.queued);
        for (key, mode) in [
            ("steering", PromptMode::Steer),
            ("followUp", PromptMode::FollowUp),
        ] {
            for text in event[key].as_array().into_iter().flatten() {
                let Some(text) = text.as_str() else { continue };
                let kept = previous
                    .iter()
                    .position(|q| q.mode == mode && q.text == text)
                    .map(|i| previous.remove(i));
                let prompt = kept.unwrap_or_else(|| {
                    self.counter += 1;
                    QueuedPrompt {
                        id: format!("queued-{}", self.counter),
                        text: text.to_string(),
                        mode,
                    }
                });
                self.queued.push(prompt);
            }
        }
        vec![HarnessEvent::QueueChanged {
            queued: self.queued.clone(),
        }]
    }

    fn tool(&mut self, kind: &str, event: &Value, now: i64) -> Vec<HarnessEvent> {
        let Some(id) = event["toolCallId"].as_str() else {
            return Vec::new();
        };
        let mut call = self.calls.remove(id).unwrap_or_else(|| {
            tool_call(
                id,
                event["toolName"].as_str().unwrap_or_default(),
                event["args"].clone(),
            )
        });
        call.started_at.get_or_insert(now);
        match kind {
            "tool_execution_start" => call.status = ToolStatus::Running,
            "tool_execution_update" => set_output(&mut call, &event["partialResult"], None),
            _ => {
                let is_error = event["isError"].as_bool().unwrap_or(false);
                set_output(&mut call, &event["result"], Some(is_error));
                call.ended_at = Some(now);
            }
        }
        if let Some(started) = call.started_at {
            self.shared
                .tool_times
                .lock()
                .unwrap()
                .insert(call.id.clone(), (started, call.ended_at));
        }
        if call.ended_at.is_none() {
            self.calls.insert(call.id.clone(), call.clone());
        }
        vec![HarnessEvent::ToolCallUpdated {
            call: Box::new(call),
        }]
    }

    fn update(&mut self, update: &Value) -> Vec<HarnessEvent> {
        let Some(message_id) = self.current.clone() else {
            return Vec::new();
        };
        let index = update["contentIndex"].as_u64().unwrap_or(0) as usize;
        let delta = |kind| HarnessEvent::BlockDelta {
            message_id: message_id.clone(),
            index,
            kind,
            delta: update["delta"].as_str().unwrap_or_default().to_string(),
        };
        let started = |block| HarnessEvent::BlockStarted {
            message_id: message_id.clone(),
            index,
            block,
        };
        match update["type"].as_str().unwrap_or_default() {
            "text_start" => vec![started(Block::Text {
                text: String::new(),
            })],
            "thinking_start" => vec![started(Block::Thinking {
                text: String::new(),
            })],
            "toolcall_start" => vec![started(Block::ToolCall(Box::new(tool_call(
                update["id"].as_str().unwrap_or_default(),
                update["toolName"].as_str().unwrap_or_default(),
                Value::Null,
            ))))],
            "text_delta" => vec![delta(DeltaKind::Text)],
            "thinking_delta" => vec![delta(DeltaKind::Thinking)],
            "toolcall_delta" => vec![delta(DeltaKind::ToolArgs)],
            // `*_end` carry authoritative block content, but `message_end`
            // replaces the whole message right after, so they are skipped.
            _ => Vec::new(),
        }
    }
}

/// The allow and reject options of a select, if it offers exactly one of
/// each.
fn allow_reject(options: &[String]) -> Option<(String, String)> {
    let is = |list: &[&str], option: &String| list.contains(&option.trim().to_lowercase().as_str());
    match options {
        [a, b] if is(ALLOW, a) && is(REJECT, b) => Some((a.clone(), b.clone())),
        [a, b] if is(REJECT, a) && is(ALLOW, b) => Some((b.clone(), a.clone())),
        _ => None,
    }
}

/// Translates an `extension_ui_request`. Dialogs become questions the user
/// answers; `notify` becomes a notice; status, widget and title updates have
/// no neutral meaning yet.
pub fn extension_ui(request: &Value, now_ms: i64) -> Vec<HarnessEvent> {
    let text = |key: &str| request[key].as_str().map(str::to_string);
    let kind = match request["method"].as_str().unwrap_or_default() {
        "select" => QuestionKind::Select {
            options: request["options"]
                .as_array()
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "confirm" => QuestionKind::Confirm,
        "input" => QuestionKind::Input {
            placeholder: text("placeholder"),
        },
        "editor" => QuestionKind::Editor {
            prefill: text("prefill"),
        },
        "notify" => {
            let level = match request["notifyType"].as_str() {
                Some("error") => NoticeLevel::Error,
                Some("warning") => NoticeLevel::Warning,
                _ => NoticeLevel::Info,
            };
            return vec![HarnessEvent::Notice {
                level,
                text: text("message").unwrap_or_default(),
            }];
        }
        _ => {
            return vec![HarnessEvent::Raw {
                payload: request.clone(),
            }];
        }
    };
    let Some(id) = text("id") else {
        return Vec::new();
    };
    vec![HarnessEvent::Question {
        question: Question {
            id,
            title: text("title").unwrap_or_else(|| "An extension needs your input".to_string()),
            message: text("message"),
            kind,
            expires_at: request["timeout"].as_i64().map(|timeout| now_ms + timeout),
        },
    }]
}

/// The active branch of a session, oldest first, from `get_entries` data.
/// Tool results are folded into their calls, with the times from `times`.
pub fn history(entries: &[Value], leaf_id: Option<&str>, shared: &SessionState) -> Vec<Message> {
    let by_id: std::collections::HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|entry| Some((entry["id"].as_str()?, entry)))
        .collect();
    let mut path = Vec::new();
    let mut cursor = leaf_id;
    while let Some(id) = cursor {
        let Some(entry) = by_id.get(id) else { break };
        path.push(*entry);
        cursor = entry["parentId"].as_str();
        if path.len() > entries.len() {
            break; // broken parent chain
        }
    }
    path.reverse();
    let results: HashMap<&str, &Value> = path
        .iter()
        .map(|entry| &entry["message"])
        .filter(|message| message["role"] == "toolResult")
        .filter_map(|message| Some((message["toolCallId"].as_str()?, message)))
        .collect();
    let times = shared.tool_times.lock().unwrap();
    let live_ids = shared.live_ids.lock().unwrap();
    let ended_at = shared.ended_at.lock().unwrap();
    let finish = |call: &mut ToolCall| {
        let seen = times.get(&call.id).copied();
        if let Some(result) = results.get(call.id.as_str()) {
            let is_error = result["isError"].as_bool().unwrap_or(false);
            set_output(call, result, Some(is_error));
            call.ended_at = seen
                .and_then(|(_, ended)| ended)
                .or(result["timestamp"].as_i64());
        } else if let Some((_, ended)) = seen {
            // Started, and no result is written: either it still runs, or
            // its run settled without one.
            call.status = match ended {
                None => ToolStatus::Running,
                Some(_) => ToolStatus::Failed,
            };
            call.ended_at = ended;
        }
        call.started_at = seen.map(|(started, _)| started);
    };
    // How many entries with each key came before, so messages that share a
    // key take their live ids in order.
    let mut seen_keys: HashMap<String, usize> = HashMap::new();
    path.into_iter()
        .filter_map(|entry| {
            let id = entry["id"].as_str()?.to_string();
            match entry["type"].as_str()? {
                "message" => {
                    // Keep the id the message streamed under, if it did.
                    let id = message_key(&entry["message"])
                        .and_then(|key| {
                            let nth = seen_keys.entry(key.clone()).or_default();
                            let live = live_ids.get(&key).and_then(|ids| ids.get(*nth));
                            *nth += 1;
                            live.cloned()
                        })
                        .unwrap_or(id);
                    let ended = ended_at
                        .get(&id)
                        .copied()
                        .or_else(|| entry["timestamp"].as_str().and_then(iso_millis));
                    message(&entry["message"], id, ended)
                }
                .map(|mut message| {
                    for block in &mut message.blocks {
                        if let Block::ToolCall(call) = block {
                            finish(call);
                        }
                    }
                    message
                }),
                "compaction" => Some(Message {
                    id,
                    role: Role::Other,
                    blocks: vec![Block::Text {
                        text: "Earlier context was compacted.".to_string(),
                    }],
                    timestamp: None,
                    error: None,
                    stopped: false,
                    usage: None,
                }),
                _ => None,
            }
        })
        .collect()
}

/// A Pi model object. Its id is `<provider>/<model id>`, which
/// [`model_ref`] splits again; model ids may contain slashes, providers not.
pub fn model(value: &Value) -> Option<ModelInfo> {
    let provider = value["provider"].as_str()?;
    let id = value["id"].as_str()?;
    Some(ModelInfo {
        id: format!("{provider}/{id}"),
        name: value["name"].as_str().unwrap_or(id).to_string(),
        provider: provider.to_string(),
        context_window: value["contextWindow"].as_u64(),
        images: value["input"]
            .as_array()
            .is_some_and(|input| input.iter().any(|kind| kind == "image")),
    })
}

/// Splits a [`ModelInfo::id`] back into Pi's provider and model id.
pub fn model_ref(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
        .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
}

/// `contextUsage` from `get_session_stats` data.
pub fn context_usage(stats: &Value) -> Option<ContextUsage> {
    let usage = &stats["contextUsage"];
    Some(ContextUsage {
        tokens: usage["tokens"].as_u64(),
        window: usage["contextWindow"].as_u64()?,
    })
}

#[cfg(test)]
mod tests;
