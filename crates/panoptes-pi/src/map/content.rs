use panoptes_harness::{Block, ToolImage};
use serde_json::Value;

use super::tools::tool_call;

/// Tool output beyond this many bytes is cut for display.
pub(super) const MAX_OUTPUT: usize = 64 * 1024;

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
pub(super) fn truncate(mut text: String) -> String {
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
pub(super) fn truncate_start(text: String) -> String {
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
pub(super) fn result_images(result: &Value) -> Vec<ToolImage> {
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
pub(super) fn result_text(result: &Value) -> String {
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

pub(super) fn content_blocks(content: &Value) -> Vec<Block> {
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
