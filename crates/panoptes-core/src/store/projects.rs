use anyhow::{Context, bail};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use super::{Store, now_ms};

/// A registered git repository and the settings every new worktree of it
/// is made with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    /// Path of the main checkout.
    pub repo: String,
    #[serde(flatten)]
    pub settings: ProjectSettings,
}

/// What can be changed about a project after it is registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    pub name: String,
    /// What new worktrees branch from.
    pub base_branch: String,
    /// Untracked files copied from the main checkout into new worktrees,
    /// relative to it.
    pub copy_files: Vec<String>,
    /// Run in a new worktree once it is made, such as `pnpm install`.
    pub setup: Option<String>,
}

impl Store {
    /// Every project, by name.
    pub(crate) fn projects(&self) -> anyhow::Result<Vec<Project>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, repo, name, base_branch, setup FROM projects ORDER BY name COLLATE NOCASE, id",
        )?;
        let mut projects = statement
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    repo: row.get(1)?,
                    settings: ProjectSettings {
                        name: row.get(2)?,
                        base_branch: row.get(3)?,
                        setup: row.get(4)?,
                        copy_files: Vec::new(),
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut files = self.conn.prepare_cached(
            "SELECT project_id, path FROM project_copy_files ORDER BY project_id, position",
        )?;
        let files = files.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get(1)?)))?;
        for file in files {
            let (id, path) = file?;
            if let Some(project) = projects.iter_mut().find(|p| p.id == id) {
                project.settings.copy_files.push(path);
            }
        }
        Ok(projects)
    }

    /// Registers the repository whose main checkout is `repo`. Fails if it is
    /// already registered.
    pub(crate) fn add_project(
        &self,
        repo: &str,
        settings: &ProjectSettings,
    ) -> anyhow::Result<Project> {
        let tx = self.write()?;
        let existing: Option<String> = tx
            .query_row("SELECT name FROM projects WHERE repo = ?1", [repo], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(name) = existing {
            bail!("{repo} is already registered as {name}");
        }
        tx.execute(
            "INSERT INTO projects (name, repo, base_branch, setup, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                settings.name,
                repo,
                settings.base_branch,
                settings.setup,
                now_ms()
            ],
        )?;
        let id = tx.last_insert_rowid();
        write_copy_files(&tx, id, &settings.copy_files)?;
        tx.commit()?;
        Ok(Project {
            id,
            repo: repo.to_string(),
            settings: settings.clone(),
        })
    }

    pub(crate) fn update_project(
        &self,
        id: i64,
        settings: &ProjectSettings,
    ) -> anyhow::Result<Project> {
        let tx = self.write()?;
        let repo: Option<String> = tx
            .query_row(
                "UPDATE projects SET name = ?2, base_branch = ?3, setup = ?4 WHERE id = ?1
                 RETURNING repo",
                rusqlite::params![id, settings.name, settings.base_branch, settings.setup],
                |row| row.get(0),
            )
            .optional()?;
        let Some(repo) = repo else {
            bail!("That project is no longer registered");
        };
        tx.execute("DELETE FROM project_copy_files WHERE project_id = ?1", [id])?;
        write_copy_files(&tx, id, &settings.copy_files)?;
        tx.commit()?;
        Ok(Project {
            id,
            repo,
            settings: settings.clone(),
        })
    }

    pub(crate) fn project(&self, id: i64) -> anyhow::Result<Project> {
        self.projects()?
            .into_iter()
            .find(|project| project.id == id)
            .context("That project is no longer registered")
    }

    pub(crate) fn remove_project(&self, id: i64) -> anyhow::Result<()> {
        let tx = self.write()?;
        let lanes: i64 = tx.query_row(
            "SELECT count(*) FROM lanes WHERE project_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if lanes > 0 {
            bail!("That project still has lanes and cannot be removed");
        }
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}

fn write_copy_files(tx: &Transaction, id: i64, files: &[String]) -> anyhow::Result<()> {
    let mut insert = tx.prepare_cached(
        "INSERT INTO project_copy_files (project_id, position, path) VALUES (?1, ?2, ?3)",
    )?;
    for (position, path) in files.iter().enumerate() {
        insert.execute(rusqlite::params![id, position as i64, path])?;
    }
    Ok(())
}
