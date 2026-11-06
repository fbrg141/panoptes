//! Panoptes' own data, in a SQLite database in the app data folder. Harnesses
//! keep their conversations; this holds what Panoptes adds on top (pins now,
//! then projects, groups, workspaces, sessions and tags, see
//! `docs/domain-model.md`).
//!
//! The schema version is SQLite's `user_version`. Opening the store runs
//! every migration it is missing, each in its own transaction, after copying
//! the database aside. A damaged database, one written by a newer Panoptes,
//! or a migration that fails stops the open with an error: the store never
//! starts empty in place of data it could not read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::{Connection, Transaction};
use serde::Deserialize;

const DATABASE: &str = "panoptes.db";
/// The JSON file that held pins before the store existed.
const LEGACY_INDEX: &str = "index.json";

/// One schema step. `dir` is the app data folder, for steps that import
/// files from it.
type Migration = fn(&Transaction, &Path) -> anyhow::Result<()>;

/// Schema version `n` is reached by running `MIGRATIONS[n - 1]`. Append new
/// steps; never edit or reorder one that has shipped.
const MIGRATIONS: &[Migration] = &[v1_pins];

fn v1_pins(tx: &Transaction, dir: &Path) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE pins (
            session_key TEXT PRIMARY KEY NOT NULL
        ) STRICT;",
    )?;
    for key in legacy_pins(dir)? {
        tx.execute(
            "INSERT OR IGNORE INTO pins (session_key) VALUES (?1)",
            [key],
        )?;
    }
    Ok(())
}

/// Pins from `index.json`, if there is one. An unreadable file fails the
/// migration rather than losing the pins.
fn legacy_pins(dir: &Path) -> anyhow::Result<BTreeSet<String>> {
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

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the store in `dir`, creating it or migrating it to the current
    /// schema as needed.
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        Self::open_with(dir, MIGRATIONS)
    }

    fn open_with(dir: &Path, migrations: &[Migration]) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Could not create {}", dir.display()))?;
        let path = dir.join(DATABASE);
        let mut conn = Connection::open(&path)
            .with_context(|| format!("Could not open {}", path.display()))?;
        let result = check(&conn).and_then(|version| migrate(&mut conn, dir, version, migrations));
        result.with_context(|| format!("Could not load Panoptes data from {}", path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    pub fn pinned(&self) -> anyhow::Result<BTreeSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT session_key FROM pins")?;
        let keys = statement.query_map([], |row| row.get(0))?;
        Ok(keys.collect::<Result<_, _>>()?)
    }

    pub fn set_pinned(&self, key: &str, pinned: bool) -> anyhow::Result<()> {
        let sql = if pinned {
            "INSERT OR IGNORE INTO pins (session_key) VALUES (?1)"
        } else {
            "DELETE FROM pins WHERE session_key = ?1"
        };
        self.conn.execute(sql, [key])?;
        Ok(())
    }
}

/// The database's schema version, after making sure it is a sound database
/// this build can read.
fn check(conn: &Connection) -> anyhow::Result<u32> {
    let status: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .context("The file is not a readable database")?;
    if status != "ok" {
        bail!("The database is damaged: {status}");
    }
    let version: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

fn migrate(
    conn: &mut Connection,
    dir: &Path,
    version: u32,
    migrations: &[Migration],
) -> anyhow::Result<()> {
    let latest = migrations.len() as u32;
    if version > latest {
        bail!(
            "It was written by a newer version of Panoptes (schema {version}; this version reads up to {latest})"
        );
    }
    if version == latest {
        return Ok(());
    }
    if version > 0 {
        let backup = backup_path(dir, version);
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])
            .with_context(|| format!("Could not back up to {}", backup.display()))?;
    }
    for (index, step) in migrations.iter().enumerate().skip(version as usize) {
        let target = index as u32 + 1;
        let tx = conn.transaction()?;
        step(&tx, dir).with_context(|| format!("Migration to schema {target} failed"))?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
    }
    // The pins are in the database now; keep the old file for reference
    // under a name that is never read again.
    let legacy = dir.join(LEGACY_INDEX);
    if version == 0 && legacy.exists() {
        let _ = std::fs::rename(&legacy, legacy.with_extension("json.migrated"));
    }
    Ok(())
}

