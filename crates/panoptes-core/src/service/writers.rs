use std::collections::{HashMap, HashSet};

/// Lanes other than their own that live sessions have edited. This is a
/// cross-lane write guard, not durable history.
#[derive(Clone, Default)]
pub(super) struct LaneWriters(std::sync::Arc<std::sync::Mutex<HashMap<i64, HashSet<String>>>>);

impl LaneWriters {
    /// Records a write. Returns true only for the session's first write here.
    pub(super) fn record(&self, lane_id: i64, key: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .entry(lane_id)
            .or_default()
            .insert(key.to_string())
    }

    pub(super) fn forget(&self, key: &str) {
        let mut writers = self.0.lock().unwrap();
        for keys in writers.values_mut() {
            keys.remove(key);
        }
        writers.retain(|_, keys| !keys.is_empty());
    }

    /// The first other-lane writer, in key order, for which `live` is true.
    pub(super) fn busy_writer(&self, lane_id: i64, live: impl Fn(&str) -> bool) -> Option<String> {
        let writers = self.0.lock().unwrap();
        let mut keys: Vec<&String> = writers.get(&lane_id)?.iter().collect();
        keys.sort();
        keys.into_iter().find(|key| live(key)).cloned()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }
}
