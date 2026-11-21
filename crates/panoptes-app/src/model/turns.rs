//! Pure transcript logic: messages grouped into turns, tool rows and
//! folded-work summaries.

use panoptes_harness::{
    Approval, Block, FileEdit, Message, Role, ToolCall, ToolImage, ToolKind, ToolOutput, ToolStatus,
};

use super::conversation::Conversation;
use super::format::plural;
use super::usage::{UsageTotal, sum_usage};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolStep {
    pub key: String,
    pub call: ToolCall,
    pub duration_ms: Option<i64>,
    /// Set while the call waits for the user to allow or reject it.
    pub approval: Option<Approval>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Step {
    Text {
        key: String,
        text: String,
    },
    Thinking {
        key: String,
        text: String,
        duration_ms: Option<i64>,
        live: bool,
    },
    Tool(ToolStep),
    /// Consecutive read and search calls, shown as one row.
    Group {
        key: String,
        steps: Vec<ToolStep>,
    },
    Note {
        key: String,
        text: String,
    },
    Image {
        key: String,
        mime_type: String,
        data: Option<String>,
    },
    Error {
        key: String,
        text: String,
    },
}

impl Step {
    fn is_answer(&self) -> bool {
        matches!(self, Step::Text { .. } | Step::Error { .. })
    }
}

/// One prompt and everything the agent did in reply.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub key: String,
    /// `None` for agent output before the first prompt (a resumed or compacted session).
    pub prompt: Option<Message>,
    /// What the agent did before its final answer: thinking, tools, interim text.
    pub work: Vec<Step>,
    /// Text and errors after the last piece of work.
    pub answer: Vec<Step>,
    /// The agent is still working on this turn.
    pub live: bool,
    pub stopped: bool,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// What the turn's model calls used, added up.
    pub usage: Option<UsageTotal>,
}

/// How many prompts a conversation holds; each one opens a turn.
pub fn prompt_count(messages: &[Message]) -> usize {
    messages.iter().filter(|m| m.role == Role::User).count()
}

/// The turn opened by the first prompt after the `before` prompts that
/// existed when a message was sent. Agent output never opens a turn, so a
/// reply streaming in is not taken for the prompt just sent, and a queued
/// prompt is found once it reaches the transcript.
pub fn sent_turn(turns: &[Turn], before: usize) -> Option<&Turn> {
    turns.iter().filter(|t| t.prompt.is_some()).nth(before)
}

const QUIET: [ToolKind; 2] = [ToolKind::Read, ToolKind::Search];

/// Groups a conversation's messages into turns.
pub fn build_turns(conversation: &Conversation) -> Vec<Turn> {
    let mut turns = Vec::new();
    let mut current: Option<(Option<&Message>, Vec<&Message>)> = None;
    for message in &conversation.messages {
        if message.role == Role::User {
            if let Some((prompt, replies)) = current.take() {
                turns.push(turn(conversation, prompt, &replies));
            }
            current = Some((Some(message), Vec::new()));
        } else {
            current.get_or_insert((None, Vec::new())).1.push(message);
        }
    }
    if let Some((prompt, replies)) = current {
        turns.push(turn(conversation, prompt, &replies));
    }
    if conversation.busy
        && let Some(last) = turns.last_mut()
    {
        last.live = true;
        last.ended_at = None;
    }
    turns
}

