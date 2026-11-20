//! What a run of model calls used, added up.

use panoptes_harness::{Message, StopReason};

use super::format::format_tokens;

#[derive(Debug, Clone, PartialEq)]
pub struct UsageTotal {
    /// Model calls that reported usage.
    pub calls: u32,
    /// Distinct models, in the order they first answered.
    pub models: Vec<String>,
    /// Prompt tokens, cached or not.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// `None` when no call was priced.
    pub cost: Option<f64>,
    /// Why the last call ended.
    pub stop_reason: Option<StopReason>,
    /// When the last call ended, Unix milliseconds.
    pub ended_at: Option<i64>,
}

/// Adds up the usage the messages report; `None` when none does.
pub fn sum_usage<'a>(messages: impl IntoIterator<Item = &'a Message>) -> Option<UsageTotal> {
    let mut total: Option<UsageTotal> = None;
    for message in messages {
        let Some(usage) = &message.usage else {
            continue;
        };
        let t = total.get_or_insert(UsageTotal {
            calls: 0,
            models: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cost: None,
            stop_reason: None,
            ended_at: None,
        });
        t.calls += 1;
        if let Some(model) = &usage.model
            && !t.models.contains(model)
        {
            t.models.push(model.clone());
        }
        t.input_tokens += usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
        t.output_tokens += usage.output_tokens;
        if let Some(cost) = usage.cost {
            t.cost = Some(t.cost.unwrap_or(0.0) + cost);
        }
        t.stop_reason = usage.stop_reason;
        if let Some(ended) = usage.ended_at {
            t.ended_at = Some(t.ended_at.unwrap_or(0).max(ended));
        }
    }
    total
}

/// "$0.42", "<$0.01"; `None` for an unpriced or free run.
pub fn format_cost(cost: Option<f64>) -> Option<String> {
    let cost = cost?;
    if cost <= 0.0 {
        return None;
    }
    if cost < 0.01 {
        return Some("<$0.01".into());
    }
    Some(format!("${cost:.2}"))
}

/// "12k in · 1.3k out".
pub fn tokens_label(total: &UsageTotal) -> String {
    format!(
        "{} in · {} out",
        format_tokens(total.input_tokens),
        format_tokens(total.output_tokens)
    )
}

/// What a turn's footer says it used: model, tokens, cost, an unusual stop.
pub fn turn_usage_parts(total: &UsageTotal) -> Vec<String> {
    [
        Some(total.models.join(", ")),
        Some(tokens_label(total)),
        format_cost(total.cost),
        (total.stop_reason == Some(StopReason::MaxTokens)).then(|| "Hit the output limit".into()),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect()
}

/// The session total beside the composer: its cost, or its tokens when it has no price.
pub fn session_usage_label(total: &UsageTotal) -> String {
    format_cost(total.cost).unwrap_or_else(|| {
        format!(
            "{} tokens",
            format_tokens(total.input_tokens + total.output_tokens)
        )
    })
}

/// The session total in full, for its tooltip.
pub fn session_usage_detail(total: &UsageTotal) -> String {
    let calls = format!(
        "{} model {}",
        total.calls,
        if total.calls == 1 { "call" } else { "calls" }
    );
    [
        Some("This session".to_string()),
        format_cost(total.cost),
        Some(tokens_label(total)),
        Some(calls),
        (!total.models.is_empty()).then(|| total.models.join(", ")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::{Role, Usage};

    fn usage() -> Usage {
        Usage {
            model: Some("claude-sonnet-4-5".into()),
            provider: Some("anthropic".into()),
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 9_000,
            cache_write_tokens: 0,
            cost: Some(0.01),
            stop_reason: Some(StopReason::ToolUse),
            ended_at: Some(5_000),
        }
    }

    fn message(id: &str, usage: Option<Usage>) -> Message {
        Message {
            id: id.into(),
            role: if usage.is_some() {
                Role::Assistant
            } else {
                Role::User
            },
            blocks: Vec::new(),
            timestamp: None,
            error: None,
            stopped: false,
            usage: usage.map(Box::new),
        }
    }

    #[test]
    fn adds_up_every_call_and_keeps_the_last_stop_and_end() {
        let total = sum_usage(&[
            message("u", None),
            message("a1", Some(usage())),
            message(
                "a2",
                Some(Usage {
                    model: Some("gpt-5".into()),
                    cost: Some(0.02),
                    stop_reason: Some(StopReason::Finished),
                    ended_at: Some(9_000),
                    ..usage()
                }),
            ),
        ])
        .unwrap();
        assert_eq!(total.calls, 2);
        assert_eq!(total.models, ["claude-sonnet-4-5", "gpt-5"]);
        assert_eq!(total.input_tokens, 20_000);
        assert_eq!(total.output_tokens, 400);
        assert!((total.cost.unwrap() - 0.03).abs() < 1e-9);
        assert_eq!(total.stop_reason, Some(StopReason::Finished));
        assert_eq!(total.ended_at, Some(9_000));
    }

    #[test]
    fn is_none_without_usage_and_unpriced_calls_leave_the_cost_unknown() {
        assert_eq!(sum_usage(&[message("u", None)]), None);
        let unpriced = sum_usage(&[message(
            "a",
            Some(Usage {
                cost: None,
                ..usage()
            }),
        )]);
        assert_eq!(unpriced.unwrap().cost, None);
    }

    #[test]
    fn formats_cost_hiding_free_and_unpriced_runs() {
        assert_eq!(format_cost(Some(1.234)).as_deref(), Some("$1.23"));
        assert_eq!(format_cost(Some(0.004)).as_deref(), Some("<$0.01"));
        assert_eq!(format_cost(Some(0.0)), None);
        assert_eq!(format_cost(None), None);
    }

    #[test]
    fn says_what_a_turn_used_and_flags_a_cut_off_answer() {
        let total = sum_usage(&[message(
            "a",
            Some(Usage {
                stop_reason: Some(StopReason::MaxTokens),
                cost: Some(0.42),
                ..usage()
            }),
        )])
        .unwrap();
        assert_eq!(
            turn_usage_parts(&total),
            [
                "claude-sonnet-4-5",
                "10k in · 200 out",
                "$0.42",
                "Hit the output limit"
            ]
        );
        let free = sum_usage(&[message(
            "a",
            Some(Usage {
                model: None,
                cost: Some(0.0),
                ..usage()
            }),
        )])
        .unwrap();
        assert_eq!(turn_usage_parts(&free), ["10k in · 200 out"]);
    }

    #[test]
    fn shows_a_sessions_cost_or_its_tokens_when_it_has_no_price() {
        let priced =
            sum_usage(&[message("a", Some(usage())), message("b", Some(usage()))]).unwrap();
        assert_eq!(session_usage_label(&priced), "$0.02");
        assert_eq!(
            session_usage_detail(&priced),
            "This session · $0.02 · 20k in · 400 out · 2 model calls · claude-sonnet-4-5"
        );
        let free = sum_usage(&[message(
            "a",
            Some(Usage {
                cost: Some(0.0),
                ..usage()
            }),
        )])
        .unwrap();
        assert_eq!(session_usage_label(&free), "10k tokens");
    }
}
