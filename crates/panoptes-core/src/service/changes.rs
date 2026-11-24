use std::path::Path;

use anyhow::Context;

use super::error::err;
use super::{Service, ServiceResult};
use crate::changes::{
    self, ChangeTotals, DiffOptions, Hunk, LaneChanges, TreeRange, WorktreeChanges,
};

impl Service {
    /// Every worktree's complete diff against its own merge-base, without
    /// patch text. Paths and object ids are meaningful only inside their
    /// worktree entry.
    pub async fn lane_changes(&self, lane_id: i64) -> ServiceResult<LaneChanges> {
        let (lane, projects) = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                let projects = lane
                    .worktrees
                    .iter()
                    .map(|worktree| store.project(worktree.project_id))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok((lane, projects))
            })
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || {
            let mut worktrees = Vec::with_capacity(projects.len());
            let mut totals = ChangeTotals::default();
            for project in projects {
                let range = changes::lane_range(&project, &lane)?;
                let changes =
                    changes::diff(Path::new(&project.repo), &range, DiffOptions::default())?;
                let base = changes::base_review(
                    Path::new(&project.repo),
                    &project.settings.base_branch,
                    &range,
                );
                totals.add(changes.totals);
                worktrees.push(WorktreeChanges {
                    project_id: project.id,
                    project_name: project.settings.name.clone(),
                    base_branch: project.settings.base_branch.clone(),
                    base,
                    changes,
                });
            }
            Ok(LaneChanges {
                lane_id,
                worktrees,
                totals,
            })
        })
        .await
        .context("The lane diff stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Patch hunks for one file in the range the lane listing returned. The
    /// project id selects the worktree explicitly, so equal paths in two
    /// repositories can never resolve against the wrong object ids.
    pub async fn file_hunks(
        &self,
        lane_id: i64,
        project_id: i64,
        path: String,
        range: TreeRange,
    ) -> ServiceResult<Vec<Hunk>> {
        let repo = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                if !lane
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.project_id == project_id)
                {
                    anyhow::bail!("That project has no worktree in this lane");
                }
                Ok(store.project(project_id)?.repo)
            })
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || {
            changes::hunks(Path::new(&repo), &range, &path, DiffOptions::default())
        })
        .await
        .context("The file diff stopped")
        .and_then(|result| result)
        .map_err(err)
    }
}
