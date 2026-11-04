//! Tauri shell: loads the harnesses and exposes [`service::Service`] to the
//! UI as commands, with session events emitted as `session-event`.

mod index;
mod service;

use std::path::PathBuf;
use std::sync::Arc;

use panoptes_harness::{Answer, Harness, Message, PromptMode, PromptOutcome};
use panoptes_pi::PiHarness;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use index::Index;
use service::{Listing, OpenedSession, Service, ServiceResult};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HarnessInfo {
    id: String,
    program: String,
    version: String,
    supported: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    harnesses: Vec<HarnessInfo>,
    /// Harnesses that could not be loaded, and why.
    errors: Vec<String>,
}

struct AppState {
    info: AppInfo,
    service: Service,
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    state.info.clone()
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> ServiceResult<Listing> {
    Ok(state.service.list().await)
}

#[tauri::command]
async fn open_session(
    state: State<'_, AppState>,
    harness: String,
    key: String,
    locator: String,
    cwd: String,
) -> ServiceResult<OpenedSession> {
    state.service.open(&harness, &key, locator, cwd).await
}

#[tauri::command]
async fn new_session(
    state: State<'_, AppState>,
    harness: String,
    cwd: String,
) -> ServiceResult<OpenedSession> {
    state.service.create(&harness, cwd).await
}

#[tauri::command]
async fn session_history(state: State<'_, AppState>, key: String) -> ServiceResult<Vec<Message>> {
    state.service.history(&key).await
}

#[tauri::command]
async fn send_prompt(
    state: State<'_, AppState>,
    key: String,
    text: String,
    mode: PromptMode,
) -> ServiceResult<PromptOutcome> {
    state.service.prompt(&key, text, mode).await
}

#[tauri::command]
async fn abort_run(state: State<'_, AppState>, key: String) -> ServiceResult<()> {
    state.service.abort(&key).await
}

#[tauri::command]
async fn answer_question(
    state: State<'_, AppState>,
    key: String,
    question_id: String,
    answer: Answer,
) -> ServiceResult<()> {
    state.service.answer(&key, question_id, answer).await
}

#[tauri::command]
async fn close_session(state: State<'_, AppState>, key: String) -> ServiceResult<()> {
    state.service.close(&key).await;
    Ok(())
}

#[tauri::command]
fn set_pinned(state: State<'_, AppState>, key: String, pinned: bool) -> ServiceResult<()> {
    state.service.set_pinned(key, pinned)
}

fn load_state(app: &AppHandle) -> AppState {
    let mut harnesses: Vec<Arc<dyn Harness>> = Vec::new();
    let mut info = AppInfo {
        harnesses: Vec::new(),
        errors: Vec::new(),
    };

    // A Pi bundled into the app's resources wins over an installed one.
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("pi/bin/pi"));
    match panoptes_pi::locate(bundled.as_deref()) {
        Ok(install) => {
            info.harnesses.push(HarnessInfo {
                id: "pi".to_string(),
                program: install.program.to_string_lossy().into_owned(),
                version: install.version.clone(),
                supported: install.is_supported(),
            });
            harnesses.push(Arc::new(PiHarness::new(install)));
        }
        Err(error) => info.errors.push(format!("{error:#}")),
    }

    let index_path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("index.json");
    let emitter = app.clone();
    let service = Service::new(
        harnesses,
        Index::load(index_path),
        Arc::new(move |event| {
            let _ = emitter.emit("session-event", event);
        }),
    );
    AppState { info, service }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = load_state(app.handle());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_sessions,
            open_session,
            new_session,
            session_history,
            send_prompt,
            abort_run,
            answer_question,
            close_session,
            set_pinned,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Let every agent process shut down cleanly when the app quits.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(state.service.close_all());
            }
        });
}
