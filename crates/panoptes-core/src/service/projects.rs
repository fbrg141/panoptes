use std::path::Path;

use super::error::err;
use super::{Service, ServiceResult};
use crate::projects;
use crate::store::{Project, ProjectSettings};

impl Service {
    /// Registers the git repository `folder` is in, by its main checkout,
    /// with the default settings. Registering a repository twice fails.
    pub async fn add_project(&self, folder: String) -> ServiceResult<Project> {
        self.store
            .call(move |store| {
                let repo = projects::repository_of(Path::new(&folder))?;
                let settings = projects::validate(projects::default_settings(&repo))?;
                store.add_project(&repo.to_string_lossy(), &settings)
            })
            .await
            .map_err(err)
    }

    pub async fn update_project(
        &self,
        id: i64,
        settings: ProjectSettings,
    ) -> ServiceResult<Project> {
        self.store
            .call(move |store| store.update_project(id, &projects::validate(settings)?))
            .await
            .map_err(err)
    }

    /// Forgets a project. Nothing on disk changes, and its sessions are
    /// listed by folder again.
    pub async fn remove_project(&self, id: i64) -> ServiceResult<()> {
        self.store
            .call(move |store| store.remove_project(id))
            .await
            .map_err(err)
    }
}