fn turn(conversation: &Conversation, prompt: Option<&Message>, replies: &[&Message]) -> Turn {
    let mut steps = Vec::new();
    for message in replies {
        if message.role == Role::Other {
            steps.push(Step::Note {
                key: message.id.clone(),
                text: text_of(message),
            });
            continue;
        }
        for (index, block) in message.blocks.iter().enumerate() {
            let key = format!("{}/{index}", message.id);
            match block {
                Block::Text { text } => {
                    if !text.trim().is_empty() {
                        steps.push(Step::Text {
                            key,
                            text: text.clone(),
                        });
                    }
                }
                Block::Thinking { text } => {
                    if !text.trim().is_empty() {
                        let live = conversation
                            .thinking
                            .as_ref()
                            .is_some_and(|(id, t)| *id == message.id && t.index == index);
                        steps.push(Step::Thinking {
                            key,
                            text: text.clone(),
                            duration_ms: conversation.thought_for.get(text.trim()).copied(),
                            live,
                        });
                    }
                }
                Block::ToolCall(call) => steps.push(Step::Tool(ToolStep {
                    key,
                    call: (**call).clone(),
                    duration_ms: match (call.started_at, call.ended_at) {
                        (Some(start), Some(end)) => Some(end - start),
                        _ => None,
                    },
                    approval: conversation
                        .approvals
                        .iter()
                        .find(|a| a.tool_call_id == call.id)
                        .cloned(),
                })),
                Block::Image { mime_type, data } => steps.push(Step::Image {
                    key,
                    mime_type: mime_type.clone(),
                    data: data.clone(),
                }),
            }
        }
        if let Some(error) = &message.error {
            steps.push(Step::Error {
                key: format!("{}/error", message.id),
                text: error.clone(),
            });
        }
    }
    let mut grouped = group_quiet(steps);
    let mut split = grouped.len();
    while split > 0 && grouped[split - 1].is_answer() {
        split -= 1;
    }
    let answer = grouped.split_off(split);
    let times: Vec<i64> = prompt
        .into_iter()
        .chain(replies.iter().copied())
        .flat_map(|m| [m.timestamp, m.usage.as_ref().and_then(|u| u.ended_at)])
        .flatten()
        .collect();
    Turn {
        key: prompt.map_or_else(|| replies[0].id.clone(), |p| p.id.clone()),
        prompt: prompt.cloned(),
        work: grouped,
        answer,
        live: false,
        stopped: replies.iter().any(|m| m.stopped),
        started_at: times.iter().min().copied(),
        ended_at: if times.len() > 1 {
            times.iter().max().copied()
        } else {
            None
        },
        usage: sum_usage(replies.iter().copied()),
    }
}

/// Runs of two or more successful read and search calls without images become one group.
fn group_quiet(steps: Vec<Step>) -> Vec<Step> {
    let mut out = Vec::new();
    let mut run: Vec<ToolStep> = Vec::new();
    fn flush(out: &mut Vec<Step>, run: &mut Vec<ToolStep>) {
        if run.len() > 1 {
            out.push(Step::Group {
                key: run[0].key.clone(),
                steps: std::mem::take(run),
            });
        } else {
            out.extend(run.drain(..).map(Step::Tool));
        }
    }
    for step in steps {
        match step {
            Step::Tool(tool)
                if QUIET.contains(&tool.call.kind)
                    && tool.call.status != ToolStatus::Failed
                    && tool.approval.is_none()
                    // A call that returned an image stays on its own row.
                    && tool.call.images.is_empty() =>
            {
                run.push(tool)
            }
            other => {
                flush(&mut out, &mut run);
                out.push(other);
            }
        }
    }
    flush(&mut out, &mut run);
    out
}

