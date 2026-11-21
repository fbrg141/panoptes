use panoptes_harness::{Approval, HarnessEvent, NoticeLevel, Question, QuestionKind};
use serde_json::Value;

use super::live::EventMapper;

/// Select options read as "allow" and "reject" when a dialog offers exactly
/// one of each, compared without case.
const ALLOW: &[&str] = &[
    "yes", "allow", "approve", "ok", "run", "proceed", "continue",
];
const REJECT: &[&str] = &["no", "block", "deny", "reject", "cancel", "skip", "abort"];

/// The status key `approvals.js` tags `tool_call` hook dialogs with.
const APPROVAL_TAG: &str = "panoptes.approval";
/// The permission system publishes this status as soon as the session starts.
const PERMISSION_GUARD_STATUS: &str = "pi-permission-system";

impl EventMapper {
    /// Translates an `extension_ui_request` (see [`extension_ui`]). A
    /// confirm, or a select between one allow and one reject option, that
    /// directly follows an approval tag is that call's approval. Any other
    /// dialog, including one a tool asks while it runs, is a question.
    pub fn ui_request(&mut self, request: &Value, now: i64) -> Vec<HarnessEvent> {
        if request["method"] == "setStatus" && request["statusKey"] == PERMISSION_GUARD_STATUS {
            self.shared.set_permission_guard();
            return Vec::new();
        }
        if request["method"] == "setStatus" && request["statusKey"] == APPROVAL_TAG {
            self.approval_for = request["statusText"].as_str().map(str::to_string);
            return Vec::new();
        }
        let mut events = extension_ui(request, now);
        let Some(tool_call_id) = self.approval_for.take() else {
            return events;
        };
        if let [HarnessEvent::Question { question }] = events.as_slice() {
            let reply = match &question.kind {
                QuestionKind::Confirm => Some(None),
                QuestionKind::Select { options } => allow_reject(options).map(Some),
                _ => None,
            };
            if let Some(reply) = reply {
                self.shared.record_approval(&question.id, reply);
                events = vec![HarnessEvent::Approval {
                    approval: Approval {
                        id: question.id.clone(),
                        tool_call_id,
                        title: question.title.clone(),
                        message: question.message.clone(),
                        expires_at: question.expires_at,
                    },
                }];
            }
        }
        events
    }
}

/// The allow and reject options of a select, if it offers exactly one of
/// each.
fn allow_reject(options: &[String]) -> Option<(String, String)> {
    let is = |list: &[&str], option: &String| list.contains(&option.trim().to_lowercase().as_str());
    match options {
        [a, b] if is(ALLOW, a) && is(REJECT, b) => Some((a.clone(), b.clone())),
        [a, b] if is(REJECT, a) && is(ALLOW, b) => Some((b.clone(), a.clone())),
        _ => None,
    }
}

/// Translates an `extension_ui_request`. Dialogs become questions the user
/// answers; `notify` becomes a notice; status, widget and title updates have
/// no neutral meaning yet.
pub fn extension_ui(request: &Value, now_ms: i64) -> Vec<HarnessEvent> {
    let text = |key: &str| request[key].as_str().map(str::to_string);
    let kind = match request["method"].as_str().unwrap_or_default() {
        "select" => QuestionKind::Select {
            options: request["options"]
                .as_array()
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "confirm" => QuestionKind::Confirm,
        "input" => QuestionKind::Input {
            placeholder: text("placeholder"),
        },
        "editor" => QuestionKind::Editor {
            prefill: text("prefill"),
        },
        "notify" => {
            let level = match request["notifyType"].as_str() {
                Some("error") => NoticeLevel::Error,
                Some("warning") => NoticeLevel::Warning,
                _ => NoticeLevel::Info,
            };
            return vec![HarnessEvent::Notice {
                level,
                text: text("message").unwrap_or_default(),
            }];
        }
        _ => {
            return vec![HarnessEvent::Raw {
                payload: request.clone(),
            }];
        }
    };
    let Some(id) = text("id") else {
        return Vec::new();
    };
    vec![HarnessEvent::Question {
        question: Question {
            id,
            title: text("title").unwrap_or_else(|| "An extension needs your input".to_string()),
            message: text("message"),
            kind,
            expires_at: request["timeout"].as_i64().map(|timeout| now_ms + timeout),
        },
    }]
}
