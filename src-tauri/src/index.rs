//! Panoptes' own small index of sessions: what the user has pinned, and
//! which project each session belongs to. Harnesses keep their sessions;
//! this file only holds what Panoptes adds on top, so new grouping rules
//! never need a data migration.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct IndexFile {
    /// Session keys (`<harness>:<id>`).
    #[serde(default)]
    pub pinned: BTreeSet<String>,
}

pub struct Index {
    path: PathBuf,
    pub file: IndexFile,
    project_roots: HashMap<String, String>,
}

impl Index {
    pub fn load(path: PathBuf) -> Self {
        let file = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, file, project_roots: HashMap::new() }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.file)?)?;
        std::fs::rename(temp, &self.path)?;
        Ok(())
    }

    /// The project a working directory belongs to: the enclosing git
    /// repository (or worktree) root, else the directory itself.
    pub fn project_root(&mut self, cwd: &str) -> String {
        if let Some(root) = self.project_roots.get(cwd) {
            return root.clone();
        }
        let root = Path::new(cwd)
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd.to_string());
        self.project_roots.insert(cwd.to_string(), root.clone());
        root
    }
}
