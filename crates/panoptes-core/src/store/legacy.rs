use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// The JSON file that held pins before the store existed.
pub(super) const LEGACY_INDEX: &str = "index.json";

/// What `index.json` is renamed to once its pins are in the database.
pub(super) const IMPORTED_INDEX: &str = "index.json.migrated";

/// Pins from `index.json`, if there is one. An unreadable file fails the
/// migration rather than losing the pins.
pub(super) fn legacy_pins(dir: &Path) -> anyhow::Result<BTreeSet<String>> {
    #[derive(Deserialize)]
    struct IndexFile {
        #[serde(default)]
        pinned: BTreeSet<String>,
    }
    let path = dir.join(LEGACY_INDEX);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", path.display()));
        }
    };
    let file: IndexFile = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} could not be read, so its pins were not imported. Move it away to start without them",
            path.display()
        )
    })?;
    Ok(file.pinned)
}

/// The pins are in the database now; keep the old file for reference under
/// a name that is never imported again.
pub(super) fn retire_legacy_index(dir: &Path) {
    let legacy = dir.join(LEGACY_INDEX);
    match std::fs::rename(&legacy, dir.join(IMPORTED_INDEX)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("Could not rename {}: {error}", legacy.display()),
    }
}
