use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use serde::Serialize;
use serde_json::json;

use super::error::err;
use super::{ErrorKind, Service, ServiceError, ServiceResult};
use crate::lanes::{self, ArchiveResult, ArchiveStrategy, DeleteResult, DetachedHead};
use crate::store::{Lane, LaneState};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ArchiveLaneOutcome {
    Archived,
    Dirty {
        lane: Lane,
    },
    DetachedHead {
        lane: Lane,
        detached_head: DetachedHead,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DeleteLaneOutcome {
    Deleted,
    DetachedHead {
        lane: Lane,
        detached_head: DetachedHead,
    },
}

impl Service {
    /// Creates a durable lane before any session exists. One creation per
    /// project at a time keeps name allocation and the
    /// one-active-main-checkout rule consistent; the store re-checks paths.
    pub async fn create_lane(
        &self,
        project_id: i64,
        task: String,
        main_checkout: bool,
    ) -> ServiceResult<Lane> {
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.lane_cancellations.lock().unwrap();
            if active.contains_key(&project_id) {
                return Err(ServiceError::new(
                    ErrorKind::Rejected,
                    "A lane is already being created for this project",
                ));
            }
            active.insert(project_id, cancelled.clone());
        }
        let root = self.lanes_root.clone();
        let in_setup = cancelled.clone();
        let store = self.store.clone();
        // The store lock is taken only for the reads and the final insert.
        // Git and the setup command can take minutes and must not block
        // every other store call. Creations for one project are serialised
        // by the cancellation map above.
        let result = tokio::task::spawn_blocking(move || {
            let task = task.trim().to_string();
            if task.is_empty() {
                anyhow::bail!("A lane needs a task");
            }
            let (project, taken) = {
                let store = store.lock();
                let project = store.project(project_id)?;
                let lanes = store.lanes()?;
                if main_checkout && has_active_main_checkout(&lanes, project_id, None) {
                    anyhow::bail!("This project already has an active main-checkout lane");
                }
                let taken: Vec<String> = lanes
                    .into_iter()
                    .filter(|lane| lane.project_id == project_id)
                    .map(|lane| lane.name)
                    .collect();
                (project, taken)
            };
            let new = lanes::create(
                &root,
                &project,
                &lanes::slug(&task),
                &taken,
                &task,
                main_checkout,
                &in_setup,
            )?;
            let added = store.lock().add_lane(&new);
            added.inspect_err(|_| lanes::discard_created(&project, &new))
        })
        .await
        .context("The lane creation stopped")
        .and_then(|result| result);
        self.lane_cancellations.lock().unwrap().remove(&project_id);
        match result {
            Ok(lane) => {
                self.log.record(
                    "laneCreated",
                    json!({
                        "lane": lane.id,
                        "project": lane.project_id,
                        "mainCheckout": lane.worktrees.iter().any(|tree| tree.main_checkout),
                    }),
                );
                Ok(lane)
            }
            Err(_) if cancelled.load(Ordering::Relaxed) => Err(ServiceError::new(
                ErrorKind::Cancelled,
                "Lane creation cancelled",
            )),
            Err(error) => Err(err(error)),
        }
    }

    pub fn cancel_lane_creation(&self, project_id: i64) {
        if let Some(cancelled) = self.lane_cancellations.lock().unwrap().get(&project_id) {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Disk usage and safely archivable idle lanes for one project. This is
    /// deliberately on demand: measuring worktrees is too costly for `list`.
    /// Archives a clean lane immediately. A dirty one is returned unchanged
    /// until the caller picks how its work should be preserved.
    #[cfg(test)]
    pub async fn archive_lane(
        &self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
    ) -> ServiceResult<ArchiveLaneOutcome> {
        self.archive_lane_recovering(id, strategy, false).await
    }

    /// Archives after optionally performing the branch recovery the user
    /// explicitly chose in the detached-HEAD dialog.
    pub async fn archive_lane_recovering(
        &self,
        id: i64,
        strategy: Option<ArchiveStrategy>,
        recreate_branch: bool,
    ) -> ServiceResult<ArchiveLaneOutcome> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's sessions before archiving it",
            ));
        }
        let store = self.store.clone();
        // As in `create_lane`, Git (and its commit hooks) runs without the
        // store lock; `lane_busy` keeps two changes to one lane apart.
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Active {
                    anyhow::bail!("That lane is already archived");
                }
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            // The checkpoint ref is stored before the folder goes, so an
            // archive that stops in between can be finished later.
            let record = |reference: Option<&str>| store.lock().set_lane_checkpoint(id, reference);
            match lanes::archive_recovering(&project, &lane, strategy, recreate_branch, &record)? {
                ArchiveResult::Dirty => Ok(ArchiveLaneOutcome::Dirty { lane }),
                ArchiveResult::DetachedHead(detached_head) => {
                    Ok(ArchiveLaneOutcome::DetachedHead {
                        lane,
                        detached_head,
                    })
                }
                ArchiveResult::Archived { checkpoint_ref } => {
                    store.lock().archive_lane(id, checkpoint_ref.as_deref())?;
                    Ok(ArchiveLaneOutcome::Archived)
                }
            }
        })
        .await
        .context("The lane archive stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|outcome| {
            if matches!(outcome, ArchiveLaneOutcome::Archived) {
                self.log
                    .record("laneArchived", json!({"lane": id, "strategy": strategy}));
            }
        })
    }

    /// Saves a detached HEAD on the lane branch without removing the
    /// worktree, so the user can then choose how to archive its changes.
    pub async fn create_lane_branch(&self, id: i64) -> ServiceResult<()> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's sessions before recovering its branch",
            ));
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let lane = {
                let store = store.lock();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Active {
                    anyhow::bail!("That lane is archived; restore it before recovering its branch");
                }
                lane
            };
            let worktree = lane
                .worktrees
                .first()
                .filter(|_| lane.worktrees.len() == 1)
                .context("This operation needs exactly one worktree")?;
            lanes::create_branch_here(Path::new(&worktree.path), &lane)
        })
        .await
        .context("The lane branch recovery stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    pub async fn restore_lane(&self, id: i64) -> ServiceResult<Lane> {
        let _busy = self.lane_busy(id)?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock();
                let lane = store.lane(id)?;
                if lane.state != LaneState::Archived {
                    anyhow::bail!("That lane is not archived");
                }
                let lanes = store.lanes()?;
                let main = lane.worktrees.iter().any(|tree| tree.main_checkout);
                if main && has_active_main_checkout(&lanes, lane.project_id, Some(id)) {
                    anyhow::bail!("This project already has an active main-checkout lane");
                }
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            lanes::restore(&project, &lane)?;
            store.lock().restore_lane(id)
        })
        .await
        .context("The lane restore stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|_| self.log.record("laneRestored", json!({"lane": id})))
    }

    /// Permanently deletes a lane after the UI's explicit confirmation.
    /// Open sessions are refused rather than killed underneath their user.
    #[cfg(test)]
    pub async fn delete_lane(&self, id: i64) -> ServiceResult<DeleteLaneOutcome> {
        self.delete_lane_detached(id, false).await
    }

    /// `allow_detached` is accepted only for a clean, reachable detached HEAD
    /// and is checked again by the lane operation.
    pub async fn delete_lane_detached(
        &self,
        id: i64,
        allow_detached: bool,
    ) -> ServiceResult<DeleteLaneOutcome> {
        let _busy = self.lane_busy(id)?;
        self.close_lane_previews(id).await;
        if self.registry.lane_in_use(id).await {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "Close this lane's session before deleting it",
            ));
        }
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let (lane, project) = {
                let store = store.lock();
                let lane = store.lane(id)?;
                let project = store.project(lane.project_id)?;
                (lane, project)
            };
            match lanes::delete_detached(&project, &lane, allow_detached)? {
                DeleteResult::Deleted => {
                    store.lock().delete_lane(id)?;
                    Ok(DeleteLaneOutcome::Deleted)
                }
                DeleteResult::DetachedHead(detached_head) => Ok(DeleteLaneOutcome::DetachedHead {
                    lane,
                    detached_head,
                }),
            }
        })
        .await
        .context("The lane deletion stopped")
        .and_then(|result| result)
        .map_err(err)
        .inspect(|outcome| {
            if matches!(outcome, DeleteLaneOutcome::Deleted) {
                self.log.record("laneDeleted", json!({"lane": id}));
            }
        })
    }

    /// Marks a lane as being archived, restored or deleted until the guard drops.
    fn lane_busy(&self, id: i64) -> ServiceResult<LaneBusy> {
        if !self.busy_lanes.lock().unwrap().insert(id) {
            return Err(ServiceError::new(
                ErrorKind::Rejected,
                "That lane is already being archived, restored or deleted",
            ));
        }
        Ok(LaneBusy {
            lanes: self.busy_lanes.clone(),
            id,
        })
    }
}

struct LaneBusy {
    lanes: Arc<std::sync::Mutex<HashSet<i64>>>,
    id: i64,
}

impl Drop for LaneBusy {
    fn drop(&mut self) {
        self.lanes.lock().unwrap().remove(&self.id);
    }
}

/// Whether another active lane of `project_id` already uses the main checkout.
fn has_active_main_checkout(lanes: &[Lane], project_id: i64, except: Option<i64>) -> bool {
    lanes.iter().any(|lane| {
        lane.project_id == project_id
            && Some(lane.id) != except
            && lane.state == LaneState::Active
            && lane.worktrees.iter().any(|tree| tree.main_checkout)
    })
}

/// Where a lane's sessions run: its (for now only) worktree.
pub(super) fn lane_cwd(lane: &Lane) -> String {
    lane.worktrees
        .first()
        .map(|worktree| worktree.path.clone())
        .unwrap_or_default()
}
