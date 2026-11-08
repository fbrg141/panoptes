//! Tauri shell: loads the harnesses and exposes [`service::Service`] to the
//! UI as commands, with session events emitted as `session-event`.

mod files;
mod lanes;
mod projects;
mod service;
mod shell_path;
mod store;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use panoptes_harness::{
    Answer, Harness, ImageAttachment, Message, ModelInfo, PromptMode, PromptOutcome,
    SessionSettings,
};
use panoptes_pi::PiHarness;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use service::{
    AdoptionOutcome, ArchiveLaneOutcome, Listing, OpenedSession, Service, ServiceResult,
};
use store::{Lane, Project, ProjectSettings, Store};

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
    lane_id: i64,
    harness: String,
) -> ServiceResult<OpenedSession> {
    state.service.create(lane_id, &harness).await
}

#[tauri::command]
async fn adopt_session(
    state: State<'_, AppState>,
    harness: String,
    key: String,
    locator: String,
    cwd: String,
    lane_id: Option<i64>,
) -> ServiceResult<AdoptionOutcome> {
    state
        .service
        .adopt(&harness, &key, locator, cwd, lane_id)
        .await
}

#[tauri::command]
async fn replace_session(state: State<'_, AppState>, key: String) -> ServiceResult<OpenedSession> {
    state.service.replace(&key).await
}

#[tauri::command]
async fn archive_session(
    state: State<'_, AppState>,
    key: String,
    archived: bool,
) -> ServiceResult<()> {
    state.service.set_archived(&key, archived).await
}

#[tauri::command]
async fn rename_session(
    state: State<'_, AppState>,
    key: String,
    title: Option<String>,
) -> ServiceResult<()> {
    state.service.rename(&key, title).await
}

#[tauri::command]
async fn set_session_tags(
    state: State<'_, AppState>,
    key: String,
    tags: Vec<String>,
) -> ServiceResult<()> {
    state.service.set_tags(&key, tags).await
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
    images: Vec<ImageAttachment>,
    mode: PromptMode,
) -> ServiceResult<PromptOutcome> {
    state.service.prompt(&key, text, images, mode).await
}

#[tauri::command]
async fn session_settings(
    state: State<'_, AppState>,
    key: String,
) -> ServiceResult<SessionSettings> {
    state.service.settings(&key).await
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>, key: String) -> ServiceResult<Vec<ModelInfo>> {
    state.service.models(&key).await
}

#[tauri::command]
async fn set_model(
    state: State<'_, AppState>,
    key: String,
    model_id: String,
) -> ServiceResult<SessionSettings> {
    state.service.set_model(&key, model_id).await
}

#[tauri::command]
async fn set_thinking_level(
    state: State<'_, AppState>,
    key: String,
    level: String,
) -> ServiceResult<SessionSettings> {
    state.service.set_thinking_level(&key, level).await
}

#[tauri::command]
async fn list_files(cwd: String) -> ServiceResult<Vec<String>> {
    tokio::task::spawn_blocking(move || files::list(Path::new(&cwd), files::MAX_FILES))
        .await
        .map_err(|error| service::ServiceError::other(error.to_string()))
}

#[tauri::command]
async fn read_attachment(path: String) -> ServiceResult<files::Attachment> {
    tokio::task::spawn_blocking(move || files::attachment(Path::new(&path)))
        .await
        .map_err(|error| service::ServiceError::other(error.to_string()))?
        .map_err(|error| service::ServiceError::other(format!("{error:#}")))
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
async fn set_pinned(state: State<'_, AppState>, key: String, pinned: bool) -> ServiceResult<()> {
    state.service.set_pinned(key, pinned).await
}

#[tauri::command]
async fn add_project(state: State<'_, AppState>, folder: String) -> ServiceResult<Project> {
    state.service.add_project(folder).await
}

#[tauri::command]
async fn update_project(
    state: State<'_, AppState>,
    id: i64,
    settings: ProjectSettings,
) -> ServiceResult<Project> {
    state.service.update_project(id, settings).await
}

#[tauri::command]
async fn remove_project(state: State<'_, AppState>, id: i64) -> ServiceResult<()> {
    state.service.remove_project(id).await
}

#[tauri::command]
async fn create_lane(
    state: State<'_, AppState>,
    project_id: i64,
    task: String,
    main_checkout: bool,
) -> ServiceResult<Lane> {
    state
        .service
        .create_lane(project_id, task, main_checkout)
        .await
}

#[tauri::command]
fn cancel_lane_creation(state: State<'_, AppState>, project_id: i64) {
    state.service.cancel_lane_creation(project_id);
}

#[tauri::command]
async fn archive_lane(
    state: State<'_, AppState>,
    id: i64,
    strategy: Option<lanes::ArchiveStrategy>,
) -> ServiceResult<ArchiveLaneOutcome> {
    state.service.archive_lane(id, strategy).await
}

#[tauri::command]
async fn restore_lane(state: State<'_, AppState>, id: i64) -> ServiceResult<Lane> {
    state.service.restore_lane(id).await
}

fn load_state(app: &AppHandle) -> anyhow::Result<AppState> {
    let data_dir = app
        .path()
        .app_data_dir()
        .context("Could not find the app data folder")?;
    // Debug builds keep their own store, so a migration that has not shipped
    // never touches the installed app's data.
    let data_dir = if cfg!(debug_assertions) {
        data_dir.join("dev")
    } else {
        data_dir
    };
    let store = Store::open(&data_dir)?;
    // Lanes are split the same way, so a debug lane never takes the folder
    // name an installed app's lane wants.
    let panoptes_home = app
        .path()
        .home_dir()
        .context("Could not find the home folder for lanes")?
        .join("Panoptes");
    let lanes_root = if cfg!(debug_assertions) {
        panoptes_home.join("dev").join("lanes")
    } else {
        panoptes_home.join("lanes")
    };

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

    let emitter = app.clone();
    let service = Service::new(
        harnesses,
        store,
        lanes_root,
        Arc::new(move |event| {
            let _ = emitter.emit("session-event", event);
        }),
    );
    Ok(AppState { info, service })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `pnpm tauri dev` inherits the terminal's PATH; an installed app does not.
    if !cfg!(debug_assertions) {
        shell_path::apply();
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            match load_state(app.handle()) {
                Ok(state) => {
                    app.manage(state);
                }
                // Starting without the store would show an empty app and
                // risk overwriting what could not be read, so stop instead.
                Err(error) => {
                    let message = format!("{error:#}");
                    eprintln!("Panoptes could not start: {message}");
                    // The window already exists, and without the state every
                    // command it makes would fail behind the dialog.
                    for window in app.webview_windows().values() {
                        let _ = window.hide();
                    }
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(message)
                        .title("Panoptes could not start")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_sessions,
            open_session,
            new_session,
            adopt_session,
            replace_session,
            archive_session,
            rename_session,
            set_session_tags,
            session_history,
            send_prompt,
            session_settings,
            list_models,
            set_model,
            set_thinking_level,
            list_files,
            read_attachment,
            abort_run,
            answer_question,
            close_session,
            set_pinned,
            add_project,
            update_project,
            remove_project,
            create_lane,
            cancel_lane_creation,
            archive_lane,
            restore_lane,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Let every agent process shut down cleanly when the app quits.
            if let tauri::RunEvent::ExitRequested { .. } = event
                && let Some(state) = app.try_state::<AppState>()
            {
                tauri::async_runtime::block_on(state.service.close_all());
            }
        });
}
