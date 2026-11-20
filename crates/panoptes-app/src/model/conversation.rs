//! Pure reducer: harness events folded into conversation state.

use std::collections::HashMap;

use panoptes_harness::{
    Approval, Block, DeltaKind, HarnessEvent, Message, NoticeLevel, Question, QueuedPrompt,
    ToolCall,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub level: NoticeLevel,
    pub text: String,
}

/// The thinking block streaming now, and when it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thinking {
    pub index: usize,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    pub messages: Vec<Message>,
    pub busy: bool,
    pub notices: Vec<Notice>,
    /// Questions from the harness still waiting for an answer.
    pub questions: Vec<Question>,
    /// Tool calls waiting for the user to allow or reject them.
    pub approvals: Vec<Approval>,
    /// Set once the harness process has ended.
    pub exited: Option<String>,
    /// Steers and follow-ups sent during a run that the agent has not read yet.
    pub queued: Vec<QueuedPrompt>,
    /// How long each thinking block took, in ms, keyed by its trimmed text:
    /// harnesses do not report it, and the text survives the swap to stable ids.
    pub thought_for: HashMap<String, i64>,
    /// The streaming thinking block: its message id, index and start.
    pub thinking: Option<(String, Thinking)>,
    /// Tool-call arguments as they stream, by tool call id.
    pub args_text: HashMap<String, String>,
}

/// Notices kept per conversation; older ones scroll away.
const MAX_NOTICES: usize = 5;

