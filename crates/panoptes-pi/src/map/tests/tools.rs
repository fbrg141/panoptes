use super::*;

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

#[test]
fn tool_results_keep_their_images() {
    let mut call = tool_call("c1", "read", json!({"path": "shot.png"}));
    let result = json!({"content": [
        {"type": "text", "text": "Read image file [image/png]"},
        {"type": "image", "mimeType": "image/png", "data": "aGVsbG8="},
    ]});
    set_output(&mut call, &result, Some(false));
    assert_eq!(
        call.images,
        vec![ToolImage {
            mime_type: "image/png".into(),
            data: "aGVsbG8=".into(),
        }]
    );
    assert_eq!(
        call.output,
        Some(ToolOutput::Text {
            text: "Read image file [image/png]".into()
        })
    );
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

#[test]
fn long_failing_output_keeps_its_exit_code_and_its_end() {
    let long = format!(
        "first line\n{}\nerror: the real reason\n\nCommand exited with code 2",
        "noise\n".repeat(MAX_OUTPUT / 6 + 100)
    );
    let ToolOutput::Shell { text, exit_code } = shell(&long, Some(true)) else {
        unreachable!()
    };
    assert_eq!(exit_code, Some(2));
    assert!(text.ends_with("error: the real reason"), "the end is kept");
    assert!(text.starts_with("… (earlier output truncated)\n"));
    assert!(!text.contains("first line"));
    assert!(text.len() <= MAX_OUTPUT + 64);

    // Other tools keep the start, as before.
    let mut call = tool_call("r1", "read", json!({"path": "big.txt"}));
    set_output(&mut call, &result(&"x".repeat(MAX_OUTPUT * 2)), Some(false));
    let Some(ToolOutput::Text { text }) = call.output else {
        unreachable!()
    };
    assert!(text.ends_with("… (output truncated)"));
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
