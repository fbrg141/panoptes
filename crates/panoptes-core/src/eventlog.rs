//! Panoptes' own record of what happened, for evaluating the app in daily
//! use: one JSON object per line in `<app data>/logs/YYYY-MM-DD.jsonl`
//! (UTC days).
//!
//! It records what Panoptes did and saw, never what was said: session and
//! lane changes, statuses, questions asked and answered, process exits and
//! the errors the UI was shown. Prompts, replies and file contents are never
//! written, and nothing leaves the machine. Files older than
//! [`KEEP_DAYS`] are removed when the log opens.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// How many days of files are kept.
pub const KEEP_DAYS: i64 = 30;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A cheap handle to the log; clones share one file. A disabled log (the
/// default, used by tests) records nothing.
#[derive(Clone, Default)]
pub struct EventLog {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    dir: PathBuf,
    /// The open file and the day it is for.
    file: Mutex<Option<(i64, File)>>,
}

impl EventLog {
    /// A log writing into `dir`, which is created if needed. Old files are
    /// pruned now. A folder that cannot be created leaves the log disabled:
    /// the app must not stop over its diagnostics.
    pub fn open(dir: &Path) -> Self {
        if std::fs::create_dir_all(dir).is_err() {
            return Self::default();
        }
        prune(dir, now_ms());
        Self {
            inner: Some(Arc::new(Inner {
                dir: dir.to_path_buf(),
                file: Mutex::new(None),
            })),
        }
    }

    /// Appends `{"at": <unix ms>, "event": <event>, ...fields}`. `fields`
    /// should be a JSON object; anything else is stored under `"data"`.
    /// Failures to write are ignored for the same reason as in [`open`].
    pub fn record(&self, event: &str, fields: Value) {
        self.record_at(now_ms(), event, fields);
    }

    fn record_at(&self, at: i64, event: &str, fields: Value) {
        let Some(inner) = &self.inner else {
            return;
        };
        let mut line = Map::new();
        line.insert("at".into(), json!(at));
        line.insert("event".into(), json!(event));
        match fields {
            Value::Object(fields) => line.extend(fields),
            Value::Null => {}
            other => {
                line.insert("data".into(), other);
            }
        }
        let mut bytes = serde_json::to_vec(&Value::Object(line)).expect("JSON always serializes");
        bytes.push(b'\n');

        let day = at.div_euclid(DAY_MS);
        let mut file = inner.file.lock().unwrap();
        if file.as_ref().is_none_or(|(open_day, _)| *open_day != day) {
            let path = inner.dir.join(format!("{}.jsonl", date(day)));
            *file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(|handle| (day, handle));
        }
        if let Some((_, handle)) = file.as_mut() {
            // One write per line, so lines from one process never interleave.
            let _ = handle.write_all(&bytes);
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD` for a count of days since 1970-01-01 (Howard Hinnant's
/// civil-from-days).
fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Removes day files older than [`KEEP_DAYS`]. Only names this log writes
/// are touched.
fn prune(dir: &Path, now: i64) {
    let today = now.div_euclid(DAY_MS);
    let oldest = date(today - KEEP_DAYS + 1);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".jsonl")) else {
            continue;
        };
        let is_day = stem.len() == 10
            && stem.bytes().enumerate().all(|(i, b)| {
                if i == 4 || i == 7 {
                    b == b'-'
                } else {
                    b.is_ascii_digit()
                }
            });
        // Day names sort as text in date order.
        if is_day && stem < oldest.as_str() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(dir: &Path, day: &str) -> Vec<Value> {
        std::fs::read_to_string(dir.join(format!("{day}.jsonl")))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn days_are_named_by_their_utc_date() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(19_723), "2024-01-01");
        assert_eq!(date(19_782), "2024-02-29");
        assert_eq!(date(20_405), "2025-11-13");
    }

    #[test]
    fn events_are_appended_one_line_each_into_their_day() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::open(dir.path());
        let day = 20_405 * DAY_MS;
        log.record_at(day + 5, "sessionStarted", json!({"key": "panoptes:1"}));
        log.record_at(
            day + 6,
            "status",
            json!({"key": "panoptes:1", "status": "running"}),
        );
        log.record_at(day + DAY_MS, "laneDeleted", json!({"lane": 3}));

        assert_eq!(
            lines(dir.path(), "2025-11-13"),
            [
                json!({"at": day + 5, "event": "sessionStarted", "key": "panoptes:1"}),
                json!({"at": day + 6, "event": "status", "key": "panoptes:1", "status": "running"}),
            ]
        );
        assert_eq!(lines(dir.path(), "2025-11-14").len(), 1);
    }

    #[test]
    fn a_disabled_log_writes_nothing() {
        EventLog::default().record("anything", json!({}));
    }

    #[test]
    fn only_old_day_files_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let today = 20_405;
        for name in [
            date(today - KEEP_DAYS) + ".jsonl",
            date(today - KEEP_DAYS + 1) + ".jsonl",
            "notes.jsonl".to_string(),
            "1999-01-01.txt".to_string(),
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        prune(dir.path(), today * DAY_MS);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "1999-01-01.txt".to_string(),
                date(today - KEEP_DAYS + 1) + ".jsonl",
                "notes.jsonl".to_string(),
            ]
        );
    }
}