impl Conversation {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Self::default()
        }
    }

    fn message_mut(&mut self, id: &str) -> Option<&mut Message> {
        self.messages.iter_mut().find(|m| m.id == id)
    }

    fn block(&self, id: &str, index: usize) -> Option<&Block> {
        self.messages
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.blocks.get(index))
    }

    /// Sets a block, filling any gap below it with empty text: a block that
    /// arrives ahead of the ones before it must not leave holes.
    fn update_block(
        &mut self,
        id: &str,
        index: usize,
        update: impl FnOnce(Option<Block>) -> Block,
    ) {
        if let Some(message) = self.message_mut(id) {
            while message.blocks.len() < index {
                message.blocks.push(Block::Text {
                    text: String::new(),
                });
            }
            let current = message.blocks.get(index).cloned();
            let next = update(current);
            if index < message.blocks.len() {
                message.blocks[index] = next;
            } else {
                message.blocks.push(next);
            }
        }
    }

    /// Records how long the streaming thinking block took, once another block starts.
    fn end_thinking(&mut self, message_id: &str, now: i64) {
        let Some((id, thinking)) = &self.thinking else {
            return;
        };
        if id != message_id {
            return;
        }
        if let Some(Block::Thinking { text }) = self.block(id, thinking.index) {
            let text = text.trim();
            if !text.is_empty() {
                self.thought_for.insert(text.to_string(), now - thinking.at);
            }
        }
        self.thinking = None;
    }

    /// Folds one harness event in. `now` is Unix milliseconds.
    pub fn apply(&mut self, event: &HarnessEvent, now: i64) {
        match event {
            HarnessEvent::RunStarted => {
                self.busy = true;
                self.exited = None;
            }
            HarnessEvent::MessageStarted { message_id, role } => {
                // A repeated start must not add the message a second time.
                if self.messages.iter().any(|m| &m.id == message_id) {
                    return;
                }
                self.messages.push(Message {
                    id: message_id.clone(),
                    role: *role,
                    blocks: Vec::new(),
                    timestamp: Some(now),
                    error: None,
                    stopped: false,
                    usage: None,
                });
            }
            HarnessEvent::BlockStarted {
                message_id,
                index,
                block,
            } => {
                self.end_thinking(message_id, now);
                let exists = self.messages.iter().any(|m| &m.id == message_id);
                self.update_block(message_id, *index, |_| block.clone());
                if exists && matches!(block, Block::Thinking { .. }) {
                    self.thinking = Some((
                        message_id.clone(),
                        Thinking {
                            index: *index,
                            at: now,
                        },
                    ));
                }
            }
            HarnessEvent::BlockDelta {
                message_id,
                index,
                kind,
                delta,
            } => {
                let target = self.block(message_id, *index).cloned();
                if *kind == DeltaKind::ToolArgs {
                    // Tool arguments only belong on a tool call; anywhere
                    // else they would show up as prose.
                    if let Some(Block::ToolCall(call)) = target {
                        self.args_text
                            .entry(call.id.clone())
                            .or_default()
                            .push_str(delta);
                    }
                    return;
                }
                self.update_block(message_id, *index, |block| match block {
                    Some(Block::Text { text }) => Block::Text { text: text + delta },
                    Some(Block::Thinking { text }) => Block::Thinking { text: text + delta },
                    _ if *kind == DeltaKind::Thinking => Block::Thinking {
                        text: delta.clone(),
                    },
                    _ => Block::Text {
                        text: delta.clone(),
                    },
                });
            }
            HarnessEvent::MessageCompleted { message } => {
                self.end_thinking(&message.id, now);
                match self.message_mut(&message.id) {
                    Some(existing) => *existing = message.clone(),
                    None => self.messages.push(message.clone()),
                }
            }
            HarnessEvent::ToolCallUpdated { call } => self.update_tool_call(call),
            HarnessEvent::QueueChanged { queued } => self.queued = queued.clone(),
            HarnessEvent::RunSettled { .. } => {
                // Nothing waits once the agent stops, as the service's status says too.
                self.busy = false;
                self.queued.clear();
                self.thinking = None;
                self.questions.clear();
                self.approvals.clear();
            }
            HarnessEvent::Notice { level, text } => {
                if self.notices.len() >= MAX_NOTICES {
                    let drop = self.notices.len() + 1 - MAX_NOTICES;
                    self.notices.drain(..drop);
                }
                self.notices.push(Notice {
                    level: *level,
                    text: text.clone(),
                });
            }
            HarnessEvent::Question { question } => {
                self.questions.retain(|q| q.id != question.id);
                self.questions.push(question.clone());
            }
            HarnessEvent::Approval { approval } => {
                self.approvals.retain(|a| a.id != approval.id);
                self.approvals.push(approval.clone());
            }
            HarnessEvent::Exited { code, stderr_tail } => {
                self.busy = false;
                self.questions.clear();
                self.approvals.clear();
                self.queued.clear();
                let tail = stderr_tail.trim();
                self.exited = Some(if tail.is_empty() {
                    format!(
                        "The agent process ended (code {}).",
                        code.map_or("none".to_string(), |c| c.to_string())
                    )
                } else {
                    tail.to_string()
                });
            }
            HarnessEvent::TitleChanged { .. } | HarnessEvent::Raw { .. } => {}
        }
    }

    /// Swaps in the latest state of a tool call; its streamed argument text stays.
    fn update_tool_call(&mut self, call: &ToolCall) {
        for message in &mut self.messages {
            for block in &mut message.blocks {
                if let Block::ToolCall(existing) = block
                    && existing.id == call.id
                {
                    **existing = call.clone();
                    return;
                }
            }
        }
    }

    /// Drops an answered question or approval.
    pub fn remove_question(&mut self, id: &str) {
        self.questions.retain(|q| q.id != id);
        self.approvals.retain(|a| a.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::{PromptMode, QuestionKind, Role, ToolKind, ToolOutput, ToolStatus};

    fn fold(events: &[HarnessEvent]) -> Conversation {
        let mut c = Conversation::default();
        for event in events {
            c.apply(event, 0);
        }
        c
    }

    fn started(id: &str) -> HarnessEvent {
        HarnessEvent::MessageStarted {
            message_id: id.into(),
            role: Role::Assistant,
        }
    }

    fn delta(id: &str, index: usize, kind: DeltaKind, text: &str) -> HarnessEvent {
        HarnessEvent::BlockDelta {
            message_id: id.into(),
            index,
            kind,
            delta: text.into(),
        }
    }

    fn text(t: &str) -> Block {
        Block::Text { text: t.into() }
    }

    #[test]
    fn ignores_a_repeated_message_start() {
        let c = fold(&[started("m1"), started("m1")]);
        assert_eq!(c.messages.len(), 1);
    }

    #[test]
    fn drops_tool_arguments_aimed_at_a_block_that_is_not_a_tool_call() {
        let c = fold(&[
            started("m1"),
            HarnessEvent::BlockStarted {
                message_id: "m1".into(),
                index: 0,
                block: text("Hi"),
            },
            delta("m1", 0, DeltaKind::ToolArgs, "{\"path\":"),
            delta("m1", 3, DeltaKind::ToolArgs, "{\"path\":"),
        ]);
        assert_eq!(c.messages[0].blocks, vec![text("Hi")]);
        assert!(c.args_text.is_empty());
    }

    #[test]
    fn fills_the_gap_when_a_block_arrives_ahead_of_the_ones_before_it() {
        let c = fold(&[started("m1"), delta("m1", 2, DeltaKind::Text, "late")]);
        assert_eq!(c.messages[0].blocks, vec![text(""), text(""), text("late")]);
    }

    #[test]
    fn clears_questions_when_the_run_settles_and_keeps_one_card_per_id() {
        let question = Question {
            id: "q1".into(),
            title: "Ok?".into(),
            message: None,
            kind: QuestionKind::Confirm,
            expires_at: None,
        };
        let mut c = fold(&[
            HarnessEvent::RunStarted,
            HarnessEvent::Question {
                question: question.clone(),
            },
            HarnessEvent::Question { question },
        ]);
        assert_eq!(c.questions.len(), 1);
        c.apply(&HarnessEvent::RunSettled { aborted: false }, 0);
        assert!(c.questions.is_empty());
        assert!(c.approvals.is_empty());
    }

    #[test]
    fn shows_the_queue_and_clears_it_once_the_run_settles() {
        let queued = vec![
            QueuedPrompt {
                id: "q1".into(),
                text: "a".into(),
                mode: PromptMode::Steer,
            },
            QueuedPrompt {
                id: "q2".into(),
                text: "a".into(),
                mode: PromptMode::FollowUp,
            },
        ];
        let mut c = fold(&[
            HarnessEvent::RunStarted,
            HarnessEvent::QueueChanged {
                queued: queued.clone(),
            },
        ]);
        assert_eq!(c.queued, queued);
        c.apply(
            &HarnessEvent::QueueChanged {
                queued: vec![queued[1].clone()],
            },
            0,
        );
        assert_eq!(c.queued.len(), 1);
        c.apply(&HarnessEvent::RunSettled { aborted: true }, 0);
        assert!(c.queued.is_empty());
    }

    #[test]
    fn streams_text_and_replaces_it_with_the_completed_message() {
        let mut c = fold(&[
            HarnessEvent::RunStarted,
            started("live-1"),
            HarnessEvent::BlockStarted {
                message_id: "live-1".into(),
                index: 0,
                block: text(""),
            },
            delta("live-1", 0, DeltaKind::Text, "Hel"),
            delta("live-1", 0, DeltaKind::Text, "lo"),
        ]);
        assert!(c.busy);
        assert_eq!(c.messages[0].blocks[0], text("Hello"));
        c.apply(
            &HarnessEvent::MessageCompleted {
                message: Message {
                    id: "live-1".into(),
                    role: Role::Assistant,
                    blocks: vec![text("Hello!")],
                    timestamp: Some(1),
                    error: None,
                    stopped: false,
                    usage: None,
                },
            },
            0,
        );
        c.apply(&HarnessEvent::RunSettled { aborted: false }, 0);
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].blocks[0], text("Hello!"));
        assert!(!c.busy);
    }

    #[test]
    fn holds_approvals_until_decided_or_the_process_ends() {
        let approval = Approval {
            id: "a1".into(),
            tool_call_id: "c1".into(),
            title: "Allow?".into(),
            message: None,
            expires_at: None,
        };
        let c = fold(&[HarnessEvent::Approval {
            approval: approval.clone(),
        }]);
        assert_eq!(c.approvals, vec![approval]);
        let mut removed = c.clone();
        removed.remove_question("a1");
        assert!(removed.approvals.is_empty());
        let mut exited = c;
        exited.apply(
            &HarnessEvent::Exited {
                code: Some(1),
                stderr_tail: String::new(),
            },
            0,
        );
        assert!(exited.approvals.is_empty());
        assert_eq!(
            exited.exited.as_deref(),
            Some("The agent process ended (code 1).")
        );
    }

    #[test]
    fn updates_a_tool_call_in_place_keeping_its_streamed_arguments() {
        let pending = ToolCall {
            id: "c1".into(),
            name: "bash".into(),
            kind: ToolKind::Execute,
            subject: None,
            input: serde_json::Value::Null,
            status: ToolStatus::Pending,
            output: None,
            images: Vec::new(),
            started_at: None,
            ended_at: None,
        };
        let mut c = fold(&[
            started("live-1"),
            HarnessEvent::BlockStarted {
                message_id: "live-1".into(),
                index: 0,
                block: Block::ToolCall(Box::new(pending.clone())),
            },
            delta("live-1", 0, DeltaKind::ToolArgs, "{\"command\""),
        ]);
        let done = ToolCall {
            subject: Some("ls".into()),
            status: ToolStatus::Done,
            output: Some(ToolOutput::Shell {
                text: "a".into(),
                exit_code: Some(0),
            }),
            started_at: Some(1000),
            ended_at: Some(2000),
            ..pending
        };
        c.apply(
            &HarnessEvent::ToolCallUpdated {
                call: Box::new(done.clone()),
            },
            0,
        );
        assert_eq!(
            c.messages[0].blocks[0],
            Block::ToolCall(Box::new(done.clone()))
        );
        assert_eq!(c.args_text["c1"], "{\"command\"");
        let before = c.clone();
        c.apply(
            &HarnessEvent::ToolCallUpdated {
                call: Box::new(ToolCall {
                    id: "other".into(),
                    ..done
                }),
            },
            0,
        );
        assert_eq!(c, before);
    }

    #[test]
    fn times_a_thinking_block_until_the_next_block_starts() {
        let mut c = fold(&[
            started("live-1"),
            HarnessEvent::BlockStarted {
                message_id: "live-1".into(),
                index: 0,
                block: Block::Thinking {
                    text: String::new(),
                },
            },
            delta("live-1", 0, DeltaKind::Thinking, "Hmm."),
        ]);
        assert_eq!(
            c.thinking,
            Some(("live-1".to_string(), Thinking { index: 0, at: 0 }))
        );
        c.apply(
            &HarnessEvent::BlockStarted {
                message_id: "live-1".into(),
                index: 1,
                block: text(""),
            },
            8000,
        );
        assert_eq!(c.thinking, None);
        assert_eq!(c.thought_for.get("Hmm."), Some(&8000));
    }
}
