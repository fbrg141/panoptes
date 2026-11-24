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
    /// The turn whose run is going on now.
    active: Option<PreparedTurn>,
    /// Turns whose prompt returned Started while another turn was still
    /// active. Ordered, because a harness that accepted a second prompt
    /// before the first settled runs them in that order.
    waiting: VecDeque<PreparedTurn>,
    /// A prompt whose response has not been returned yet.
    candidate: Option<PreparedTurn>,
    /// Prompts the harness has queued, in its order.
    queued: VecDeque<PreparedTurn>,
}

impl SessionTurns {
    fn matches(&self, generation: u64) -> bool {
        self.generation == Some(generation)
    }

    /// The turn a message belongs to: the running one, then one that has
    /// started, then the prompt in flight, then the one waiting its turn.
    fn current(&self) -> Option<&PreparedTurn> {
        self.active
            .as_ref()
            .or_else(|| self.waiting.front())
            .or(self.candidate.as_ref())
            .or_else(|| self.queued.front())
    }

    /// Every turn this generation still holds, running ones first.
    fn drain(
        &mut self,
    ) -> (
        Vec<PreparedTurn>, /* started */
        Vec<PreparedTurn>, /* never ran */
    ) {
        let mut started = Vec::new();
        started.extend(self.active.take());
        started.extend(self.waiting.drain(..));
        let mut never_ran = Vec::new();
        never_ran.extend(self.candidate.take());
        never_ran.extend(self.queued.drain(..));
        (started, never_ran)
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
    /// predecessor might try to settle. Turns the predecessor still held are
    /// finished here, with the worktree as it is now: waiting for startup
    /// reconciliation would credit their edits to whoever comes next.
    pub(super) async fn session_started(
        &self,
        key: &str,
        generation: u64,
        lane_id: Option<i64>,
        session_id: Option<i64>,
        writable: bool,
    ) {
        // Read through the store's own thread: SQLite never runs on the async
        // runtime, and a snapshot may be holding the same lock.
        let active_lane = match lane_id {
            Some(id) => self
                .store
                .call(move |store| {
                    Ok(store
                        .lane(id)
                        .ok()
                        .filter(|lane| lane.state == LaneState::Active))
                })
                .await
                .unwrap_or(None),
            None => None,
        };
        let writable = writable && active_lane.is_some() && session_id.is_some();
        let previous = {
            let mut all = self.state.lock().unwrap();
            all.remove(key)
        };
        if let Some(previous) = previous {
            self.finish_dropped(previous).await;
        }
        if writable {
            let mut all = self.state.lock().unwrap();
            all.insert(
                key.to_string(),
                SessionTurns {
                    generation: Some(generation),
                    lane_id,
                    session_id,
                    ..Default::default()
                },
            );
        }
    }

    /// Finishes the turns a replaced or closed session left behind: the ones
    /// that started are settled from the worktree as it is now, and the ones
    /// that never ran leave nothing to settle.
    async fn finish_dropped(&self, mut state: SessionTurns) {
        let (started, never_ran) = state.drain();
        for prepared in started {
            let _ = self
                .write_after(prepared, TurnOutcome::Stopped, false)
                .await;
        }
        for prepared in never_ran {
            self.discard(prepared).await;
        }
    }

    /// A session's process went away without a new one replacing it.
    pub(super) async fn session_closed(&self, key: &str) {
        let previous = self.state.lock().unwrap().remove(key);
        if let Some(previous) = previous {
            self.finish_dropped(previous).await;
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
                    // A run that has not settled yet keeps its turn: the new
                    // one waits behind it rather than replacing it, which
                    // would leave the first turn unsettled and the late
                    // RunSettled settling the wrong one.
                    if state.active.is_none() {
                        state.active = Some(prepared);
                    } else {
                        state.waiting.push_back(prepared);
                    }
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
    /// One turn that cannot be restored is given up on rather than left to
    /// fail again at every launch, and the rest still run.
    pub(super) async fn reconcile(&self) -> anyhow::Result<()> {
        let mut problems = Vec::new();
        let unfinished = self.store.call(|store| store.unfinished_turns()).await?;
        for turn in unfinished {
            let (lane_id, number) = (turn.lane_id, turn.turn);
            let store = self.store.clone();
            let prepared = spawn_blocking(move || {
                let store = store.lock();
                hydrate(&store, turn)
            })
            .await
            .unwrap_or_else(|error| Err(error.into()));
            let restored = match prepared {
                Ok(prepared) => self.reconcile_one(prepared).await,
                Err(error) => Err(error),
            };
            if let Err(error) = restored {
                let reason = format!("{error:#}");
                if let Err(error) = self
                    .store
                    .call(move |store| store.abandon_turn(lane_id, number, &reason))
                    .await
                {
                    problems.push(format!(
                        "Turn {number} of lane {lane_id} could not be settled: {error}"
                    ));
                }
            }
        }
        let owned: HashSet<String> = self
            .store
            .call(|store| store.owned_turn_refs())
            .await?
            .into_iter()
            .collect();
        let lanes = self.store.call(|store| store.lanes()).await?;
        let store = self.store.clone();
        let errors = spawn_blocking(move || {
            let store = store.lock();
            let mut seen = HashSet::new();
            let mut errors = Vec::new();
            for lane in lanes {
                for worktree in &lane.worktrees {
                    let repo = match store.project(worktree.project_id) {
                        Ok(project) => PathBuf::from(&project.repo),
                        Err(error) => {
                            errors.push(format!("Could not read a turn's project: {error:#}"));
                            continue;
                        }
                    };
                    // Generated refs live under the lane's own prefix, so each
                    // lane in a repository needs its own pass. Keying on the
                    // repository alone skipped every lane after the first.
                    if !seen.insert((repo.clone(), lane.name.clone())) {
                        continue;
                    }
                    if let Err(error) = remove_orphan_turn_refs(&repo, &lane, &owned) {
                        errors.push(format!(
                            "Could not clean up turn refs in {}: {error:#}",
                            repo.display()
                        ));
                    }
                }
            }
            errors
        })
        .await
        .unwrap_or_else(|_| vec!["Turn ref cleanup stopped unexpectedly".to_string()]);
        problems.extend(errors);
        if !problems.is_empty() {
            anyhow::bail!(problems.join("; "));
        }
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
            match prepare_turn(&store, turn.clone(), boundary) {
                Ok(prepared) => Ok(prepared),
                Err(error) => {
                    // The row is committed already, so a preparation that
                    // fails must take it back: leaving it would consume a turn
                    // number and be settled as Stopped at the next launch,
                    // with an after snapshot taken long after the fact.
                    let _ = discard_generated_turn(&store, &turn);
                    Err(error)
                }
            }
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
                // The parent and the comparison base are read again here: when
                // this prompt was queued the turn ahead of it had no after
                // tree, so the values from `hydrate` were the turn before's.
                // Its agent edits would then count as the human's, and the
                // snapshot would be parented to the wrong ref.
                let previous = store.previous_after(lane_id, turn, worktree.project_id)?;
                let (previous_ref, previous_tree) = match previous {
                    Some((reference, tree)) => (Some(reference), Some(tree)),
                    None => (None, None),
                };
                match write_turn_snapshot(
                    &worktree.path,
                    &worktree.repo,
                    &prepared.lane,
                    &worktree.before_ref,
                    previous_ref.as_deref(),
                ) {
                    Ok(tree) => {
                        let human = previous_tree
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
        // A run that starts now belongs either to a turn whose prompt already
        // answered Started while an earlier turn was still active, or to a
        // prompt the harness queued. A prompt whose answer has not arrived is
        // not touched here: it already holds its exact pre-prompt boundary,
        // and re-snapshotting could catch the agent mid-edit.
        enum Next {
            Ready(PreparedTurn),
            Refresh(PreparedTurn),
        }
        let pending = {
            let mut all = self.state.lock().unwrap();
            let Some(state) = all.get_mut(key).filter(|state| state.matches(generation)) else {
                return;
            };
            if state.active.is_some() {
                return;
            }
            match state.waiting.pop_front() {
                Some(prepared) => Some(Next::Ready(prepared)),
                None => state.queued.pop_front().map(Next::Refresh),
            }
        };
        // A turn begins at Service::prompt. A harness-only RunStarted with no
        // prompt does not invent one, and never blocks event forwarding.
        let Some(pending) = pending else {
            return;
        };
        let prepared = match pending {
            Next::Ready(prepared) => prepared,
            Next::Refresh(prepared) => {
                self.refresh_before(&prepared).await;
                prepared
            }
        };
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
            // Only turns that ran: a queued follow-up that never started must
            // not be given this run's after snapshot.
            state.active.take().or_else(|| state.waiting.pop_front())
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
                .current()
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
            let store = store.lock();
            discard_generated_turn(&store, &prepared.turn)
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

/// Takes the pre-prompt snapshot of every worktree of a turn it just created.
fn prepare_turn(store: &Store, turn: Turn, boundary: Boundary) -> anyhow::Result<PreparedTurn> {
    let prepared = hydrate(store, turn)?;
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
                    prepared.turn.lane_id,
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
                prepared.turn.lane_id,
                prepared.turn.turn,
                worktree.project_id,
                &format!("{error:#}"),
            )?,
        }
    }
    Ok(prepared)
}

/// Drops a turn that never became a run: its rows, and any refs already
/// written for it. Errors are ignored; there is nothing left to fix.
fn discard_generated_turn(store: &Store, turn: &Turn) -> anyhow::Result<()> {
    for worktree in &turn.worktrees {
        if let Ok(project) = store.project(worktree.project_id) {
            let repo = PathBuf::from(&project.repo);
            let _ = remove_turn_ref(&repo, &worktree.before_ref);
            let _ = remove_turn_ref(&repo, &worktree.after_ref);
        }
    }
    store.discard_turn(turn.lane_id, turn.turn)
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