/// Where the copy taken before migrating away from `version` goes. An older
/// backup of the same version is replaced.
fn backup_path(dir: &Path, version: u32) -> PathBuf {
    let path = dir.join(format!("{DATABASE}.v{version}.bak"));
    let _ = std::fs::remove_file(&path);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(dir: &Path) -> u32 {
        let conn = Connection::open(dir.join(DATABASE)).unwrap();
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn broken(_: &Transaction, _: &Path) -> anyhow::Result<()> {
        bail!("no room")
    }

    fn v2_notes(tx: &Transaction, _: &Path) -> anyhow::Result<()> {
        tx.execute_batch("CREATE TABLE notes (body TEXT NOT NULL) STRICT;")?;
        Ok(())
    }

    #[test]
    fn a_new_store_starts_at_the_latest_schema() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(store.pinned().unwrap().is_empty());
        assert_eq!(version(dir.path()), MIGRATIONS.len() as u32);
    }

    #[test]
    fn pins_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.set_pinned("pi:a", true).unwrap();
        store.set_pinned("pi:b", true).unwrap();
        store.set_pinned("pi:b", true).unwrap();
        store.set_pinned("pi:a", false).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
    }

    #[test]
    fn pins_in_index_json_are_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_INDEX);
        std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:b"]}"#).unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.pinned().unwrap(),
            BTreeSet::from(["pi:a".into(), "pi:b".into()])
        );
        assert!(!legacy.exists());
        assert!(dir.path().join("index.json.migrated").exists());

        // Unpinning sticks, and a stray index.json is not imported again.
        store.set_pinned("pi:a", false).unwrap();
        drop(store);
        std::fs::write(&legacy, r#"{"pinned":["pi:a","pi:c"]}"#).unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:b".into()]));
    }

    #[test]
    fn an_unreadable_index_json_stops_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_INDEX);
        std::fs::write(&legacy, "{not json").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("Migration to schema 1 failed"), "{error}");
        assert!(error.contains("its pins were not imported"), "{error}");
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "{not json");
        assert_eq!(version(dir.path()), 0);
    }

    #[test]
    fn a_failed_migration_stops_and_keeps_the_data() {
        let dir = tempfile::tempdir().unwrap();
        Store::open(dir.path())
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();

        let error = Store::open_with(dir.path(), &[v1_pins, broken])
            .err()
            .unwrap();
        let error = format!("{error:#}");
        assert!(
            error.contains("Migration to schema 2 failed: no room"),
            "{error}"
        );
        assert_eq!(version(dir.path()), 1);

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));
    }

    #[test]
    fn migrating_backs_up_the_database_first() {
        let dir = tempfile::tempdir().unwrap();
        Store::open(dir.path())
            .unwrap()
            .set_pinned("pi:a", true)
            .unwrap();

        let store = Store::open_with(dir.path(), &[v1_pins, v2_notes]).unwrap();
        assert_eq!(version(dir.path()), 2);
        assert_eq!(store.pinned().unwrap(), BTreeSet::from(["pi:a".into()]));

        let backup = Connection::open(dir.path().join("panoptes.db.v1.bak")).unwrap();
        let backed_up: Vec<String> = backup
            .prepare("SELECT session_key FROM pins")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(backed_up, ["pi:a"]);
        let backup_version: u32 = backup
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_version, 1);
    }

    #[test]
    fn a_store_from_a_newer_panoptes_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        Store::open_with(dir.path(), &[v1_pins, v2_notes]).unwrap();
        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("newer version of Panoptes"), "{error}");
        assert_eq!(version(dir.path()), 2);
    }

    #[test]
    fn a_damaged_database_is_refused_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DATABASE);
        std::fs::write(&path, b"this is not a database at all, not even close").unwrap();

        let error = format!("{:#}", Store::open(dir.path()).err().unwrap());
        assert!(error.contains("not a readable database"), "{error}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"this is not a database at all, not even close"
        );
    }
}
