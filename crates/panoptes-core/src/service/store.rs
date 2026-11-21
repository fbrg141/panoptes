use anyhow::Context;

use crate::store::Store;

/// The one SQLite handle shared by service calls and blocking Git work.
#[derive(Clone)]
pub(super) struct SharedStore(std::sync::Arc<std::sync::Mutex<Store>>);

impl SharedStore {
    pub(super) fn new(store: Store) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(store)))
    }

    /// SQLite calls block, so they never run on the async runtime.
    pub(super) async fn call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&Store) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let store = self.0.clone();
        tokio::task::spawn_blocking(move || call(&store.lock().unwrap()))
            .await
            .context("The store call stopped")?
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.0.lock().unwrap()
    }
}
