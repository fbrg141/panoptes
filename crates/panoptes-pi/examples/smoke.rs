//! End-to-end check against the installed Pi. The test suite only talks to a
//! scripted stand-in, so this is the one place the real protocol is proven:
//! it starts a new session in a temporary folder (with Panoptes' approvals
//! extension loaded, as the app does), sends one prompt, and fails unless
//! the events and the history keep the contract the UI relies on. The
//! session it creates is deleted again at the end.
//!
//! pnpm smoke
//! cargo run -p panoptes-pi --example smoke -- "Reply with just OK"

use std::time::Duration;

use anyhow::{Context, bail, ensure};
use panoptes_harness::{
    AgentSession, Block, Harness, HarnessEvent, PromptMode, Role, StartSession,
};
use panoptes_pi::{PiHarness, locate};

/// A single short reply should never take longer than this.
const RUN_LIMIT: Duration = Duration::from_secs(120);

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Reply with just OK".to_string());
    let install = locate(None)?;
    println!("pi {} at {}", install.version, install.program.display());
    ensure!(
        install.is_supported(),
        "Pi {} is not a version Panoptes supports",
        install.version
    );

    let harness = PiHarness::new(install);
    let sessions = harness.list_sessions().await?;
    println!("{} sessions found", sessions.len());

    let cwd = std::env::temp_dir().join("panoptes-smoke");
    std::fs::create_dir_all(&cwd)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let session = harness
        .start_session(StartSession::new(cwd), tx)
        .await
        .context("Pi did not start a session with the approvals extension loaded")?;
    println!("started {:?}", session.harness_ref());

    let checked = check(session.as_ref(), &mut rx, prompt).await;
    session.shutdown().await;

    let file = std::path::PathBuf::from(session.harness_ref().locator);
    let _ = std::fs::remove_file(&file);
    if let Some(dir) = file.parent() {
        let _ = std::fs::remove_dir(dir); // only succeeds once empty
    }
    checked?;
    println!("ok: the round trip kept the contract");
    Ok(())
}

/// One prompt, then every promise the UI builds on.
async fn check(
    session: &dyn AgentSession,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<HarnessEvent>,
    prompt: String,
) -> anyhow::Result<()> {
    session
        .prompt(prompt, Vec::new(), PromptMode::Start)
        .await?;
    let mut events = Vec::new();
    let run = async {
        while let Some(event) = rx.recv().await {
            println!("{}", serde_json::to_string(&event)?);
            let done = matches!(
                event,
                HarnessEvent::RunSettled { .. } | HarnessEvent::Exited { .. }
            );
            events.push(event);
            if done {
                break;
            }
        }
        anyhow::Ok(())
    };
    tokio::time::timeout(RUN_LIMIT, run)
        .await
        .context("The run did not settle in time")??;

    if let Some(HarnessEvent::Exited { code, stderr_tail }) = events.last() {
        bail!("Pi exited during the run (code {code:?}): {stderr_tail}");
    }
    // Extensions talk outside any run (Pi's `setWidget` and the like
    // arrive as `Raw`, even before the prompt), so they are skipped here;
    // every event of the run itself must follow RunStarted.
    ensure!(
        matches!(
            events
                .iter()
                .find(|event| !matches!(event, HarnessEvent::Raw { .. })),
            Some(HarnessEvent::RunStarted)
        ),
        "The run did not begin with RunStarted"
    );
    ensure!(
        matches!(events.last(), Some(HarnessEvent::RunSettled { .. })),
        "The run did not end with RunSettled"
    );
    let reply = events
        .iter()
        .rev()
        .find_map(|event| match event {
            HarnessEvent::MessageCompleted { message } if message.role == Role::Assistant => {
                Some(message)
            }
            _ => None,
        })
        .context("No assistant message completed")?;
    ensure!(
        reply.error.is_none(),
        "The reply reported an error: {:?}",
        reply.error
    );
    ensure!(
        reply
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Text { text } if !text.trim().is_empty())),
        "The reply has no text"
    );
    ensure!(
        events.iter().any(|event| matches!(
            event,
            HarnessEvent::MessageStarted { message_id, .. } if *message_id == reply.id
        )),
        "The reply completed under a different id than it streamed under"
    );

    let history = session.history().await?;
    println!("history: {}", serde_json::to_string(&history)?);
    ensure!(
        history.iter().any(|message| message.role == Role::User),
        "The history does not hold the prompt"
    );
    ensure!(
        history.iter().any(|message| message.id == reply.id),
        "The history lists the reply under a different id than it streamed under"
    );
    Ok(())
}
