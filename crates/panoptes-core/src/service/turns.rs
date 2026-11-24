//! Turn checkpoint lifecycle. The service owns prompt and event boundaries;
//! Git snapshots and durable rows stay outside the harness adapters.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use panoptes_harness::{HarnessEvent, PromptMode, PromptOutcome, Role, SessionStatus};
use tokio::task::spawn_blocking;

use super::Service;
use super::store::SharedStore;
use crate::lanes::turn::{
    remove_orphan_turn_refs, remove_turn_ref, turn_tree, write_tree_snapshot, write_turn_snapshot,
};
use crate::store::turns::{Boundary, SettleResult, SnapshotResult, Turn, TurnOutcome};
use crate::store::{Lane, LaneState, Store};

/// Per-process state for lane turns. Every method drops the state lock before
/// awaiting Git or SQLite work.
#[derive(Clone)]
pub(super) struct TurnCoordinator {
    store: SharedStore,
    state: Arc<Mutex<HashMap<String, SessionTurns>>>,
}

#[derive(Clone)]
struct PreparedTurn {
    turn: Turn,
    lane: Lane,
    worktrees: Vec<PreparedWorktree>,
}

#[derive(Clone)]
struct PreparedWorktree {
    project_id: i64,
    repo: PathBuf,
    path: PathBuf,
    checkpoint_ref: Option<String>,
    before_ref: String,
    after_ref: String,
    previous_ref: Option<String>,
    previous_tree: Option<String>,
    before_tree: Option<String>,
    after_tree: Option<String>,
    boundary: Boundary,
    human_edits_before: bool,
    after_error: Option<String>,
}

#[derive(Default)]
struct SessionTurns {
    generation: Option<u64>,
    lane_id: Option<i64>,
    session_id: Option<i64>,
    active: Option<PreparedTurn>,
    candidate: Option<PreparedTurn>,
    queued: VecDeque<PreparedTurn>,
}

impl SessionTurns {
    fn matches(&self, generation: u64) -> bool {
        self.generation == Some(generation)
    }
}

impl TurnCoordinator {
    pub(super) fn new(store: SharedStore) -> Self {
        Self {
            store,
            state: Arc::default(),
        }
    }

    /// A new process generation replaces any state a late event from its
    /// predecessor might try to settle.
    pub(super) fn session_started(
        &self,
        key: &str,
        generation: u64,
        lane_id: Option<i64>,
        session_id: Option<i64>,
        writable: bool,
    ) {
        let active_lane = lane_id.and_then(|id| {
            self.store
                .lock()
                .lane(id)
                .ok()
                .filter(|lane| lane.state == LaneState::Active)
        });
        let writable = writable && active_lane.is_some() && session_id.is_some();
        let mut all = self.state.lock().unwrap();
        if writable {
            all.insert(
                key.to_string(),
                SessionTurns {
                    generation: Some(generation),
                    lane_id,
                    session_id,
                    ..Default::default()
                },
            );
        } else {
            all.remove(key);
        }
    }

    /// Reserves a turn and writes a provisional before snapshot before the
    /// harness receives the prompt. A steer modifies a running turn and does
    /// not invent a new one.
    pub(super) async fn prepare_prompt(
        &self,
        key: &str,
        generation: u64,
        prompt: &str,
        mode: PromptMode,
    ) -> anyhow::Result<Option<u32>> {
        if mode == PromptMode::Steer {
            return Ok(None);
        }
        let (lane_id, session_id) = {
            let all = self.state.lock().unwrap();
            let Some(state) = all.get(key).filter(|state| state.matches(generation)) else {
                return Ok(None);
            };
            match (state.lane_id, state.session_id) {
                (Some(lane), Some(session)) => (lane, session),
                _ => return Ok(None),
            }
        };
        let prepared = self
            .create_prepared(
                lane_id,
                session_id,
                None,
                prompt,
                mode == PromptMode::FollowUp,
                Boundary::Unavailable,
            )
            .await?;
        let turn = prepared.turn.turn;
        let prepared = {
            let mut all = self.state.lock().unwrap();
            match all.get_mut(key).filter(|state| state.matches(generation)) {
                Some(state) => {
                    state.candidate = Some(prepared);
                    None
                }
                None => Some(prepared),
            }
        };
        if let Some(prepared) = prepared {
            self.discard(prepared).await;
        }
        Ok(Some(turn))
    }

