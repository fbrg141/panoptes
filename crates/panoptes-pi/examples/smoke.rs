//! End-to-end check against the installed Pi: lists sessions, starts a new
//! session in a temporary folder, sends one prompt and prints the events.
//! The session it creates is deleted again at the end.
//!
//! cargo run -p panoptes-pi --example smoke -- "Reply with just OK"

use panoptes_harness::{Harness, HarnessEvent, PromptMode, StartSession};
use panoptes_pi::{PiHarness, locate};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with just OK".to_string());
    let install = locate(None)?;
    println!(
        "pi {} at {} (supported: {})",
        install.version,
        install.program.display(),
        install.is_supported()
    );

    let harness = PiHarness::new(install);
    let sessions = harness.list_sessions().await?;
    println!("{} sessions found", sessions.len());

    let cwd = std::env::temp_dir().join("panoptes-smoke");
    std::fs::create_dir_all(&cwd)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let session = harness.start_session(StartSession::New { cwd }, tx).await?;
    println!("started {:?}", session.session_ref());

    session.prompt(prompt, PromptMode::Start).await?;
    while let Some(event) = rx.recv().await {
        println!("{}", serde_json::to_string(&event)?);
        if matches!(
            event,
            HarnessEvent::RunSettled { .. } | HarnessEvent::Exited { .. }
        ) {
            break;
        }
    }
    println!(
        "history: {}",
        serde_json::to_string(&session.history().await?)?
    );
    session.shutdown().await;

    let file = std::path::PathBuf::from(session.session_ref().locator);
    let _ = std::fs::remove_file(&file);
    if let Some(dir) = file.parent() {
        let _ = std::fs::remove_dir(dir); // only succeeds once empty
    }
    Ok(())
}
