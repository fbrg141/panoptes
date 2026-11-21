use std::collections::HashMap;

use panoptes_harness::{Block, Message, Role, ToolCall, ToolStatus};
use serde_json::Value;

use super::SessionState;
use super::message::{iso_millis, message, message_key};
use super::tools::set_output;

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
