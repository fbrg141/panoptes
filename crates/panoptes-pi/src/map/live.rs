use std::collections::HashMap;

use panoptes_harness::{
    Block, DeltaKind, HarnessEvent, NoticeLevel, PromptMode, QueuedPrompt, ToolCall, ToolStatus,
};
use serde_json::Value;

use super::Shared;
use super::message::{message, message_key};
use super::tools::{set_output, tool_call};

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
    pub(super) shared: Shared,
    /// Prompts waiting in Pi's queue, in delivery order.
    queued: Vec<QueuedPrompt>,
    /// The call named by the status line Panoptes' Pi extension sends just
    /// before a `tool_call` hook's dialog (see `approvals.js`).
    pub(super) approval_for: Option<String>,
}

impl EventMapper {
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            ..Self::default()
        }
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
