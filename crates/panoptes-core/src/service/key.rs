const PANOPTES_PREFIX: &str = "panoptes:";

/// The key of a Panoptes session. It is known before the harness starts, so
/// every event the session sends carries it.
pub fn panoptes_key(id: i64) -> String {
    format!("{PANOPTES_PREFIX}{id}")
}

/// The key of a harness conversation that no lane has adopted.
pub fn harness_key(harness: &str, id: &str) -> String {
    format!("{harness}:{id}")
}

pub(super) fn panoptes_id(key: &str) -> Option<i64> {
    key.strip_prefix(PANOPTES_PREFIX)?.parse().ok()
}

/// A session key parsed without letting a harness id get confused with a
/// Panoptes-owned numeric id.
pub(super) enum SessionKey<'a> {
    Panoptes(i64),
    Harness { id: &'a str },
}

impl<'a> SessionKey<'a> {
    pub(super) fn parse(harness: &str, key: &'a str) -> Self {
        match panoptes_id(key) {
            Some(id) => Self::Panoptes(id),
            None => Self::Harness {
                id: key.strip_prefix(&format!("{harness}:")).unwrap_or_default(),
            },
        }
    }
}
