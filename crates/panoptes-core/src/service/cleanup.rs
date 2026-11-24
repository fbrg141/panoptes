use std::collections::{HashMap, HashSet};

use anyhow::Context;

use super::error::err;
use super::status::now_ms;
use super::{Service, ServiceResult};
use crate::cleanup::{self, IdleLane, LaneCleanup, Pruned};
use crate::store::{Lane, LaneState, Project};

impl Service {
    pub async fn lane_cleanup(&self, project_id: i64) -> ServiceResult<LaneCleanup> {
        let listing = self.list().await;
        let activity_complete = listing.errors.is_empty();
        let (idle_days, creation_times) = self
            .store
            .call(move |store| {
                store.project(project_id)?;
                Ok((
                    store.preferences()?.cleanup_idle_days,
                    store.lane_creation_times()?,
                ))
            })
            .await
            .map_err(err)?;
        let created: HashMap<i64, i64> = creation_times.into_iter().collect();
        let active: Vec<(Project, Lane)> = listing
            .lanes
            .iter()
            .filter(|lane| lane.state == LaneState::Active)
            .filter_map(|lane| {
                let project = listing
                    .projects
                    .iter()
                    .find(|project| project.id == lane.project_id)?;
                Some((project.clone(), lane.clone()))
            })
            .collect();
        let scan_projects = listing.projects.clone();
        let scan_lanes = listing.lanes.clone();
        let (scan, inventory) = tokio::task::spawn_blocking(move || {
            let scan = cleanup::scan(project_id, &active);
            // The scan has just measured the lane worktrees: measuring them
            // again would double the dialog's wait for nothing.
            let inventory = cleanup::inventory(&scan_projects, &scan_lanes, &scan.sizes);
            (scan, inventory)
        })
        .await
        .context("The worktree cleanup scan stopped")
        .map_err(err)?;

        // Starting sessions are not listed as open yet. Include both sets so
        // cleanup can never archive a lane under a process that is starting.
        let in_use = self.registry.lanes_in_use().await;
        let busy_lanes = self.busy_lanes.lock().unwrap().clone();
        let busy_writers: HashSet<i64> = listing
            .navigation
            .iter()
            .filter(|entry| entry.busy_writer.is_some())
            .map(|entry| entry.lane_id)
            .collect();

        let mut candidates = Vec::new();
        if activity_complete {
            let now = now_ms();
            for lane in listing
                .lanes
                .iter()
                .filter(|lane| lane.project_id == project_id && lane.state == LaneState::Active)
            {
                if in_use.contains(&lane.id)
                    || busy_lanes.contains(&lane.id)
                    || busy_writers.contains(&lane.id)
                {
                    continue;
                }
                let last_activity = listing
                    .sessions
                    .iter()
                    .filter(|session| session.lane_id == Some(lane.id))
                    .fold(
                        created.get(&lane.id).copied().unwrap_or(0),
                        |latest, session| latest.max(session.updated_at),
                    );
                if !cleanup::is_idle(last_activity, now, idle_days) {
                    continue;
                }
                let Some(scanned) = scan.by_lane.get(&lane.id) else {
                    continue;
                };
                let Some(preservation) = scanned.preservation else {
                    continue;
                };
                candidates.push(IdleLane {
                    lane: lane.clone(),
                    last_activity,
                    bytes: scanned.bytes,
                    preservation,
                });
            }
        }
        candidates.sort_by(|a, b| {
            b.bytes
                .unwrap_or(0)
                .cmp(&a.bytes.unwrap_or(0))
                .then_with(|| a.last_activity.cmp(&b.last_activity))
        });
        let mut errors = listing.errors;
        errors.extend(scan.errors);
        Ok(LaneCleanup {
            project: scan.project,
            total: scan.total,
            idle_days,
            candidates,
            errors,
            worktree_inventory: inventory,
        })
    }

    /// Removes stale Git administrative records for registered projects. A
    /// live worktree, its files and its branch are never touched. Each
    /// project is pruned on its own, so one unreachable repository does not
    /// stop the others.
    pub async fn prune_worktrees(&self) -> ServiceResult<Pruned> {
        let projects = self
            .store
            .call(move |store| store.projects())
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || cleanup::prune(&projects))
            .await
            .context("The worktree prune stopped")
            .map_err(err)
    }
}
