use panoptes_harness::{Block, Message, Role, StopReason, Usage};
use serde_json::Value;

use super::content::{content_blocks, truncate, user_text};

/// What identifies a Pi message both live and in its session file: its role
/// and timestamp, or, for a message Pi did not stamp, its role and content.
pub(super) fn message_key(message: &Value) -> Option<String> {
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
pub(super) fn iso_millis(text: &str) -> Option<i64> {
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
