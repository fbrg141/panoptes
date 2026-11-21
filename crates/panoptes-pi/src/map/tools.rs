use panoptes_harness::{FileEdit, ToolCall, ToolKind, ToolOutput, ToolStatus};
use serde_json::Value;

use super::content::{result_images, result_text, truncate, truncate_start};

/// A tool call with what Pi's built-in tools do spelled out neutrally.
/// Extension tools are `Other`, with their first string argument as the
/// subject. The call is pending until Pi reports it running.
pub(super) fn tool_call(id: &str, name: &str, args: Value) -> ToolCall {
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
        images: Vec::new(),
        started_at: None,
        ended_at: None,
    }
}

/// How Pi's `bash` tool reports a non-zero exit, as the last line of its
/// output.
const EXIT_PREFIX: &str = "Command exited with code ";

/// Puts a tool's (partial or final) output on its call. `finished` is
/// `Some(is_error)` once the tool has ended.
pub(super) fn set_output(call: &mut ToolCall, result: &Value, finished: Option<bool>) {
    let text = result_text(result);
    call.images = result_images(result);
    call.status = match finished {
        None => ToolStatus::Running,
        Some(false) => ToolStatus::Done,
        Some(true) => ToolStatus::Failed,
    };
    call.output = match call.kind {
        // The exit code is read from the full text, before any cut.
        ToolKind::Execute => Some(shell_output(text, finished)),
        ToolKind::Edit => {
            let text = truncate(text);
            let edits = match call.output.take() {
                Some(ToolOutput::Edits { edits, .. }) => edits,
                _ => Vec::new(),
            };
            Some(ToolOutput::Edits {
                edits,
                text: (!text.is_empty()).then_some(text),
            })
        }
        _ => (!text.is_empty() || finished.is_some()).then_some(ToolOutput::Text {
            text: truncate(text),
        }),
    };
}

/// Shell output with its exit code. Pi's `bash` succeeds only on exit 0 and
/// otherwise ends its output with "Command exited with code N"; a command
/// that was stopped or timed out says so instead and has no code.
pub(super) fn shell_output(text: String, finished: Option<bool>) -> ToolOutput {
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
    ToolOutput::Shell {
        text: truncate_start(text),
        exit_code,
    }
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
