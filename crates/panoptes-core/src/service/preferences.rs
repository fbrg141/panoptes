use super::error::err;
use super::{Service, ServiceResult};
use crate::store::{Preferences, ProfileInput};

impl Service {
    pub async fn preferences(&self) -> ServiceResult<Preferences> {
        self.store
            .call(|store| store.preferences())
            .await
            .map_err(err)
    }

    pub async fn dismiss_permission_warning(&self) -> ServiceResult<Preferences> {
        self.store
            .call(|store| store.dismiss_permission_warning())
            .await
            .map_err(err)?;
        self.preferences().await
    }

    pub async fn save_profile(
        &self,
        id: Option<i64>,
        input: ProfileInput,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.save_profile(id, &input))
            .await?;
        self.preferences().await
    }

    pub async fn delete_profile(&self, id: i64) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.delete_profile(id))
            .await?;
        self.preferences().await
    }

    pub async fn set_model_favorite(
        &self,
        model_id: String,
        favorite: bool,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_model_favorite(&model_id, favorite))
            .await?;
        self.preferences().await
    }

    pub async fn set_model_hidden(
        &self,
        model_id: String,
        hidden: bool,
    ) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_model_hidden(&model_id, hidden))
            .await?;
        self.preferences().await
    }

    pub async fn set_cleanup_idle_days(&self, days: u32) -> ServiceResult<Preferences> {
        self.store
            .call(move |store| store.set_cleanup_idle_days(days))
            .await?;
        self.preferences().await
    }
}
