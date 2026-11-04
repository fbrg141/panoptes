mod index;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use panoptes_harness::{
    AgentSession, Harness, HarnessEvent, Message, PromptMode, SessionRef, SessionSummary,
    StartSession,
};
use panoptes_pi::PiHarness;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use index::Index;

/// Commands return errors as display strings for the UI.
type CmdResult<T> = Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

fn session_key(harness: &str, id: &str) -> String {
    format!("{harness}:{id}")
}

struct AppState {
    harnesses: HashMap<&'static str, Arc<dyn Harness>>,
    /// Why a harness could not be loaded, keyed by harness id.
    harness_errors: HashMap<&'static str, String>,
    pi_info: Option<HarnessInfo>,
    open: Mutex<HashMap<String, Arc<dyn AgentSession>>>,
    index: std::sync::Mutex<Index>,
}

impl AppState {
    fn harness(&self, id: &str) -> CmdResult<Arc<dyn Harness>> {
        self.harnesses.get(id).cloned().ok_or_else(|| {
            self.harness_errors
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("Unknown harness `{id}`"))
        })
    }

    async fn session(&self, key: &str) -> CmdResult<Arc<dyn AgentSession>> {
        self.open
            .lock()
            .await
            .get(key)
            .cloned()
            .ok_or_else(|| "This session is not open".to_string())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HarnessInfo {
    id: String,
    program: String,
    version: String,
    supported: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    harnesses: Vec<HarnessInfo>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidebarSession {
    key: String,
    #[serde(flatten)]
    summary: SessionSummary,
    project_root: String,
    pinned: bool,
    open: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenedSession {
    key: String,
    session: SessionRef,
    history: Vec<Message>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionEvent {
    key: String,
    event: HarnessEvent,
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        harnesses: state.pi_info.iter().cloned().collect(),
        errors: state.harness_errors.values().cloned().collect(),
    }
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> CmdResult<Vec<SidebarSession>> {
    let mut summaries = Vec::new();
    for harness in state.harnesses.values() {
        summaries.extend(harness.list_sessions().await.map_err(err)?);
    }
    let open: Vec<String> = state.open.lock().await.keys().cloned().collect();
    let mut index = state.index.lock().unwrap();
    Ok(summaries
        .into_iter()
        .map(|summary| {
            let key = session_key(&summary.harness, &summary.id);
            SidebarSession {
                project_root: index.project_root(&summary.cwd),
                pinned: index.file.pinned.contains(&key),
                open: open.contains(&key),
                key,
                summary,
            }
        })
        .collect())
}

/// Starts a session's process and forwards its events to the UI as
/// `session-event`, tagged with the session key.
async fn start(
    app: &AppHandle,
    state: &AppState,
    harness_id: &str,
    request: StartSession,
) -> CmdResult<OpenedSession> {
    let harness = state.harness(harness_id)?;
    let key_slot: Arc<OnceLock<String>> = Arc::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let app = app.clone();
        let key_slot = key_slot.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Events before the session reports its id (startup
                // widgets and the like) have nowhere to go yet.
                let Some(key) = key_slot.get().cloned() else { continue };
                if matches!(event, HarnessEvent::Exited { .. }) {
                    app.state::<AppState>().open.lock().await.remove(&key);
                }
                let _ = app.emit("session-event", SessionEvent { key, event });
            }
        });
    }

    let session: Arc<dyn AgentSession> =
        Arc::from(harness.start_session(request, tx).await.map_err(err)?);
    let session_ref = session.session_ref();
    let key = session_key(&session_ref.harness, &session_ref.id);
    let _ = key_slot.set(key.clone());
    let history = session.history().await.map_err(err)?;
    state.open.lock().await.insert(key.clone(), session);
    Ok(OpenedSession { key, session: session_ref, history })
}

#[tauri::command]
async fn open_session(
    app: AppHandle,
    state: State<'_, AppState>,
    harness: String,
    locator: String,
    cwd: String,
    key: String,
) -> CmdResult<OpenedSession> {
    let existing = state.open.lock().await.get(&key).cloned();
    if let Some(session) = existing {
        let history = session.history().await.map_err(err)?;
        return Ok(OpenedSession { key, session: session.session_ref(), history });
    }
    let request = StartSession::Resume { locator, cwd: PathBuf::from(cwd) };
    start(&app, &state, &harness, request).await
}

#[tauri::command]
async fn new_session(
    app: AppHandle,
    state: State<'_, AppState>,
    harness: String,
    cwd: String,
) -> CmdResult<OpenedSession> {
    start(&app, &state, &harness, StartSession::New { cwd: PathBuf::from(cwd) }).await
}

#[tauri::command]
async fn session_history(state: State<'_, AppState>, key: String) -> CmdResult<Vec<Message>> {
    state.session(&key).await?.history().await.map_err(err)
}

#[tauri::command]
async fn send_prompt(
    state: State<'_, AppState>,
    key: String,
    text: String,
    mode: PromptMode,
) -> CmdResult<()> {
    state.session(&key).await?.prompt(text, mode).await.map_err(err)
}

#[tauri::command]
async fn abort_run(state: State<'_, AppState>, key: String) -> CmdResult<()> {
    state.session(&key).await?.abort().await.map_err(err)
}

#[tauri::command]
async fn close_session(state: State<'_, AppState>, key: String) -> CmdResult<()> {
    let session = state.open.lock().await.remove(&key);
    if let Some(session) = session {
        session.shutdown().await;
    }
    Ok(())
}

#[tauri::command]
fn set_pinned(state: State<'_, AppState>, key: String, pinned: bool) -> CmdResult<()> {
    let mut index = state.index.lock().unwrap();
    if pinned {
        index.file.pinned.insert(key);
    } else {
        index.file.pinned.remove(&key);
    }
    index.save().map_err(err)
}

fn load_state(app: &AppHandle) -> AppState {
    let mut harnesses: HashMap<&'static str, Arc<dyn Harness>> = HashMap::new();
    let mut harness_errors = HashMap::new();
    let mut pi_info = None;

    // A Pi bundled into the app's resources wins over an installed one.
    let bundled = app.path().resource_dir().ok().map(|dir| dir.join("pi/bin/pi"));
    match panoptes_pi::locate(bundled.as_deref()) {
        Ok(install) => {
            pi_info = Some(HarnessInfo {
                id: "pi".to_string(),
                program: install.program.to_string_lossy().into_owned(),
                version: install.version.clone(),
                supported: install.is_supported(),
            });
            harnesses.insert("pi", Arc::new(PiHarness::new(install)));
        }
        Err(error) => {
            harness_errors.insert("pi", err(error));
        }
    }

    let index_path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("index.json");
    AppState {
        harnesses,
        harness_errors,
        pi_info,
        open: Mutex::default(),
        index: std::sync::Mutex::new(Index::load(index_path)),
    }
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
            close_session,
            set_pinned,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Close every Pi process cleanly when the app quits.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                tauri::async_runtime::block_on(async {
                    let sessions: Vec<_> =
                        state.open.lock().await.drain().map(|(_, session)| session).collect();
                    for session in sessions {
                        session.shutdown().await;
                    }
                });
            }
        });
}
