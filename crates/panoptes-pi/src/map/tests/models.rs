use super::*;

#[test]
fn models_round_trip_through_their_id() {
    let info = model(&json!({
        "id": "anthropic/claude-x",
        "name": "Claude X",
        "provider": "openrouter",
        "contextWindow": 200000,
        "input": ["text", "image"]
    }))
    .unwrap();
    assert_eq!(info.id, "openrouter/anthropic/claude-x");
    assert_eq!(info.context_window, Some(200_000));
    assert!(info.images);
    assert_eq!(
        model_ref(&info.id),
        Some(("openrouter", "anthropic/claude-x"))
    );
    assert_eq!(model_ref("no-provider"), None);
}

#[test]
fn context_usage_needs_a_window() {
    let known = json!({"contextUsage": {"tokens": 60000, "contextWindow": 200000}});
    assert_eq!(
        context_usage(&known),
        Some(ContextUsage {
            tokens: Some(60_000),
            window: 200_000
        })
    );
    let compacted = json!({"contextUsage": {"tokens": null, "contextWindow": 200000}});
    assert_eq!(context_usage(&compacted).unwrap().tokens, None);
    assert_eq!(context_usage(&json!({})), None);
}
