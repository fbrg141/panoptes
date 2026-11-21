use panoptes_harness::{ContextUsage, ModelInfo};
use serde_json::Value;

/// A Pi model object. Its id is `<provider>/<model id>`, which
/// [`model_ref`] splits again; model ids may contain slashes, providers not.
pub fn model(value: &Value) -> Option<ModelInfo> {
    let provider = value["provider"].as_str()?;
    let id = value["id"].as_str()?;
    Some(ModelInfo {
        id: format!("{provider}/{id}"),
        name: value["name"].as_str().unwrap_or(id).to_string(),
        provider: provider.to_string(),
        context_window: value["contextWindow"].as_u64(),
        images: value["input"]
            .as_array()
            .is_some_and(|input| input.iter().any(|kind| kind == "image")),
    })
}

/// Splits a [`ModelInfo::id`] back into Pi's provider and model id.
pub fn model_ref(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
        .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
}

/// `contextUsage` from `get_session_stats` data.
pub fn context_usage(stats: &Value) -> Option<ContextUsage> {
    let usage = &stats["contextUsage"];
    Some(ContextUsage {
        tokens: usage["tokens"].as_u64(),
        window: usage["contextWindow"].as_u64()?,
    })
}
