//! Translating Pi's messages and RPC events into the neutral harness types.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use panoptes_harness::{
    Approval, Block, DeltaKind, HarnessEvent, Message, NoticeLevel, PromptMode, Question,
    QuestionKind, QueuedPrompt, Role, ToolCall, ToolStatus,
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

mod content;
mod message;
mod models;
mod tools;

use message::{iso_millis, message_key};
use tools::{set_output, tool_call};

pub use content::user_text;
pub use message::message;
pub use models::{context_usage, model, model_ref};

#[cfg(test)]
use content::MAX_OUTPUT;
#[cfg(test)]
use panoptes_harness::{FileEdit, StopReason, ToolImage, ToolKind, ToolOutput, Usage};
#[cfg(test)]
use tools::shell_output;

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

#[cfg(test)]
mod tests;
