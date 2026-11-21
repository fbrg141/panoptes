use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::{Connection, ErrorCode, OpenFlags, Transaction, TransactionBehavior};

use super::DATABASE;
use super::legacy::{IMPORTED_INDEX, LEGACY_INDEX};
use super::migrations::Migration;

pub(super) const IN_USE: &str = "Another Panoptes is using it. Quit that one and try again";

/// The database's schema version, after making sure it is a sound database
/// this build can read.
pub(super) fn check(conn: &Connection) -> anyhow::Result<u32> {
    let status: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| explain(error, "The file is not a readable database"))?;
    if status != "ok" {
        bail!("The database is damaged: {status}");
    }
    user_version(conn)
}

pub(super) fn user_version(conn: &Connection) -> anyhow::Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// `error` with `otherwise` as its reason, unless it only means another
/// Panoptes holds the database: that is not damage and must not read as it.
pub(super) fn explain(error: rusqlite::Error, otherwise: &'static str) -> anyhow::Error {
    let in_use = matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    );
    anyhow::Error::new(error).context(if in_use { IN_USE } else { otherwise })
}

fn refuse_newer(version: u32, latest: u32) -> anyhow::Result<()> {
    if version > latest {
        bail!(
            "It was written by a newer version of Panoptes (schema {version}; this version reads up to {latest})"
        );
    }
    Ok(())
}

pub(super) fn migrate(
    conn: &mut Connection,
    dir: &Path,
    version: u32,
    migrations: &[Migration],
) -> anyhow::Result<()> {
    let latest = migrations.len() as u32;
    refuse_newer(version, latest)?;
    if version == latest {
        return Ok(());
    }
    if version > 0 {
        back_up(conn, dir, version)?;
    }
    // Bundled SQLite enforces foreign keys by default, and a step that
    // rebuilds a table (create, copy, drop, rename) needs them off. The
    // pragma does nothing inside a transaction, so it is turned off here and
    // each step is checked before it commits instead.
    conn.pragma_update(None, "foreign_keys", false)?;
    loop {
        // IMMEDIATE takes the write lock up front and the version is read
        // again under it, so a second Panoptes opening the same store waits,
        // then skips the steps the first one ran.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| explain(error, "Could not start a migration"))?;
        let current = user_version(&tx)?;
        refuse_newer(current, latest)?;
        if current == latest {
            return Ok(());
        }
        if current == 0 && dir.join(IMPORTED_INDEX).exists() {
            bail!(
                "The database is empty, but Panoptes data was stored in it before ({IMPORTED_INDEX} is next to it). Rename {IMPORTED_INDEX} back to {LEGACY_INDEX} to import those pins again, or move it away to start without them"
            );
        }
        let target = current + 1;
        let step = migrations[current as usize];
        (step.apply)(&tx, dir)
            .and_then(|()| check_foreign_keys(&tx))
            .with_context(|| format!("Migration to schema {target} failed"))?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
        (step.committed)(dir);
    }
}

/// Fails if a step left a row pointing at one that does not exist, which it
/// could while foreign keys were off.
fn check_foreign_keys(tx: &Transaction) -> anyhow::Result<()> {
    let mut statement = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let parent: String = row.get(2)?;
        bail!("It left a row in {table} pointing to a missing row in {parent}");
    }
    Ok(())
}

/// Copies the database to `panoptes.db.v<version>.bak` before migrating away
/// from `version`. The copy is written under a temporary name and only then
/// replaces an older backup of the same version, so a failed copy never
/// costs the previous one. If another Panoptes migrated the store in the
/// meantime, the copy no longer holds `version` and is dropped: that one
/// took its own backup first.
fn back_up(conn: &Connection, dir: &Path, version: u32) -> anyhow::Result<()> {
    let backup = dir.join(format!("{DATABASE}.v{version}.bak"));
    let partial = partial_backup(dir);
    let result = copy_holds(conn, &partial, version).and_then(|holds| {
        if holds {
            std::fs::rename(&partial, &backup)?;
        }
        Ok(())
    });
    let _ = std::fs::remove_file(&partial);
    result.with_context(|| format!("Could not back up to {}", backup.display()))
}

/// Per process, so two copies being written at once don't collide.
pub(super) fn partial_backup(dir: &Path) -> PathBuf {
    dir.join(format!("{DATABASE}.{}.partial", std::process::id()))
}

/// Writes a copy of the database to `path` and says whether it holds
/// schema `version`.
fn copy_holds(conn: &Connection, path: &Path, version: u32) -> anyhow::Result<bool> {
    // VACUUM INTO refuses to write over an existing file.
    let _ = std::fs::remove_file(path);
    conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
    let copy = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(user_version(&copy)? == version)
}
