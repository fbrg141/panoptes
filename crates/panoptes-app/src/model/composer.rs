//! The composer's text logic: @-mentions, file matching and labels. Pure, so it
//! is tested without a window. Offsets are byte offsets into the text, as the
//! GPUI text field uses.

use std::collections::{BTreeSet, HashSet};

use panoptes_core::files::Attachment;
use panoptes_harness::{CommandInfo, CommandKind, ContextUsage, ImageAttachment};

use crate::model::format::format_tokens;
use crate::store::Draft;

/// An `@query` or `/query` being typed: where it starts and what follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub start: usize,
    pub query: String,
}

/// Text after an edit, and where the caret goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub caret: usize,
}

fn edited(text: impl Into<String>, caret: usize) -> Edited {
    Edited {
        text: text.into(),
        caret,
    }
}

/// The mention the caret is in, if any. An `@` starts one only at the start
/// of the text or after whitespace, so e-mail addresses don't.
pub fn mention_at(text: &str, caret: usize) -> Option<Query> {
    let before = text.get(..caret)?;
    let at = before.rfind(|c: char| c == '@' || c.is_whitespace())?;
    if !before[at..].starts_with('@') {
        return None;
    }
    let opens = before[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    opens.then(|| Query {
        start: at,
        query: before[at + 1..].to_string(),
    })
}

/// A `/command` being typed as the message's first token.
pub fn command_at(text: &str, caret: usize) -> Option<Query> {
    let before = text.get(..caret)?;
    let query = before.strip_prefix('/')?;
    (!query.contains(char::is_whitespace)).then(|| Query {
        start: 0,
        query: query.to_string(),
    })
}

/// The commands offered: the harness's own, plus lane commands in a lane.
/// Lane commands are handled by the composer itself, not sent to the agent.
pub fn available_commands(harness: &[CommandInfo], lane: bool) -> Vec<CommandInfo> {
    if !lane {
        return harness.to_vec();
    }
    let lane = |name: &str, description: &str| CommandInfo {
        name: name.into(),
        description: Some(description.into()),
        kind: CommandKind::Other,
    };
    [
        lane("new", "Start a new session in this lane"),
        lane("resume", "Resume a previous session in this lane"),
    ]
    .into_iter()
    .chain(harness.iter().cloned())
    .collect()
}

/// Narrows discoverable commands by every word typed, prefix matches first.
pub fn match_commands(query: &str, commands: &[CommandInfo], limit: usize) -> Vec<CommandInfo> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut found: Vec<CommandInfo> = commands
        .iter()
        .filter(|command| {
            let text = format!(
                "{} {}",
                command.name,
                command.description.as_deref().unwrap_or("")
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .cloned()
        .collect();
    found.sort_by(|a, b| {
        let prefix = |c: &CommandInfo| c.name.to_lowercase().starts_with(&q);
        prefix(b).cmp(&prefix(a)).then_with(|| a.name.cmp(&b.name))
    });
    found.truncate(limit);
    found
}

fn starts_with_space(text: &str) -> bool {
    text.chars().next().is_some_and(char::is_whitespace)
}

/// Completes a slash command, leaving a space for its arguments.
pub fn insert_command(text: &str, caret: usize, command: &CommandInfo) -> Edited {
    let rest = &text[caret..];
    let name = format!("/{}", command.name);
    let caret = name.len() + 1;
    let inserted = if starts_with_space(rest) {
        name
    } else {
        format!("{name} ")
    };
    edited(inserted + rest, caret)
}

/// Replaces the mention being typed with `@path`; the caret goes after the
/// space that follows it.
pub fn insert_mention(text: &str, caret: usize, mention: &Query, path: &str) -> Edited {
    let rest = &text[caret..];
    let inserted = if starts_with_space(rest) {
        format!("@{path}")
    } else {
        format!("@{path} ")
    };
    edited(
        format!("{}{inserted}{rest}", &text[..mention.start]),
        mention.start + path.len() + 2,
    )
}

/// Greedy subsequence score of `query` in `target`, or `None` if it does not
/// match. Consecutive letters and letters that start a word count most.
fn subsequence(query: &[char], target: &[char]) -> Option<f64> {
    let mut score = 0.;
    let mut from = 0;
    let mut previous: isize = -2;
    for ch in query {
        let found = from + target[from..].iter().position(|c| c == ch)?;
        score += 1.;
        if found as isize == previous + 1 {
            score += 4.;
        }
        if found == 0 || "/._- ".contains(target[found - 1]) {
            score += 3.;
        }
        previous = found as isize;
        from = found + 1;
    }
    Some(score)
}

/// How well `path` matches a fuzzy `query`. Matches inside the file name beat
/// matches spread over folders, and shorter paths win ties.
pub fn fuzzy_score(query: &str, path: &str) -> Option<f64> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let p: Vec<char> = path.to_lowercase().chars().collect();
    let whole = subsequence(&q, &p)?;
    let name_start = p.iter().rposition(|c| *c == '/').map_or(0, |i| i + 1);
    let best = match subsequence(&q, &p[name_start..]) {
        Some(name) => whole.max(name + 10.),
        None => whole,
    };
    Some(best - p.len() as f64 / 100.)
}

/// The best `limit` files for a query, best first.
pub fn match_files(query: &str, files: &[String], limit: usize) -> Vec<String> {
    if query.is_empty() {
        let mut all = files.to_vec();
        all.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        all.truncate(limit);
        return all;
    }
    let mut scored: Vec<(&String, f64)> = files
        .iter()
        .filter_map(|file| fuzzy_score(query, file).map(|score| (file, score)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    scored
        .into_iter()
        .take(limit)
        .map(|(file, _)| file.clone())
        .collect()
}

/// A line that opens a list item: its indentation, a marker and a space.
/// Returns the opening's length, the indentation and the marker.
fn list_item(line: &str) -> Option<(usize, &str, &str)> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let marker_len = match rest.chars().next()? {
        '-' | '*' | '+' => 1,
        c if c.is_ascii_digit() => {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 9 || !matches!(rest.as_bytes().get(digits), Some(b'.' | b')')) {
                return None;
            }
            digits + 1
        }
        _ => return None,
    };
    if rest.as_bytes().get(marker_len) != Some(&b' ') {
        return None;
    }
    Some((indent_len + marker_len + 1, indent, &rest[..marker_len]))
}

/// Enter inside a list: the item's marker continues on the next line, ordered
/// numbers counting on, an open task item continuing unchecked. An item with
/// nothing after its marker ends the list instead. `None` when the line is
/// not a list item, so the newline is left to the field.
pub fn continue_list(original: &str, start: usize, end: usize) -> Option<Edited> {
    // A selection is replaced by the newline, as the field itself would do.
    let text = format!("{}{}", &original[..start], &original[end..]);
    let caret = start;
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[caret..].find('\n').map(|i| caret + i);
    let line = &text[line_start..line_end.unwrap_or(text.len())];
    let (opened, indent, marker) = list_item(line)?;
    // Only once the caret is past the marker does Enter continue the list.
    if caret < line_start + opened {
        return None;
    }
    let item = &line[opened..];
    // An item with nothing after its marker ends the list: the marker goes
    // and the caret stays on the now empty line.
    if item.trim().is_empty() {
        let tail = line_end.map_or("", |end| &text[end..]);
        return Some(edited(format!("{}{tail}", &text[..line_start]), line_start));
    }
    let next = if marker.starts_with(|c: char| c.is_ascii_digit()) {
        let number: u64 = marker[..marker.len() - 1].parse().ok()?;
        format!("{}{}", number + 1, &marker[marker.len() - 1..])
    } else {
        marker.to_string()
    };
    // An open task item continues unchecked.
    let task = if ["[ ] ", "[x] ", "[X] "].iter().any(|t| item.starts_with(t)) {
        "[ ] "
    } else {
        ""
    };
    let inserted = format!("\n{indent}{next} {task}");
    // Splitting an item mid-line moves the rest after the new marker, without
    // the space that separated it from the text before the caret.
    let rest = text[caret..].trim_start_matches([' ', '\t']);
    Some(edited(
        format!("{}{inserted}{rest}", &text[..caret]),
        caret + inserted.len(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub mention: bool,
}

/// The file name shown for a mention, while its full path stays in the text.
pub fn mention_label(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Splits text into plain runs and the `@path` mentions in `known`, for
/// drawing mentions as chips.
pub fn segments(text: &str, known: &HashSet<String>) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut last = 0;
    let mut i = 0;
    while let Some(found) = text[i..].find('@') {
        let at = i + found;
        i = at + 1;
        let opens = text[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if !opens {
            continue;
        }
        let end = text[at + 1..]
            .find(char::is_whitespace)
            .map_or(text.len(), |e| at + 1 + e);
        let path = &text[at + 1..end];
        if path.is_empty() || !known.contains(path) {
            continue;
        }
        if at > last {
            out.push(Segment {
                text: text[last..at].to_string(),
                mention: false,
            });
        }
        out.push(Segment {
            text: format!("@{path}"),
            mention: true,
        });
        last = end;
        i = end;
    }
    if last < text.len() {
        out.push(Segment {
            text: text[last..].to_string(),
            mention: false,
        });
    }
    out
}

/// "42k / 200k", or just the window while usage is unknown.
pub fn context_label(usage: &ContextUsage) -> String {
    let window = format_tokens(usage.window);
    match usage.tokens {
        None => format!("{window} context"),
        Some(tokens) => format!("{} / {window}", format_tokens(tokens)),
    }
}

/// The level after `current`, wrapping round.
pub fn next_level(levels: &[String], current: Option<&str>) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    let next = current
        .and_then(|c| levels.iter().position(|l| l == c))
        .map_or(0, |i| (i + 1) % levels.len());
    Some(levels[next].clone())
}

/// The prompt to send: images inline, other files appended as `@path`
/// mentions for the agent to read with its own tools.
pub fn compose_prompt(text: &str, attachments: &[Attachment]) -> (String, Vec<ImageAttachment>) {
    let mut images = Vec::new();
    let mut paths = Vec::new();
    for file in attachments {
        match (&file.mime_type, &file.data) {
            (Some(mime_type), Some(data)) => images.push(ImageAttachment {
                mime_type: mime_type.clone(),
                data: data.clone(),
            }),
            _ => paths.push(format!("@{}", file.path)),
        }
    }
    let parts: Vec<String> = [text.trim().to_string(), paths.join("\n")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    (parts.join("\n\n"), images)
}

/// The draft to show after a prompt failed to reach the agent: what was sent,
/// followed by anything typed since, so neither is lost.
pub fn restore_unsent(unsent: Draft, current: Option<&Draft>) -> Draft {
    let Some(current) = current.filter(|c| !c.text.trim().is_empty() || !c.attachments.is_empty())
    else {
        return unsent;
    };
    let text = if unsent.text.trim().is_empty() {
        current.text.clone()
    } else {
        format!("{}\n{}", unsent.text, current.text)
    };
    let mut seen = BTreeSet::new();
    let mentioned = unsent
        .mentioned
        .iter()
        .chain(&current.mentioned)
        .filter(|m| seen.insert((*m).clone()))
        .cloned()
        .collect();
    let mut attachments = unsent.attachments;
    attachments.extend(current.attachments.iter().cloned());
    Draft {
        text,
        attachments,
        mentioned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_harness::CommandKind;

    fn q(start: usize, query: &str) -> Option<Query> {
        Some(Query {
            start,
            query: query.into(),
        })
    }

    #[test]
    fn mentions() {
        assert_eq!(mention_at("look at @src/ap", 15), q(8, "src/ap"));
        assert_eq!(mention_at("@", 1), q(0, ""));
        assert_eq!(mention_at("mail me@example.com", 19), None);
        assert_eq!(mention_at("@src/a.ts and", 13), None);
        let mention = mention_at("see @ap please", 7).unwrap();
        assert_eq!(
            insert_mention("see @ap please", 7, &mention, "src/api.ts"),
            edited("see @src/api.ts please", 16)
        );
        assert_eq!(
            insert_mention("@ap", 3, &q(0, "ap").unwrap(), "src/api.ts"),
            edited("@src/api.ts ", 12)
        );
    }

    #[test]
    fn mention_labels_show_only_the_file_name() {
        assert_eq!(mention_label("src/api.ts"), "api.ts");
        assert_eq!(mention_label("api.ts"), "api.ts");
    }

    #[test]
    fn splits_out_only_known_mentions() {
        let known: HashSet<String> = ["a.ts".to_string()].into();
        let seg = |t: &str, m| Segment {
            text: t.into(),
            mention: m,
        };
        assert_eq!(
            segments("fix @a.ts not @b.ts", &known),
            [
                seg("fix ", false),
                seg("@a.ts", true),
                seg(" not @b.ts", false)
            ]
        );
        assert_eq!(segments("@a.ts", &known), [seg("@a.ts", true)]);
    }

    fn commands() -> Vec<CommandInfo> {
        let c = |name: &str, description: Option<&str>, kind| CommandInfo {
            name: name.into(),
            description: description.map(Into::into),
            kind,
        };
        vec![
            c("review", Some("Review changes"), CommandKind::Prompt),
            c(
                "fix-tests",
                Some("Fix failing tests"),
                CommandKind::Extension,
            ),
            c("skill:review", None, CommandKind::Skill),
        ]
    }

    #[test]
    fn slash_commands() {
        let commands = commands();
        assert_eq!(command_at("/rev", 4), q(0, "rev"));
        assert_eq!(command_at("please /rev", 10), None);
        assert_eq!(command_at("/review changes", 8), None);
        assert_eq!(command_at("/x", 0), None);
        let names = |q: &str| -> Vec<String> {
            match_commands(q, &commands, 8)
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names("review"), ["review", "skill:review"]);
        assert_eq!(names("failing"), ["fix-tests"]);
        assert_eq!(names("").len(), 3);
        assert_eq!(
            insert_command("/rev", 4, &commands[0]),
            edited("/review ", 8)
        );
        assert_eq!(
            insert_command("/rev now", 4, &commands[0]),
            edited("/review now", 8)
        );
    }

    #[test]
    fn lane_commands_are_offered_only_in_a_lane() {
        let commands = commands();
        let names = |commands: Vec<CommandInfo>| -> Vec<String> {
            commands.into_iter().map(|c| c.name).collect()
        };
        assert_eq!(
            names(available_commands(&commands, true)),
            ["new", "resume", "review", "fix-tests", "skill:review"]
        );
        assert_eq!(available_commands(&commands, false), commands);
        let lane = available_commands(&commands, true);
        assert_eq!(
            match_commands("re", &lane, 8)
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>(),
            ["resume", "review", "skill:review"]
        );
    }

    #[test]
    fn matches_files() {
        let files: Vec<String> = [
            "src/components/ChatPane.tsx",
            "src/store.ts",
            "src/store.test.ts",
            "crates/panoptes-pi/src/map.rs",
            "docs/decisions.md",
        ]
        .map(String::from)
        .into();
        assert_eq!(
            match_files("store", &files, 50),
            ["src/store.ts", "src/store.test.ts"]
        );
        assert_eq!(match_files("chat", &files, 50)[0], files[0]);
        assert_eq!(match_files("cp", &files, 50)[0], files[0]);
        assert_eq!(
            match_files("pimap", &files, 50),
            ["crates/panoptes-pi/src/map.rs"]
        );
        assert!(match_files("zzz", &files, 50).is_empty());
        assert_eq!(
            match_files("", &files, 2),
            ["src/store.ts", "docs/decisions.md"]
        );
    }

    fn cl(text: &str, start: usize) -> Option<Edited> {
        continue_list(text, start, start)
    }

    #[test]
    fn continues_lists() {
        assert_eq!(cl("- first", 7), Some(edited("- first\n- ", 10)));
        assert_eq!(cl("* a\nb", 3), Some(edited("* a\n* \nb", 6)));
        assert_eq!(cl("1. one", 6), Some(edited("1. one\n2. ", 10)));
        assert_eq!(cl("10) ten", 7), Some(edited("10) ten\n11) ", 12)));
        assert_eq!(cl("  - [ ] a", 9), Some(edited("  - [ ] a\n  - [ ] ", 18)));
        assert_eq!(cl("- one\n- ", 8), Some(edited("- one\n", 6)));
        assert_eq!(cl("- one\n- \n- two", 8), Some(edited("- one\n\n- two", 6)));
        assert_eq!(cl("1. ", 3), Some(edited("", 0)));
        assert_eq!(cl("- first item", 7), Some(edited("- first\n- item", 10)));
        assert_eq!(
            continue_list("- keep this drop", 6, 11),
            Some(edited("- keep\n- drop", 9))
        );
        assert_eq!(cl("just text", 9), None);
        assert_eq!(cl("-5 stars", 8), None);
        assert_eq!(cl("-", 1), None);
        assert_eq!(cl("- item", 0), None);
    }

    #[test]
    fn labels() {
        let usage = |tokens| ContextUsage {
            tokens,
            window: 200_000,
        };
        assert_eq!(context_label(&usage(Some(42_000))), "42k / 200k");
        assert_eq!(context_label(&usage(None)), "200k context");
        let levels: Vec<String> = ["off", "low", "high"].map(String::from).into();
        assert_eq!(next_level(&levels, Some("low")).as_deref(), Some("high"));
        assert_eq!(next_level(&levels, Some("high")).as_deref(), Some("off"));
        assert_eq!(next_level(&levels, None).as_deref(), Some("off"));
        assert_eq!(next_level(&[], Some("off")), None);
    }

    fn file(path: &str, image: bool) -> Attachment {
        Attachment {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            mime_type: image.then(|| "image/png".into()),
            data: image.then(|| "AQID".into()),
        }
    }

    #[test]
    fn composes_prompts() {
        let (text, images) = compose_prompt(
            " fix this ",
            &[file("/a.png", true), file("/tmp/log.txt", false)],
        );
        assert_eq!(text, "fix this\n\n@/tmp/log.txt");
        assert_eq!(images.len(), 1);
        assert_eq!(compose_prompt("", &[file("/x", false)]).0, "@/x");
    }

    #[test]
    fn restores_unsent_prompts() {
        let draft = |text: &str, mentioned: &[&str]| Draft {
            text: text.into(),
            attachments: vec![],
            mentioned: mentioned.iter().map(|m| m.to_string()).collect(),
        };
        let unsent = draft("fix the login", &["a.ts"]);
        assert_eq!(restore_unsent(unsent.clone(), None), unsent);
        assert_eq!(
            restore_unsent(unsent.clone(), Some(&draft("  ", &[]))),
            unsent
        );
        assert_eq!(
            restore_unsent(unsent, Some(&draft("and the tests", &["b.ts"]))),
            draft("fix the login\nand the tests", &["a.ts", "b.ts"])
        );
    }
}
