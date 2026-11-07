//! Translating Pi's messages and RPC events into the neutral harness types.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use panoptes_harness::{
    Block, ContextUsage, DeltaKind, FileEdit, HarnessEvent, Message, ModelInfo, NoticeLevel,
    Question, QuestionKind, Role, ToolCall, ToolKind, ToolOutput, ToolStatus,
};
use serde_json::Value;

/// When each tool call of a session started and ended, as seen live. Pi's
/// history has no such times, so history reloads borrow them from here and
/// give the same values the live events did.
pub type ToolTimes = Arc<Mutex<HashMap<String, (i64, Option<i64>)>>>;

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
    call.status = match finished {
        None => ToolStatus::Running,
        Some(false) => ToolStatus::Done,
        Some(true) => ToolStatus::Failed,
    };
    call.output = match call.kind {
        ToolKind::Execute => Some(shell_output(text, finished)),
        ToolKind::Edit => {
            let edits = match call.output.take() {
                Some(ToolOutput::Edits { edits, .. }) => edits,
                _ => Vec::new(),
            };
            Some(ToolOutput::Edits {
                edits,
                text: (!text.is_empty()).then_some(text),
            })
        }
        _ => (!text.is_empty() || finished.is_some()).then_some(ToolOutput::Text { text }),
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
    ToolOutput::Shell { text, exit_code }
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

/// Converts one Pi `AgentMessage`. System messages (prompt and tool
/// loadout) are not part of the visible conversation, and tool results
/// belong to their calls; both return `None`.
pub fn message(value: &Value, id: String) -> Option<Message> {
    let timestamp = value["timestamp"].as_i64();
    let stopped = value["stopReason"].as_str() == Some("aborted");
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
    times: ToolTimes,
}

impl EventMapper {
    pub fn new(times: ToolTimes) -> Self {
        Self {
            times,
            ..Self::default()
        }
    }

    /// `now` is the time the event arrived, Unix milliseconds.
    pub fn map(&mut self, event: Value, now: i64) -> Vec<HarnessEvent> {
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
                message(&event["message"], id)
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
            self.times
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
pub fn history(entries: &[Value], leaf_id: Option<&str>, times: &ToolTimes) -> Vec<Message> {
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
    let times = times.lock().unwrap();
    let finish = |call: &mut ToolCall| {
        let seen = times.get(&call.id).copied();
        if let Some(result) = results.get(call.id.as_str()) {
            let is_error = result["isError"].as_bool().unwrap_or(false);
            set_output(call, result, Some(is_error));
            call.ended_at = seen
                .and_then(|(_, ended)| ended)
                .or(result["timestamp"].as_i64());
        } else if let Some((_, None)) = seen {
            // Started, not yet finished: the result isn't written yet.
            call.status = ToolStatus::Running;
        }
        call.started_at = seen.map(|(started, _)| started);
    };
    path.into_iter()
        .filter_map(|entry| {
            let id = entry["id"].as_str()?.to_string();
            match entry["type"].as_str()? {
                "message" => message(&entry["message"], id).map(|mut message| {
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn streams_text_under_one_live_id() {
        let mut mapper = EventMapper::default();
        let started = mapper.map(json!({"type":"message_start","message":{"role":"assistant","content":[],"stopReason":"pending"}}), 0);
        assert!(
            matches!(&started[0], HarnessEvent::MessageStarted { message_id, role: Role::Assistant } if message_id == "live-1")
        );
        let delta = mapper.map(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi"}}), 0);
        assert!(
            matches!(&delta[0], HarnessEvent::BlockDelta { message_id, delta, .. } if message_id == "live-1" && delta == "Hi")
        );
        let done = mapper.map(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"stopReason":"stop"}}), 0);
        assert!(
            matches!(&done[0], HarnessEvent::MessageCompleted { message } if message.id == "live-1" && message.error.is_none())
        );
    }

    #[test]
    fn history_follows_the_active_branch() {
        let entries = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"one"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"two"}]}}),
            json!({"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"abandoned"}]}}),
            json!({"type":"session_info","id":"d","parentId":"b","name":"x"}),
        ];
        let ids: Vec<_> = history(&entries, Some("d"), &ToolTimes::default())
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn built_in_tools_get_a_neutral_kind_and_subject() {
        let block = |name: &str, args: Value| {
            let call = tool_call("t", name, args);
            (call.kind, call.subject, call.output)
        };
        let (kind, subject, _) = block("bash", json!({"command": "pnpm test\necho done"}));
        assert_eq!(
            (kind, subject.as_deref()),
            (ToolKind::Execute, Some("pnpm test"))
        );
        let (kind, subject, _) = block("grep", json!({"path": "src", "pattern": "TODO"}));
        assert_eq!((kind, subject.as_deref()), (ToolKind::Search, Some("TODO")));
        let (kind, subject, _) = block("read", json!({"path": "a.ts"}));
        assert_eq!((kind, subject.as_deref()), (ToolKind::Read, Some("a.ts")));
        let (kind, subject, _) = block("my_ext", json!({"limit": 3, "url": "https://x"}));
        assert_eq!(
            (kind, subject.as_deref()),
            (ToolKind::Other, Some("https://x"))
        );
        let (kind, subject, _) = block("bash", Value::Null);
        assert_eq!((kind, subject), (ToolKind::Execute, None));
    }

    #[test]
    fn edits_and_writes_carry_their_file_changes() {
        let edits = |args: Value| match tool_call("t", "edit", args).output {
            Some(ToolOutput::Edits { edits, .. }) => edits,
            other => panic!("expected edits, got {other:?}"),
        };
        let change = |old: &str, new: &str| FileEdit {
            path: "a.ts".into(),
            old_text: Some(old.into()),
            new_text: new.into(),
        };
        assert_eq!(
            edits(
                json!({"path": "a.ts", "edits": [{"oldText": "a", "newText": "b"}, {"oldText": "c", "newText": "d"}]})
            ),
            vec![change("a", "b"), change("c", "d")]
        );
        assert_eq!(
            edits(json!({"path": "a.ts", "edits": "[{\"oldText\":\"a\",\"newText\":\"b\"}]"})),
            vec![change("a", "b")]
        );
        assert_eq!(
            edits(json!({"path": "a.ts", "edits": {"oldText": "a", "newText": "b"}})),
            vec![change("a", "b")]
        );
        assert_eq!(
            edits(json!({"path": "a.ts", "oldText": "a", "newText": "b"})),
            vec![change("a", "b")]
        );
        let write = tool_call("t", "write", json!({"path": "n.ts", "content": "x"}));
        assert_eq!(write.kind, ToolKind::Edit);
        assert_eq!(
            write.output,
            Some(ToolOutput::Edits {
                edits: vec![FileEdit {
                    path: "n.ts".into(),
                    old_text: None,
                    new_text: "x".into()
                }],
                text: None
            })
        );
    }

    fn result(text: &str) -> Value {
        json!({"content": [{"type": "text", "text": text}]})
    }

    fn shell(text: &str, finished: Option<bool>) -> ToolOutput {
        shell_output(text.to_string(), finished)
    }

    #[test]
    fn shell_output_carries_the_exit_code() {
        let shell_ = |text: &str, exit_code| ToolOutput::Shell {
            text: text.into(),
            exit_code,
        };
        assert_eq!(shell("a\nb", Some(false)), shell_("a\nb", Some(0)));
        assert_eq!(
            shell("a\n\nCommand exited with code 2", Some(true)),
            shell_("a", Some(2))
        );
        assert_eq!(
            shell("Command exited with code 1", Some(true)),
            shell_("", Some(1))
        );
        assert_eq!(
            shell("a\n\nCommand timed out after 5 seconds", Some(true)),
            shell_("a\n\nCommand timed out after 5 seconds", None)
        );
        assert_eq!(shell("partial", None), shell_("partial", None));
    }

    /// A bash call from Pi's events: message, start, update, end.
    fn live_call(mapper: &mut EventMapper) -> Vec<HarnessEvent> {
        let call =
            json!({"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"false"}});
        let mut events = Vec::new();
        for (event, now) in [
            (
                json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
                0,
            ),
            (
                json!({"type":"message_end","message":{"role":"assistant","content":[call],"stopReason":"toolUse"}}),
                0,
            ),
            (
                json!({"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"false"}}),
                1_000,
            ),
            (
                json!({"type":"tool_execution_update","toolCallId":"c1","toolName":"bash","partialResult":result("out")}),
                1_500,
            ),
            (
                json!({"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":result("out\n\nCommand exited with code 1"),"isError":true}),
                3_000,
            ),
            (
                json!({"type":"message_start","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[],"isError":true}}),
                3_000,
            ),
            (
                json!({"type":"message_end","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[],"isError":true}}),
                3_000,
            ),
        ] {
            events.extend(mapper.map(event, now));
        }
        events
    }

    #[test]
    fn live_tool_events_carry_the_whole_call() {
        let mut mapper = EventMapper::default();
        let events = live_call(&mut mapper);
        let calls: Vec<&ToolCall> = events
            .iter()
            .filter_map(|event| match event {
                HarnessEvent::ToolCallUpdated { call } => Some(call.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls.iter().map(|c| c.status).collect::<Vec<_>>(),
            [ToolStatus::Running, ToolStatus::Running, ToolStatus::Failed]
        );
        assert!(calls.iter().all(|c| c.kind == ToolKind::Execute
            && c.subject.as_deref() == Some("false")
            && c.input["command"] == "false"));
        let last = calls[2];
        assert_eq!((last.started_at, last.ended_at), (Some(1_000), Some(3_000)));
        assert_eq!(
            last.output,
            Some(ToolOutput::Shell {
                text: "out".into(),
                exit_code: Some(1)
            })
        );
        // The result message is folded into the call, not sent on its own.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, HarnessEvent::MessageCompleted { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn history_gives_the_same_call_as_the_live_events() {
        let times = ToolTimes::default();
        let mut mapper = EventMapper::new(times.clone());
        let events = live_call(&mut mapper);
        let Some(HarnessEvent::ToolCallUpdated { call: live }) = events
            .iter()
            .rfind(|e| matches!(e, HarnessEvent::ToolCallUpdated { .. }))
        else {
            panic!("no tool call in {events:?}");
        };
        let entries = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"false"}}],"stopReason":"toolUse"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[{"type":"text","text":"out\n\nCommand exited with code 1"}],"isError":true,"timestamp":9_999}}),
        ];
        let messages = history(&entries, Some("b"), &times);
        assert_eq!(messages.len(), 1, "the result is not a message of its own");
        assert_eq!(messages[0].blocks, vec![Block::ToolCall(live.clone())]);

        // Without live times (a session opened later), the result's own
        // timestamp is the end.
        let cold = history(&entries, Some("b"), &ToolTimes::default());
        let Block::ToolCall(call) = &cold[0].blocks[0] else {
            unreachable!()
        };
        assert_eq!(
            (call.status, call.started_at, call.ended_at),
            (ToolStatus::Failed, None, Some(9_999))
        );
    }

    #[test]
    fn edits_keep_their_changes_when_they_finish() {
        let mut call = tool_call(
            "e",
            "edit",
            json!({"path": "a.ts", "edits": [{"oldText": "a", "newText": "b"}]}),
        );
        set_output(&mut call, &result("Could not find the text"), Some(true));
        let Some(ToolOutput::Edits { edits, text }) = call.output else {
            panic!("expected edits");
        };
        assert_eq!(
            (edits.len(), text.as_deref()),
            (1, Some("Could not find the text"))
        );
        assert_eq!(call.status, ToolStatus::Failed);
    }

    #[test]
    fn aborted_messages_are_stopped_not_failed() {
        let stopped = message(
            &json!({"role":"assistant","content":[],"stopReason":"aborted"}),
            "m".into(),
        )
        .unwrap();
        assert!(stopped.stopped && stopped.error.is_none());
    }

    #[test]
    fn hides_system_messages() {
        assert!(message(&json!({"role":"system","content":""}), "s".into()).is_none());
    }

    #[test]
    fn dialogs_become_questions_with_a_deadline() {
        let events = extension_ui(
            &json!({"type":"extension_ui_request","id":"q1","method":"select","title":"Allow?","options":["Allow","Block"],"timeout":10000}),
            1_000,
        );
        let [HarnessEvent::Question { question }] = events.as_slice() else {
            panic!("expected one question, got {events:?}");
        };
        assert_eq!(question.id, "q1");
        assert_eq!(question.expires_at, Some(11_000));
        assert_eq!(
            question.kind,
            QuestionKind::Select {
                options: vec!["Allow".into(), "Block".into()]
            }
        );
    }

    #[test]
    fn notify_becomes_a_notice() {
        let events = extension_ui(
            &json!({"type":"extension_ui_request","id":"n","method":"notify","message":"Blocked","notifyType":"warning"}),
            0,
        );
        assert_eq!(
            events,
            vec![HarnessEvent::Notice {
                level: NoticeLevel::Warning,
                text: "Blocked".into()
            }]
        );
    }

    #[test]
    fn models_round_trip_through_their_id() {
        let info = model(&json!({
            "id": "anthropic/claude-x",
            "name": "Claude X",
            "provider": "openrouter",
            "contextWindow": 200000,
            "input": ["text", "image"]
        }))
        .unwrap();
        assert_eq!(info.id, "openrouter/anthropic/claude-x");
        assert_eq!(info.context_window, Some(200_000));
        assert!(info.images);
        assert_eq!(
            model_ref(&info.id),
            Some(("openrouter", "anthropic/claude-x"))
        );
        assert_eq!(model_ref("no-provider"), None);
    }

    #[test]
    fn context_usage_needs_a_window() {
        let known = json!({"contextUsage": {"tokens": 60000, "contextWindow": 200000}});
        assert_eq!(
            context_usage(&known),
            Some(ContextUsage {
                tokens: Some(60_000),
                window: 200_000
            })
        );
        let compacted = json!({"contextUsage": {"tokens": null, "contextWindow": 200000}});
        assert_eq!(context_usage(&compacted).unwrap().tokens, None);
        assert_eq!(context_usage(&json!({})), None);
    }
}