    /// Resolves the prompt response. An immediate run upgrades its exact
    /// pre-prompt boundary; a queued run waits for its actual `RunStarted`.
    pub(super) async fn prompt_result(
        &self,
        key: &str,
        generation: u64,
        turn: u32,
        outcome: &Result<PromptOutcome, anyhow::Error>,
    ) {
        let candidate = {
            let mut all = self.state.lock().unwrap();
            all.get_mut(key)
                .filter(|state| state.matches(generation))
                .and_then(|state| {
                    (state
                        .candidate
                        .as_ref()
                        .is_some_and(|p| p.turn.turn == turn))
                    .then(|| state.candidate.take())
                    .flatten()
                })
        };
        let Some(prepared) = candidate else {
            return;
        };
        match outcome {
            Ok(PromptOutcome::Started) => {
                let _ = self.mark_boundary(&prepared, Boundary::Exact).await;
                let _ = self.set_queued(&prepared, false).await;
                let mut all = self.state.lock().unwrap();
                if let Some(state) = all.get_mut(key).filter(|s| s.matches(generation)) {
                    state.active = Some(prepared);
                }
            }
            Ok(PromptOutcome::Queued) => {
                let _ = self.set_queued(&prepared, true).await;
                let mut all = self.state.lock().unwrap();
                if let Some(state) = all.get_mut(key).filter(|s| s.matches(generation)) {
                    state.queued.push_back(prepared);
                }
            }
            Ok(PromptOutcome::Handled) | Err(_) => self.discard(prepared).await,
        }
    }

    /// Applies turn-relevant harness events after the status tracker has
    /// folded them, but before the UI sees them.
    pub(super) async fn handle_event(
        &self,
        key: &str,
        generation: u64,
        event: &HarnessEvent,
        status: SessionStatus,
    ) {
        match event {
            HarnessEvent::RunStarted => self.run_started(key, generation).await,
            HarnessEvent::RunSettled { aborted } => {
                let outcome = if *aborted {
                    TurnOutcome::Stopped
                } else if status == SessionStatus::Failed {
                    TurnOutcome::Failed
                } else {
                    TurnOutcome::Done
                };
                self.settle(key, generation, outcome).await;
            }
            HarnessEvent::Exited { .. } => {
                self.settle(key, generation, TurnOutcome::Failed).await;
            }
            HarnessEvent::QueueChanged { queued } => {
                self.queue_changed(key, generation, queued).await
            }
            HarnessEvent::MessageStarted { message_id, role } if *role == Role::User => {
                self.note_message(key, generation, message_id).await;
            }
            HarnessEvent::MessageCompleted { message } if message.role == Role::User => {
                self.note_message(key, generation, &message.id).await;
            }
            _ => {}
        }
    }

