//! Translating Pi's messages and RPC events into the neutral harness types.

use panoptes_harness::{Block, DeltaKind, HarnessEvent, Message, NoticeLevel, Role};
use serde_json::Value;

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

/// Joined text of a tool result's `content` blocks.
fn result_text(result: &Value) -> String {
    let text = result["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    truncate(text)
}

fn content_blocks(content: &Value) -> Vec<Block> {
    let Some(blocks) = content.as_array() else {
        return user_text(content).map(|text| vec![Block::Text { text }]).unwrap_or_default();
    };
    blocks
        .iter()
        .filter_map(|block| match block["type"].as_str()? {
            "text" => Some(Block::Text { text: block["text"].as_str()?.to_string() }),
            "thinking" => Some(Block::Thinking {
                text: block["thinking"].as_str().unwrap_or_default().to_string(),
            }),
            "toolCall" => Some(Block::ToolCall {
                call_id: block["id"].as_str()?.to_string(),
                name: block["name"].as_str()?.to_string(),
                args: block["arguments"].clone(),
            }),
            "image" => Some(Block::Image {
                mime_type: block["mimeType"].as_str().unwrap_or_default().to_string(),
            }),
            _ => None,
        })
        .collect()
}

/// Converts one Pi `AgentMessage`. System messages (prompt and tool
/// loadout) are not part of the visible conversation and return `None`.
pub fn message(value: &Value, id: String) -> Option<Message> {
    let timestamp = value["timestamp"].as_i64();
    let (role, blocks, error) = match value["role"].as_str()? {
        "system" => return None,
        "user" => (Role::User, content_blocks(&value["content"]), None),
        "assistant" => {
            let error = match value["stopReason"].as_str() {
                Some("error") => Some(
                    value["errorMessage"].as_str().unwrap_or("The model request failed").to_string(),
                ),
                Some("aborted") => Some("Stopped".to_string()),
                _ => None,
            };
            (Role::Assistant, content_blocks(&value["content"]), error)
        }
        "toolResult" => (
            Role::Tool,
            vec![Block::ToolResult {
                call_id: value["toolCallId"].as_str().unwrap_or_default().to_string(),
                name: value["toolName"].as_str().unwrap_or_default().to_string(),
                output: result_text(value),
                is_error: value["isError"].as_bool().unwrap_or(false),
            }],
            None,
        ),
        "bashExecution" => {
            let command = value["command"].as_str().unwrap_or_default();
            let output = value["output"].as_str().unwrap_or_default();
            (
                Role::Other,
                vec![Block::Text { text: truncate(format!("$ {command}\n{output}")) }],
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
    Some(Message { id, role, blocks, timestamp, error })
}

/// Turns the Pi event stream into harness events. Pi does not give
/// streaming messages an id, so each one gets a live id here; history
/// reloads later replace them with stable entry ids.
#[derive(Default)]
pub struct EventMapper {
    counter: u64,
    current: Option<String>,
}

impl EventMapper {
    pub fn map(&mut self, event: Value) -> Vec<HarnessEvent> {
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "agent_start" => vec![HarnessEvent::RunStarted],
            "agent_settled" => vec![HarnessEvent::RunSettled {
                aborted: event["aborted"].as_bool().unwrap_or(false),
            }],
            "message_start" => {
                self.counter += 1;
                let id = format!("live-{}", self.counter);
                self.current = Some(id.clone());
                match message(&event["message"], id.clone()) {
                    Some(started) => {
                        vec![HarnessEvent::MessageStarted { message_id: id, role: started.role }]
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
                message(&event["message"], id)
                    .map(|message| vec![HarnessEvent::MessageCompleted { message }])
                    .unwrap_or_default()
            }
            "tool_execution_start" => vec![HarnessEvent::ToolStarted {
                call_id: event["toolCallId"].as_str().unwrap_or_default().to_string(),
                name: event["toolName"].as_str().unwrap_or_default().to_string(),
                args: event["args"].clone(),
            }],
            "tool_execution_update" => vec![HarnessEvent::ToolUpdated {
                call_id: event["toolCallId"].as_str().unwrap_or_default().to_string(),
                output: result_text(&event["partialResult"]),
            }],
            "tool_execution_end" => vec![HarnessEvent::ToolFinished {
                call_id: event["toolCallId"].as_str().unwrap_or_default().to_string(),
                output: result_text(&event["result"]),
                is_error: event["isError"].as_bool().unwrap_or(false),
            }],
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
                text: event["finalError"].as_str().unwrap_or("Retries failed").to_string(),
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
        let started = |block| HarnessEvent::BlockStarted { message_id: message_id.clone(), index, block };
        match update["type"].as_str().unwrap_or_default() {
            "text_start" => vec![started(Block::Text { text: String::new() })],
            "thinking_start" => vec![started(Block::Thinking { text: String::new() })],
            "toolcall_start" => vec![started(Block::ToolCall {
                call_id: update["id"].as_str().unwrap_or_default().to_string(),
                name: update["toolName"].as_str().unwrap_or_default().to_string(),
                args: Value::Null,
            })],
            "text_delta" => vec![delta(DeltaKind::Text)],
            "thinking_delta" => vec![delta(DeltaKind::Thinking)],
            "toolcall_delta" => vec![delta(DeltaKind::ToolArgs)],
            // `*_end` carry authoritative block content, but `message_end`
            // replaces the whole message right after, so they are skipped.
            _ => Vec::new(),
        }
    }
}

/// The active branch of a session, oldest first, from `get_entries` data.
pub fn history(entries: &[Value], leaf_id: Option<&str>) -> Vec<Message> {
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
    path.into_iter()
        .filter_map(|entry| {
            let id = entry["id"].as_str()?.to_string();
            match entry["type"].as_str()? {
                "message" => message(&entry["message"], id),
                "compaction" => Some(Message {
                    id,
                    role: Role::Other,
                    blocks: vec![Block::Text { text: "Earlier context was compacted.".to_string() }],
                    timestamp: None,
                    error: None,
                }),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn streams_text_under_one_live_id() {
        let mut mapper = EventMapper::default();
        let started = mapper.map(json!({"type":"message_start","message":{"role":"assistant","content":[],"stopReason":"pending"}}));
        assert!(matches!(&started[0], HarnessEvent::MessageStarted { message_id, role: Role::Assistant } if message_id == "live-1"));
        let delta = mapper.map(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi"}}));
        assert!(matches!(&delta[0], HarnessEvent::BlockDelta { message_id, delta, .. } if message_id == "live-1" && delta == "Hi"));
        let done = mapper.map(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"stopReason":"stop"}}));
        assert!(matches!(&done[0], HarnessEvent::MessageCompleted { message } if message.id == "live-1" && message.error.is_none()));
    }

    #[test]
    fn history_follows_the_active_branch() {
        let entries = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"one"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"two"}]}}),
            json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"abandoned"}]}}),
            json!({"type":"session_info","id":"d","parentId":"b","name":"x"}),
        ];
        let ids: Vec<_> = history(&entries, Some("d")).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn hides_system_messages() {
        assert!(message(&json!({"role":"system","content":""}), "s".into()).is_none());
    }
}
