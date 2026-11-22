use panoptes_harness::{Approval, HarnessEvent, NoticeLevel, Question, QuestionKind};
use serde_json::{Value, json};

use super::live::EventMapper;
use super::state::ApprovalReply;
use super::tools::approval_subject;

/// Select options read as "allow" and "reject" when they start with one of
/// these words, compared without case: the permission system asks with
/// "Allow Once", "Allow Always", "Reject" and "Reject with Reason".
const ALLOW: &[&str] = &[
    "yes", "allow", "approve", "ok", "run", "proceed", "continue",
];
const REJECT: &[&str] = &["no", "reject", "block", "deny", "cancel", "skip", "abort"];

/// The status key `approvals.js` tags `tool_call` hook dialogs with.
const APPROVAL_TAG: &str = "panoptes.approval";
/// The permission system publishes this status as soon as the session starts.
const PERMISSION_GUARD_STATUS: &str = "pi-permission-system";

impl EventMapper {
    /// Translates an `extension_ui_request` (see [`extension_ui`]). A
    /// confirm, or a select offering both an allow and a reject option, that
    /// directly follows an approval tag is that call's approval. Any other
    /// dialog, including one a tool asks while it runs, is a question. An
    /// approval the user already allowed for this session, or that its lane
    /// remembers, is answered here and shown to nobody.
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
        let [HarnessEvent::Question { question }] = events.as_slice() else {
            return events;
        };
        let options = match &question.kind {
            QuestionKind::Confirm => Some(None),
            QuestionKind::Select { options } => allow_reject(options).map(Some),
            _ => None,
        };
        let Some(options) = options else {
            return events;
        };
        let subject = self.approval_subject(&tool_call_id);
        let id = question.id.clone();
        if let Some(subject) = subject.as_deref()
            && self.shared.is_allowed(subject)
        {
            // One allow covers the repeats: settle the dialog with the same
            // answer the user would have given, without asking again.
            self.shared
                .remember_auto_answer(dialog_response(&id, &options, true));
            return Vec::new();
        }
        self.shared.record_approval(
            &id,
            ApprovalReply {
                options,
                subject: subject.clone(),
            },
        );
        events = vec![HarnessEvent::Approval {
            approval: Approval {
                id,
                tool_call_id,
                title: question.title.clone(),
                message: question.message.clone(),
                subject,
                expires_at: question.expires_at,
            },
        }];
        events
    }

    /// Responses to approvals that were settled without the user, waiting to
    /// be written back to Pi.
    pub fn take_auto_answers(&self) -> Vec<Value> {
        self.shared.take_auto_answers()
    }

    /// What an allow for the call the tag points at covers. A call the
    /// mapper no longer holds, or one with nothing to name, is asked about
    /// every time.
    fn approval_subject(&self, tool_call_id: &str) -> Option<String> {
        approval_subject(self.calls.get(tool_call_id)?)
    }
}

/// The response that allows or rejects a dialog, in the shape the dialog
/// asks for: a select is answered with its allow or reject option, a confirm
/// with a yes or no.
pub fn dialog_response(id: &str, options: &Option<(String, String)>, allow: bool) -> Value {
    let mut response = json!({"type": "extension_ui_response", "id": id});
    match options {
        Some((yes, no)) => response["value"] = json!(if allow { yes } else { no }),
        None => response["confirmed"] = json!(allow),
    }
    response
}

/// Whether an option reads as an allow, so an option picked from a dialog
/// remembered as an allow covers the repeats.
pub fn is_allow(option: &str) -> bool {
    reads_as(ALLOW, option)
}

/// Whether `option` is one of `words` or a phrase starting with it, as
/// "Allow Always" starts with "allow".
fn reads_as(words: &[&str], option: &str) -> bool {
    let option = option.trim().to_lowercase();
    words.iter().any(|word| {
        option == *word
            || option.strip_prefix(word).is_some_and(|rest| {
                rest.starts_with(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == '-')
            })
    })
}

/// The allow and reject options of a select, if it offers at least one of
/// each. The allow option is the lasting one when the dialog offers a choice
/// ("Allow Always" over "Allow Once"), so one allow covers the repeats.
fn allow_reject(options: &[String]) -> Option<(String, String)> {
    let reject = options
        .iter()
        .find(|option| reads_as(REJECT, option))?
        .clone();
    let allow = options
        .iter()
        .find(|option| option.trim().to_lowercase().contains("always") && is_allow(option))
        .or_else(|| options.iter().find(|option| is_allow(option)))?;
    Some((allow.clone(), reject))
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