    /// Startup adoption of interrupted rows and cleanup of generated orphans.
    pub(super) async fn reconcile(&self) -> anyhow::Result<()> {
        let unfinished = self.store.call(|store| store.unfinished_turns()).await?;
        for turn in unfinished {
            let prepared = {
                let store = self.store.clone();
                spawn_blocking(move || {
                    let store = store.lock();
                    hydrate(&store, turn.clone())
                })
                .await??
            };
            self.reconcile_one(prepared).await?;
        }
        let owned: HashSet<String> = self
            .store
            .call(|store| store.owned_turn_refs())
            .await?
            .into_iter()
            .collect();
        let lanes = self.store.call(|store| store.lanes()).await?;
        let store = self.store.clone();
        spawn_blocking(move || {
            let store = store.lock();
            let mut repos = HashSet::new();
            for lane in lanes {
                for worktree in &lane.worktrees {
                    let repo = PathBuf::from(&store.project(worktree.project_id)?.repo);
                    if repos.insert(repo.clone()) {
                        remove_orphan_turn_refs(&repo, &lane, &owned)?;
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        Ok(())
    }

    async fn create_prepared(
        &self,
        lane_id: i64,
        session_id: i64,
        message_id: Option<&str>,
        prompt: &str,
        queued: bool,
        boundary: Boundary,
    ) -> anyhow::Result<PreparedTurn> {
        let message_id = message_id.map(str::to_string);
        let prompt = prompt.to_string();
        let store = self.store.clone();
        spawn_blocking(move || {
            let store = store.lock();
            let turn =
                store.create_turn(lane_id, session_id, message_id.as_deref(), &prompt, queued)?;
            let prepared = hydrate(&store, turn)?;
            for worktree in &prepared.worktrees {
                match write_turn_snapshot(
                    &worktree.path,
                    &worktree.repo,
                    &prepared.lane,
                    &worktree.before_ref,
                    worktree.previous_ref.as_deref(),
                ) {
                    Ok(tree) => {
                        let human = worktree
                            .previous_tree
                            .as_deref()
                            .is_some_and(|previous| previous != tree);
                        store.set_turn_before(
                            lane_id,
                            prepared.turn.turn,
                            &SnapshotResult {
                                project_id: worktree.project_id,
                                tree,
                                human_edits_before: human,
                            },
                            boundary,
                            None,
                        )?;
                    }
                    Err(error) => store.fail_turn_before(
                        lane_id,
                        prepared.turn.turn,
                        worktree.project_id,
                        &format!("{error:#}"),
                    )?,
                }
            }
            Ok(prepared)
        })
        .await?
    }

    async fn refresh_before(&self, prepared: &PreparedTurn) {
        let prepared = prepared.clone();
        let store = self.store.clone();
        let _ = spawn_blocking(move || {
            let store = store.lock();
            let (lane_id, turn) = (prepared.turn.lane_id, prepared.turn.turn);
            for worktree in &prepared.worktrees {
                match write_turn_snapshot(
                    &worktree.path,
                    &worktree.repo,
                    &prepared.lane,
                    &worktree.before_ref,
                    worktree.previous_ref.as_deref(),
                ) {
                    Ok(tree) => {
                        let human = worktree
                            .previous_tree
                            .as_deref()
                            .is_some_and(|previous| previous != tree);
                        store.set_turn_before(
                            lane_id,
                            turn,
                            &SnapshotResult {
                                project_id: worktree.project_id,
                                tree,
                                human_edits_before: human,
                            },
                            Boundary::Observed,
                            None,
                        )?;
                    }
                    Err(error) => store.fail_turn_before(
                        lane_id,
                        turn,
                        worktree.project_id,
                        &format!("{error:#}"),
                    )?,
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .await;
    }

    async fn run_started(&self, key: &str, generation: u64) {
        let pending = {
            let mut all = self.state.lock().unwrap();
            let Some(state) = all.get_mut(key).filter(|state| state.matches(generation)) else {
                return;
            };
            if state.active.is_some() {
                return;
            }
            state.candidate.take().or_else(|| state.queued.pop_front())
        };
        // A turn begins at Service::prompt. A harness-only RunStarted with no
        // prompt does not invent one, and never blocks event forwarding.
        let Some(prepared) = pending else {
            return;
        };
        self.refresh_before(&prepared).await;
        let mut all = self.state.lock().unwrap();
        if let Some(state) = all.get_mut(key).filter(|state| state.matches(generation)) {
            state.active = Some(prepared);
        }
    }

    async fn settle(&self, key: &str, generation: u64, outcome: TurnOutcome) {
        let prepared = {
            let mut all = self.state.lock().unwrap();
            let Some(state) = all.get_mut(key).filter(|state| state.matches(generation)) else {
                return;
            };
            state
                .active
                .take()
                .or_else(|| state.candidate.take())
                .or_else(|| state.queued.pop_front())
        };
        if let Some(prepared) = prepared {
            let _ = self.write_after(prepared, outcome, false).await;
        }
    }

    async fn write_after(
        &self,
        prepared: PreparedTurn,
        outcome: TurnOutcome,
        recovered: bool,
    ) -> anyhow::Result<()> {
        let store = self.store.clone();
        spawn_blocking(move || {
            let store = store.lock();
            let (lane_id, turn) = (prepared.turn.lane_id, prepared.turn.turn);
            for worktree in &prepared.worktrees {
                if worktree.after_tree.is_some() || worktree.after_error.is_some() {
                    continue;
                }
                match after_snapshot(&prepared.lane, worktree, recovered) {
                    Ok(Some(result)) => store.set_turn_after(lane_id, turn, &result, None)?,
                    Ok(None) => store.fail_turn_after(
                        lane_id,
                        turn,
                        worktree.project_id,
                        "No worktree or archive checkpoint is available",
                    )?,
                    Err(error) => store.fail_turn_after(
                        lane_id,
                        turn,
                        worktree.project_id,
                        &format!("{error:#}"),
                    )?,
                }
            }
            store.settle_turn(lane_id, turn, outcome)?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        Ok(())
    }

    async fn reconcile_one(&self, prepared: PreparedTurn) -> anyhow::Result<()> {
        let store = self.store.clone();
        spawn_blocking(move || {
            let store = store.lock();
            let (lane_id, turn) = (prepared.turn.lane_id, prepared.turn.turn);
            for worktree in &prepared.worktrees {
                match &worktree.before_tree {
                    Some(tree) if worktree.boundary == Boundary::Unavailable => {
                        store.set_turn_before(
                            lane_id,
                            turn,
                            &SnapshotResult {
                                project_id: worktree.project_id,
                                tree: tree.clone(),
                                human_edits_before: worktree.human_edits_before,
                            },
                            Boundary::Observed,
                            None,
                        )?;
                    }
                    Some(_) => {}
                    None => match turn_tree(&worktree.repo, &worktree.before_ref) {
                        Ok(Some(tree)) => {
                            let human = worktree
                                .previous_tree
                                .as_deref()
                                .is_some_and(|previous| previous != tree);
                            store.set_turn_before(
                                lane_id,
                                turn,
                                &SnapshotResult {
                                    project_id: worktree.project_id,
                                    tree,
                                    human_edits_before: human,
                                },
                                Boundary::Observed,
                                None,
                            )?;
                        }
                        Ok(None) => store.fail_turn_before(
                            lane_id,
                            turn,
                            worktree.project_id,
                            "The before snapshot is missing",
                        )?,
                        Err(error) => store.fail_turn_before(
                            lane_id,
                            turn,
                            worktree.project_id,
                            &format!("{error:#}"),
                        )?,
                    },
                }
                if worktree.after_tree.is_some() || worktree.after_error.is_some() {
                    continue;
                }
                match after_snapshot(&prepared.lane, worktree, true) {
                    Ok(Some(result)) => store.set_turn_after(lane_id, turn, &result, None)?,
                    Ok(None) => store.fail_turn_after(
                        lane_id,
                        turn,
                        worktree.project_id,
                        "No worktree or archive checkpoint is available",
                    )?,
                    Err(error) => store.fail_turn_after(
                        lane_id,
                        turn,
                        worktree.project_id,
                        &format!("{error:#}"),
                    )?,
                }
            }
            store.settle_turn(lane_id, turn, TurnOutcome::Stopped)?;
            Ok::<(), anyhow::Error>(())
        })
        .await??;
        Ok(())
    }

    async fn queue_changed(
        &self,
        key: &str,
        generation: u64,
        queued: &[panoptes_harness::QueuedPrompt],
    ) {
        let updates = {
            let mut all = self.state.lock().unwrap();
            let Some(state) = all.get_mut(key).filter(|state| state.matches(generation)) else {
                return;
            };
            let mut updates = Vec::new();
            for prompt in queued
                .iter()
                .filter(|prompt| prompt.mode != PromptMode::Steer)
            {
                let label = first_line(&prompt.text);
                if let Some(prepared) = state
                    .queued
                    .iter_mut()
                    .find(|prepared| prepared.turn.prompt == label)
                {
                    prepared.turn.message_id = Some(prompt.id.clone());
                    updates.push((prepared.turn.lane_id, prepared.turn.turn, prompt.id.clone()));
                    continue;
                }
                if let Some(prepared) = state
                    .candidate
                    .as_mut()
                    .filter(|prepared| prepared.turn.queued && prepared.turn.prompt == label)
                {
                    prepared.turn.message_id = Some(prompt.id.clone());
                    updates.push((prepared.turn.lane_id, prepared.turn.turn, prompt.id.clone()));
                }
            }
            updates
        };
        for (lane_id, turn, message_id) in updates {
            let _ = self
                .store
                .call(move |store| store.set_turn_message_id(lane_id, turn, &message_id))
                .await;
        }
    }

    async fn note_message(&self, key: &str, generation: u64, message_id: &str) {
        let update = {
            let all = self.state.lock().unwrap();
            let Some(state) = all.get(key).filter(|state| state.matches(generation)) else {
                return;
            };
            state
                .active
                .as_ref()
                .or(state.candidate.as_ref())
                .or_else(|| state.queued.front())
                .map(|prepared| (prepared.turn.lane_id, prepared.turn.turn))
        };
        if let Some((lane_id, turn)) = update {
            let message_id = message_id.to_string();
            let _ = self
                .store
                .call(move |store| store.set_turn_message_id(lane_id, turn, &message_id))
                .await;
        }
    }

    async fn mark_boundary(
        &self,
        prepared: &PreparedTurn,
        boundary: Boundary,
    ) -> anyhow::Result<()> {
        let (lane_id, turn) = (prepared.turn.lane_id, prepared.turn.turn);
        self.store
            .call(move |store| store.mark_turn_boundary(lane_id, turn, boundary))
            .await
    }

    async fn set_queued(&self, prepared: &PreparedTurn, queued: bool) -> anyhow::Result<()> {
        let (lane_id, turn) = (prepared.turn.lane_id, prepared.turn.turn);
        self.store
            .call(move |store| store.set_turn_queued(lane_id, turn, queued))
            .await
    }

    async fn discard(&self, prepared: PreparedTurn) {
        let store = self.store.clone();
        let _ = spawn_blocking(move || {
            for worktree in &prepared.worktrees {
                let _ = remove_turn_ref(&worktree.repo, &worktree.before_ref);
                let _ = remove_turn_ref(&worktree.repo, &worktree.after_ref);
            }
            store
                .lock()
                .discard_turn(prepared.turn.lane_id, prepared.turn.turn)
        })
        .await;
    }
}

impl Service {
    /// Reconciles interrupted turn checkpoints at startup. Failure is
    /// reported to the app's caller; it does not replace the existing store.
    pub async fn reconcile_turns(&self) -> anyhow::Result<()> {
        self.turns.reconcile().await
    }
}

fn hydrate(store: &Store, turn: Turn) -> anyhow::Result<PreparedTurn> {
    let lane = store.lane(turn.lane_id)?;
    let mut worktrees = Vec::with_capacity(turn.worktrees.len());
    for row in &turn.worktrees {
        let project = store.project(row.project_id)?;
        let lane_worktree = lane
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == row.project_id)
            .with_context(|| format!("Lane {} lost worktree {}", lane.id, row.project_id))?;
        let previous = store.previous_after(lane.id, turn.turn, row.project_id)?;
        let (previous_ref, previous_tree) = match previous {
            Some((reference, tree)) => (Some(reference), Some(tree)),
            None => (None, None),
        };
        worktrees.push(PreparedWorktree {
            project_id: row.project_id,
            repo: PathBuf::from(&project.repo),
            path: PathBuf::from(&lane_worktree.path),
            checkpoint_ref: lane_worktree.checkpoint_ref.clone(),
            before_ref: row.before_ref.clone(),
            after_ref: row.after_ref.clone(),
            previous_ref,
            previous_tree,
            before_tree: row.before_tree.clone(),
            after_tree: row.after_tree.clone(),
            boundary: row.boundary,
            human_edits_before: row.human_edits_before,
            after_error: row.after_error.clone(),
        });
    }
    Ok(PreparedTurn {
        turn,
        lane,
        worktrees,
    })
}

/// Adopts an existing generated after ref, snapshots a live worktree, or
/// falls back to an archive checkpoint. No user state is changed.
fn after_snapshot(
    lane: &Lane,
    worktree: &PreparedWorktree,
    recovered: bool,
) -> anyhow::Result<Option<SettleResult>> {
    if let Some(tree) = turn_tree(&worktree.repo, &worktree.after_ref)? {
        return Ok(Some(SettleResult {
            project_id: worktree.project_id,
            tree,
            recovered: true,
        }));
    }
    if worktree.path.is_dir() {
        let tree = write_turn_snapshot(
            &worktree.path,
            &worktree.repo,
            lane,
            &worktree.after_ref,
            Some(&worktree.before_ref),
        )?;
        return Ok(Some(SettleResult {
            project_id: worktree.project_id,
            tree,
            recovered,
        }));
    }
    if let Some(checkpoint) = &worktree.checkpoint_ref
        && let Some(tree) = turn_tree(&worktree.repo, checkpoint)?
    {
        write_tree_snapshot(
            &worktree.repo,
            lane,
            &worktree.after_ref,
            &tree,
            Some(&worktree.before_ref),
        )?;
        return Ok(Some(SettleResult {
            project_id: worktree.project_id,
            tree,
            recovered: true,
        }));
    }
    Ok(None)
}

fn first_line(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    if line.is_empty() {
        "Run".to_string()
    } else {
        line.to_string()
    }
}
