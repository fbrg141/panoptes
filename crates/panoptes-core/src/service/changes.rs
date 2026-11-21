use std::path::Path;

use anyhow::Context;

use super::error::err;
use super::{Service, ServiceResult};
use crate::changes::{self, Changes, DiffOptions, Hunk, TreeRange};

impl Service {
    /// The lane's complete diff against its merge-base, without patch text.
    pub async fn lane_changes(&self, lane_id: i64) -> ServiceResult<Changes> {
        let (project, lane) = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                let project = store.project(lane.project_id)?;
                Ok((project, lane))
            })
            .await
            .map_err(err)?;
        tokio::task::spawn_blocking(move || {
            let range = changes::lane_range(&project, &lane)?;
            changes::diff(Path::new(&project.repo), &range, DiffOptions::default())
        })
        .await
        .context("The lane diff stopped")
        .and_then(|result| result)
        .map_err(err)
    }

    /// Patch hunks for one file in the range the lane list returned.
    pub async fn file_hunks(
        &self,
        lane_id: i64,
        path: String,
        range: TreeRange,
    ) -> ServiceResult<Vec<Hunk>> {
        let repo = self
            .store
            .call(move |store| {
                let lane = store.lane(lane_id)?;
                Ok(store.project(lane.project_id)?.repo)
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