fn text_of(message: &Message) -> String {
    message
        .blocks
        .iter()
        .map(|b| match b {
            Block::Text { text } => text.as_str(),
            _ => "",
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn basename(path: &str) -> &str {
    path.split('/').rfind(|s| !s.is_empty()).unwrap_or(path)
}

/// The tool calls among a turn's steps, groups opened up.
pub fn tool_steps(steps: &[Step]) -> Vec<&ToolStep> {
    steps
        .iter()
        .flat_map(|s| match s {
            Step::Tool(t) => vec![t],
            Step::Group { steps, .. } => steps.iter().collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// Every image the steps' tool calls returned, in order.
pub fn tool_images(steps: &[Step]) -> Vec<ToolImage> {
    tool_steps(steps)
        .into_iter()
        .flat_map(|s| s.call.images.clone())
        .collect()
}

/// Approvals whose tool call no message holds, to show on their own.
pub fn loose_approvals(conversation: &Conversation) -> Vec<Approval> {
    let ids: Vec<&str> = conversation
        .messages
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            Block::ToolCall(call) => Some(call.id.as_str()),
            _ => None,
        })
        .collect();
    conversation
        .approvals
        .iter()
        .filter(|a| !ids.contains(&a.tool_call_id.as_str()))
        .cloned()
        .collect()
}

/// The text a tool call produced, whatever its kind.
pub fn output_text(call: &ToolCall) -> &str {
    match &call.output {
        Some(ToolOutput::Text { text }) | Some(ToolOutput::Shell { text, .. }) => text,
        Some(ToolOutput::Edits { text, .. }) => text.as_deref().unwrap_or(""),
        None => "",
    }
}

/// The file changes a tool call makes.
pub fn edits_of(call: &ToolCall) -> &[FileEdit] {
    match &call.output {
        Some(ToolOutput::Edits { edits, .. }) => edits,
        _ => &[],
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "Read 3 files" or "Read 2 files, searched 1 pattern", for a group row.
pub fn group_label(steps: &[ToolStep]) -> String {
    let reads = steps
        .iter()
        .filter(|s| s.call.kind == ToolKind::Read)
        .count();
    let searches = steps.len() - reads;
    let parts: Vec<String> = [
        (reads > 0).then(|| format!("read {}", plural(reads, "file", "files"))),
        (searches > 0).then(|| format!("searched {}", plural(searches, "pattern", "patterns"))),
    ]
    .into_iter()
    .flatten()
    .collect();
    capitalize(&parts.join(", "))
}

/// What a finished turn's folded work row says, when it did enough to fold.
pub fn summarize_work(steps: &[Step]) -> Option<(String, String)> {
    let calls = tool_steps(steps);
    // A single call reads fine as its own row.
    if calls.len() < 2 {
        return None;
    }
    let of = |kind: ToolKind| calls.iter().filter(move |s| s.call.kind == kind);
    let reads = of(ToolKind::Read).count();
    let searches = of(ToolKind::Search).count();
    let mut edited: Vec<&str> = Vec::new();
    for step in of(ToolKind::Edit) {
        let name = basename(step.call.subject.as_deref().unwrap_or(&step.call.name));
        if !edited.contains(&name) {
            edited.push(name);
        }
    }
    let ran: Vec<&str> = of(ToolKind::Execute)
        .map(|s| s.call.subject.as_deref().unwrap_or(&s.call.name))
        .collect();
    let other = of(ToolKind::Other).count();
    let more = |n: usize| {
        if n > 0 {
            format!(" and {n} more")
        } else {
            String::new()
        }
    };
    let detail: Vec<String> = [
        (reads > 0).then(|| format!("read {}", plural(reads, "file", "files"))),
        (searches > 0).then(|| format!("searched {}", plural(searches, "pattern", "patterns"))),
        (!edited.is_empty()).then(|| {
            format!(
                "edited {}{}",
                edited[..edited.len().min(3)].join(", "),
                more(edited.len().saturating_sub(3))
            )
        }),
        (!ran.is_empty()).then(|| format!("ran {}{}", ran[0], more(ran.len() - 1))),
        (other > 0).then(|| format!("used {}", plural(other, "other tool", "other tools"))),
    ]
    .into_iter()
    .flatten()
    .collect();
    Some((
        format!("Ran {}", plural(calls.len(), "tool", "tools")),
        detail.join(" · "),
    ))
}

/// A turn's answer as markdown, for copying.
pub fn answer_text(turn: &Turn) -> String {
    turn.answer
        .iter()
        .filter_map(|s| match s {
            Step::Text { text, .. } => Some(text.trim()),
            _ => None,
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// "8s", "1m 12s", for durations shown in rows.
pub fn format_duration(ms: i64) -> String {
    if ms < 1000 {
        return format!("{:.1}s", ms as f64 / 1000.0);
    }
    let seconds = (ms as f64 / 1000.0).round() as i64;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m {}s", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::{StopReason, Usage};

    fn message(id: &str, role: Role, blocks: Vec<Block>) -> Message {
        Message {
            id: id.into(),
            role,
            blocks,
            timestamp: None,
            error: None,
            stopped: false,
            usage: None,
        }
    }

    fn text(t: &str) -> Block {
        Block::Text { text: t.into() }
    }

    fn call(id: &str, kind: ToolKind, subject: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: format!("{kind:?}").to_lowercase(),
            kind,
            subject: Some(subject.into()),
            input: serde_json::json!({}),
            status: ToolStatus::Done,
            output: None,
            images: Vec::new(),
            started_at: None,
            ended_at: None,
        }
    }

    fn block(call: ToolCall) -> Block {
        Block::ToolCall(Box::new(call))
    }

    fn turns_of(messages: Vec<Message>, busy: bool) -> Vec<Turn> {
        build_turns(&Conversation {
            busy,
            ..Conversation::new(messages)
        })
    }

    #[test]
    fn the_sent_turn_is_the_next_prompt_not_streamed_output() {
        let mut messages = vec![
            message("a1", Role::Assistant, vec![text("resumed")]),
            message("u1", Role::User, vec![text("first")]),
            message("a2", Role::Assistant, vec![text("working")]),
        ];
        let before = prompt_count(&messages);
        assert_eq!(before, 1);
        // A reply streams in while the sent prompt waits in the queue.
        messages.push(message("a3", Role::Assistant, vec![text("more")]));
        assert!(sent_turn(&turns_of(messages.clone(), true), before).is_none());
        messages.push(message("u2", Role::User, vec![text("second")]));
        messages.push(message("a4", Role::Assistant, vec![text("ok")]));
        let turns = turns_of(messages, true);
        assert_eq!(
            sent_turn(&turns, before)
                .and_then(|t| t.prompt.as_ref())
                .map(|m| m.id.as_str()),
            Some("u2")
        );
    }

    #[test]
    fn starts_a_turn_at_each_prompt_and_times_tool_calls() {
        let output = ToolOutput::Shell {
            text: "a\nb".into(),
            exit_code: Some(0),
        };
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("list files")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![block(ToolCall {
                        output: Some(output),
                        started_at: Some(1000),
                        ended_at: Some(3500),
                        ..call("c1", ToolKind::Execute, "ls")
                    })],
                ),
                message("a2", Role::Assistant, vec![text("Two files.")]),
                message("u2", Role::User, vec![text("thanks")]),
            ],
            false,
        );
        assert_eq!(
            turns.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
            ["u1", "u2"]
        );
        let Step::Tool(step) = &turns[0].work[0] else {
            panic!("expected a tool step")
        };
        assert_eq!(step.duration_ms, Some(2500));
        assert!(matches!(&turns[0].answer[..], [Step::Text { text, .. }] if text == "Two files."));
    }

    #[test]
    fn keeps_output_before_the_first_prompt_in_a_turn_of_its_own() {
        let turns = turns_of(
            vec![message("a0", Role::Assistant, vec![text("resumed")])],
            false,
        );
        assert_eq!(turns[0].key, "a0");
        assert!(turns[0].prompt.is_none());
    }

    #[test]
    fn groups_runs_of_successful_reads_and_searches() {
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("look")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![
                        block(call("c1", ToolKind::Read, "a.ts")),
                        block(call("c2", ToolKind::Search, "TODO")),
                        block(ToolCall {
                            status: ToolStatus::Failed,
                            ..call("c3", ToolKind::Read, "b.ts")
                        }),
                        block(call("c4", ToolKind::Execute, "pnpm test")),
                        block(call("c5", ToolKind::Read, "c.ts")),
                    ],
                ),
            ],
            false,
        );
        let work = &turns[0].work;
        assert!(matches!(work[0], Step::Group { .. }));
        assert_eq!(work.len(), 4);
        assert_eq!(tool_steps(work).len(), 5);
        let Step::Group { steps, .. } = &work[0] else {
            unreachable!()
        };
        assert_eq!(group_label(steps), "Read 1 file, searched 1 pattern");
    }

    #[test]
    fn keeps_a_read_that_returned_an_image_out_of_a_group() {
        let shot = ToolImage {
            mime_type: "image/png".into(),
            data: "aGVsbG8=".into(),
        };
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("look")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![
                        block(call("c1", ToolKind::Read, "a.ts")),
                        block(ToolCall {
                            images: vec![shot.clone()],
                            ..call("c2", ToolKind::Read, "shot.png")
                        }),
                        block(call("c3", ToolKind::Read, "b.ts")),
                    ],
                ),
            ],
            false,
        );
        let work = &turns[0].work;
        assert!(work.iter().all(|s| matches!(s, Step::Tool(_))));
        assert!(summarize_work(work).is_some());
        assert_eq!(tool_images(work), vec![shot]);
    }

    #[test]
    fn puts_an_approval_on_the_call_it_is_about_outside_any_group() {
        let approval = Approval {
            id: "a1".into(),
            tool_call_id: "c2".into(),
            title: "Allow?".into(),
            message: None,
            expires_at: None,
        };
        let loose = Approval {
            id: "a2".into(),
            tool_call_id: "gone".into(),
            ..approval.clone()
        };
        let conversation = Conversation {
            approvals: vec![approval.clone(), loose.clone()],
            ..Conversation::new(vec![
                message("u1", Role::User, vec![text("look")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![
                        block(call("c1", ToolKind::Read, "a.ts")),
                        block(ToolCall {
                            status: ToolStatus::Running,
                            ..call("c2", ToolKind::Read, "b.ts")
                        }),
                    ],
                ),
            ])
        };
        let turns = build_turns(&conversation);
        let approvals: Vec<_> = tool_steps(&turns[0].work)
            .iter()
            .map(|s| s.approval.clone())
            .collect();
        assert_eq!(approvals, vec![None, Some(approval)]);
        assert_eq!(loose_approvals(&conversation), vec![loose]);
    }

    #[test]
    fn marks_only_the_last_turn_live_while_the_agent_works() {
        let at = |m: Message, t: i64| Message {
            timestamp: Some(t),
            ..m
        };
        let turns = turns_of(
            vec![
                at(message("u1", Role::User, vec![text("a")]), 1),
                at(message("a1", Role::Assistant, vec![text("b")]), 5),
                at(message("u2", Role::User, vec![text("c")]), 9),
            ],
            true,
        );
        assert_eq!(
            turns.iter().map(|t| t.live).collect::<Vec<_>>(),
            [false, true]
        );
        assert_eq!((turns[0].started_at, turns[0].ended_at), (Some(1), Some(5)));
        assert_eq!(turns[1].ended_at, None);
    }

    #[test]
    fn adds_up_a_turns_usage_and_ends_it_when_its_last_call_ended() {
        let usage = |ended_at: i64, cost: f64| {
            Some(Box::new(Usage {
                model: Some("m".into()),
                provider: Some("p".into()),
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost: Some(cost),
                stop_reason: Some(StopReason::Finished),
                ended_at: Some(ended_at),
            }))
        };
        let with = |m: Message, t: i64, u| Message {
            timestamp: Some(t),
            usage: u,
            ..m
        };
        let turns = turns_of(
            vec![
                with(message("u1", Role::User, vec![text("a")]), 1_000, None),
                with(
                    message("a1", Role::Assistant, vec![text("b")]),
                    1_200,
                    usage(3_000, 0.1),
                ),
                with(
                    message("a2", Role::Assistant, vec![text("c")]),
                    3_100,
                    usage(8_000, 0.2),
                ),
                with(message("u2", Role::User, vec![text("d")]), 9_000, None),
            ],
            false,
        );
        assert_eq!(
            (turns[0].started_at, turns[0].ended_at),
            (Some(1_000), Some(8_000))
        );
        let total = turns[0].usage.as_ref().unwrap();
        assert_eq!(
            (total.calls, total.input_tokens, total.output_tokens),
            (2, 20, 10)
        );
        assert!((total.cost.unwrap() - 0.3).abs() < 1e-9);
        assert!(turns[1].usage.is_none());
    }

    #[test]
    fn keeps_errors_in_the_answer_and_carries_the_stopped_flag() {
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("go")]),
                Message {
                    error: Some("Rate limited".into()),
                    stopped: true,
                    ..message(
                        "a1",
                        Role::Assistant,
                        vec![block(call("c1", ToolKind::Execute, "make"))],
                    )
                },
            ],
            false,
        );
        assert!(
            matches!(&turns[0].answer[..], [Step::Error { text, .. }] if text == "Rate limited")
        );
        assert!(turns[0].stopped);
    }

    #[test]
    fn says_what_the_folded_work_did() {
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("fix")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![
                        block(call("c1", ToolKind::Read, "src/a.ts")),
                        block(call("c2", ToolKind::Read, "src/b.ts")),
                        block(call("c3", ToolKind::Edit, "src/a.ts")),
                        block(call("c4", ToolKind::Edit, "src/a.ts")),
                        block(call("c5", ToolKind::Execute, "pnpm test")),
                        block(call("c6", ToolKind::Execute, "pnpm lint")),
                    ],
                ),
                message("a2", Role::Assistant, vec![text("Done.")]),
            ],
            false,
        );
        assert_eq!(
            summarize_work(&turns[0].work),
            Some((
                "Ran 6 tools".to_string(),
                "read 2 files · edited a.ts · ran pnpm test and 1 more".to_string()
            ))
        );
    }

    #[test]
    fn has_nothing_to_fold_for_fewer_than_two_calls() {
        let turns = turns_of(
            vec![
                message("u1", Role::User, vec![text("ls")]),
                message(
                    "a1",
                    Role::Assistant,
                    vec![block(call("c1", ToolKind::Execute, "ls"))],
                ),
            ],
            false,
        );
        assert_eq!(summarize_work(&turns[0].work), None);
    }

    #[test]
    fn durations_read_like_a_clock_for_long_runs() {
        let formatted: Vec<_> = [400, 8_200, 72_000]
            .into_iter()
            .map(format_duration)
            .collect();
        assert_eq!(formatted, ["0.4s", "8s", "1m 12s"]);
    }
}
