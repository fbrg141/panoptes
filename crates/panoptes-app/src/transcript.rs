//! Pure transcript state: harness events folded into the messages the
//! session pane shows. The walking skeleton keeps text and one line per tool
//! call; markdown, thinking and images come with the transcript slice.

use panoptes_harness::{Block, DeltaKind, HarnessEvent, Message, Role, ToolCall};

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    Tool {
        id: String,
        label: String,
    },
    /// Thinking and images, which the skeleton does not show yet.
    Hidden,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub role: Role,
    pub pieces: Vec<Piece>,
    pub error: Option<String>,
}

impl Entry {
    /// The entry as plain text: text pieces, then one line per tool call.
    pub fn text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) if !text.is_empty() => parts.push(text),
                Piece::Tool { label, .. } => parts.push(label),
                _ => {}
            }
        }
        if let Some(error) = &self.error {
            parts.push(error);
        }
        parts.join("\n\n")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub entries: Vec<Entry>,
}

impl Transcript {
    pub fn from_history(history: &[Message]) -> Self {
        Self {
            entries: history.iter().map(entry).collect(),
        }
    }

    /// Applies one live event. `RunSettled` is not handled here: the caller
    /// reloads the history, which is authoritative.
    pub fn apply(&mut self, event: &HarnessEvent) {
        match event {
            HarnessEvent::MessageStarted { message_id, role } => {
                if self.find(message_id).is_none() {
                    self.entries.push(Entry {
                        id: message_id.clone(),
                        role: *role,
                        pieces: Vec::new(),
                        error: None,
                    });
                }
            }
            HarnessEvent::BlockStarted {
                message_id,
                index,
                block,
            } => {
                if let Some(entry) = self.find(message_id) {
                    if entry.pieces.len() <= *index {
                        entry.pieces.resize(*index + 1, Piece::Hidden);
                    }
                    entry.pieces[*index] = piece(block);
                }
            }
            HarnessEvent::BlockDelta {
                message_id,
                index,
                kind: DeltaKind::Text,
                delta,
            } => {
                if let Some(Piece::Text(text)) = self
                    .find(message_id)
                    .and_then(|entry| entry.pieces.get_mut(*index))
                {
                    text.push_str(delta);
                }
            }
            HarnessEvent::MessageCompleted { message } => {
                let completed = entry(message);
                match self.find(&message.id) {
                    Some(existing) => *existing = completed,
                    None => self.entries.push(completed),
                }
            }
            HarnessEvent::ToolCallUpdated { call } => {
                for entry in &mut self.entries {
                    for piece in &mut entry.pieces {
                        if let Piece::Tool { id, label } = piece
                            && *id == call.id
                        {
                            *label = tool_label(call);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn find(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.id == id)
    }
}

fn entry(message: &Message) -> Entry {
    Entry {
        id: message.id.clone(),
        role: message.role,
        pieces: message.blocks.iter().map(piece).collect(),
        error: message.error.clone(),
    }
}

fn piece(block: &Block) -> Piece {
    match block {
        Block::Text { text } => Piece::Text(text.clone()),
        Block::ToolCall(call) => Piece::Tool {
            id: call.id.clone(),
            label: tool_label(call),
        },
        Block::Thinking { .. } | Block::Image { .. } => Piece::Hidden,
    }
}

fn tool_label(call: &ToolCall) -> String {
    match &call.subject {
        Some(subject) => format!("{} {subject}", call.name),
        None => call.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::{ToolKind, ToolStatus};

    fn message(id: &str, role: Role, text: &str) -> Message {
        Message {
            id: id.to_string(),
            role,
            blocks: vec![Block::Text {
                text: text.to_string(),
            }],
            timestamp: None,
            error: None,
            stopped: false,
            usage: None,
        }
    }

    fn tool(id: &str, subject: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: "read".to_string(),
            kind: ToolKind::Read,
            subject: Some(subject.to_string()),
            input: serde_json::Value::Null,
            status: ToolStatus::Running,
            output: None,
            images: Vec::new(),
            started_at: None,
            ended_at: None,
        }
    }

    #[test]
    fn history_becomes_entries() {
        let transcript = Transcript::from_history(&[
            message("1", Role::User, "hi"),
            message("2", Role::Assistant, "hello"),
        ]);
        let texts: Vec<_> = transcript.entries.iter().map(Entry::text).collect();
        assert_eq!(texts, ["hi", "hello"]);
    }

    #[test]
    fn streamed_text_is_replaced_by_the_completed_message() {
        let mut transcript = Transcript::default();
        transcript.apply(&HarnessEvent::MessageStarted {
            message_id: "m".into(),
            role: Role::Assistant,
        });
        transcript.apply(&HarnessEvent::BlockStarted {
            message_id: "m".into(),
            index: 0,
            block: Block::Text {
                text: String::new(),
            },
        });
        for delta in ["Hel", "lo"] {
            transcript.apply(&HarnessEvent::BlockDelta {
                message_id: "m".into(),
                index: 0,
                kind: DeltaKind::Text,
                delta: delta.into(),
            });
        }
        assert_eq!(transcript.entries[0].text(), "Hello");

        transcript.apply(&HarnessEvent::MessageCompleted {
            message: message("m", Role::Assistant, "Hello there"),
        });
        assert_eq!(transcript.entries.len(), 1);
        assert_eq!(transcript.entries[0].text(), "Hello there");
    }

    #[test]
    fn tool_calls_update_in_place() {
        let mut transcript = Transcript::default();
        transcript.apply(&HarnessEvent::MessageCompleted {
            message: Message {
                blocks: vec![Block::ToolCall(Box::new(tool("t", "a.rs")))],
                ..message("m", Role::Assistant, "")
            },
        });
        transcript.apply(&HarnessEvent::ToolCallUpdated {
            call: Box::new(tool("t", "b.rs")),
        });
        assert_eq!(transcript.entries[0].text(), "read b.rs");
    }

    #[test]
    fn thinking_deltas_do_not_show_as_text() {
        let mut transcript = Transcript::from_history(&[message("m", Role::Assistant, "")]);
        transcript.apply(&HarnessEvent::BlockStarted {
            message_id: "m".into(),
            index: 1,
            block: Block::Thinking {
                text: String::new(),
            },
        });
        transcript.apply(&HarnessEvent::BlockDelta {
            message_id: "m".into(),
            index: 1,
            kind: DeltaKind::Thinking,
            delta: "hmm".into(),
        });
        assert_eq!(transcript.entries[0].text(), "");
    }
}
