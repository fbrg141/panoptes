use panoptes_harness::{
    Answer, CommandInfo, ImageAttachment, Message, ModelInfo, PromptMode, PromptOutcome,
    SessionSettings,
};
use serde_json::json;

use super::error::{err, not_open, only_in_a_lane};
use super::key::panoptes_id;
use super::status::now_ms;
use super::{Service, ServiceResult, SessionEvent, SessionUpdate, StatusChanged};
use crate::store::{lane_scope, session_scope};

impl Service {
    pub async fn history(&self, key: &str) -> ServiceResult<Vec<Message>> {
        self.registry
            .session(key)
            .await?
            .history()
            .await
            .map_err(err)
    }

    /// Sends a prompt to the addressed session.
    pub async fn prompt(
        &self,
        key: &str,
        text: String,
        images: Vec<ImageAttachment>,
        mode: PromptMode,
    ) -> ServiceResult<PromptOutcome> {
        self.registry
            .session(key)
            .await?
            .prompt(text, images, mode)
            .await
            .map_err(err)
    }

    pub async fn settings(&self, key: &str) -> ServiceResult<SessionSettings> {
        self.registry
            .session(key)
            .await?
            .settings()
            .await
            .map_err(err)
    }

    pub async fn models(&self, key: &str) -> ServiceResult<Vec<ModelInfo>> {
        self.registry
            .session(key)
            .await?
            .models()
            .await
            .map_err(err)
    }

    pub async fn commands(&self, key: &str) -> ServiceResult<Vec<CommandInfo>> {
        self.registry
            .session(key)
            .await?
            .commands()
            .await
            .map_err(err)
    }

    /// Pins a Panoptes session in its record, and any other conversation by
    /// its harness key.
    pub async fn set_pinned(&self, key: String, pinned: bool) -> ServiceResult<()> {
        self.store
            .call(move |store| match panoptes_id(&key) {
                Some(id) => store.set_session_pinned(id, pinned),
                None => store.set_pinned(&key, pinned),
            })
            .await
            .map_err(err)
    }

    /// Archives a session, closing it first, or brings it back. Its lane is
    /// never touched.
    pub async fn set_archived(&self, key: &str, archived: bool) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("archived"))?;
        if archived {
            self.close(key).await;
        }
        self.store
            .call(move |store| store.set_session_archived(id, archived))
            .await
            .map_err(err)
    }

    pub async fn rename(&self, key: &str, title: Option<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("renamed"))?;
        self.store
            .call(move |store| store.set_session_title(id, title.as_deref()))
            .await
            .map_err(err)
    }

    pub async fn set_tags(&self, key: &str, tags: Vec<String>) -> ServiceResult<()> {
        let id = panoptes_id(key).ok_or_else(|| only_in_a_lane("tagged"))?;
        self.store
            .call(move |store| store.set_session_tags(id, &tags))
            .await
            .map_err(err)
    }

    /// Switches the session's model and returns its settings after the
    /// switch, since the thinking levels on offer depend on the model.
    pub async fn set_model(&self, key: &str, model_id: String) -> ServiceResult<SessionSettings> {
        let session = self.registry.session(key).await?;
        session.set_model(model_id).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn set_thinking_level(
        &self,
        key: &str,
        level: String,
    ) -> ServiceResult<SessionSettings> {
        let session = self.registry.session(key).await?;
        session.set_thinking_level(level).await.map_err(err)?;
        session.settings().await.map_err(err)
    }

    pub async fn abort(&self, key: &str) -> ServiceResult<()> {
        self.registry.session(key).await?.abort().await.map_err(err)
    }

    pub async fn answer(
        &self,
        key: &str,
        question_id: String,
        answer: Answer,
    ) -> ServiceResult<()> {
        let (session, status, lane) = self
            .registry
            .open()
            .await
            .get(key)
            .map(|open| (open.session.clone(), open.status.clone(), open.lane))
            .ok_or_else(not_open)?;
        // What an allow covers, read before the answer clears it from the
        // tracker's waiting list.
        let allowed = match answer {
            Answer::Decision { allow: true } => {
                status.lock().unwrap().approval_subject(&question_id)
            }
            _ => None,
        };
        session
            .answer(question_id.clone(), answer)
            .await
            .map_err(err)?;
        if let Some(subject) = allowed {
            let scope = approval_scope(key, lane);
            let remembered = subject.clone();
            self.store
                .call(move |store| store.allow_action(&scope, &remembered))
                .await
                .map_err(err)?;
            // The subject names a command or a file, so it stays out of the
            // log: that one records what happened, never what was said.
            self.log.record("allowed", json!({"key": key}));
        }
        self.log
            .record("answered", json!({"key": key, "id": question_id}));
        let mut tracker = status.lock().unwrap();
        if let Some(status) = tracker.answered(&question_id, now_ms()) {
            self.log
                .record("status", json!({"key": key, "status": status}));
            (self.emit)(SessionEvent {
                key: key.to_string(),
                event: SessionUpdate::Status(StatusChanged { status }),
            });
        }
        Ok(())
    }
}

/// Where an allow in this session is remembered: its lane, so every session
/// working in the same worktree inherits it, or the conversation itself when
/// no lane owns it.
fn approval_scope(key: &str, lane: Option<i64>) -> String {
    match lane {
        Some(lane) => lane_scope(lane),
        None => session_scope(key),
    }
}
