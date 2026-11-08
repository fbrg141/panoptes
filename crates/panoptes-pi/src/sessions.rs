//! Listing Pi sessions by reading its session folder.
//!
//! Pi's RPC mode has no command that lists sessions, so this reads the
//! documented layout directly: `<sessions>/--<encoded cwd>--/<ts>_<id>.jsonl`,
//! a header line, then one entry per line. Results are cached per file by
//! size and modification time, so only changed sessions are re-read.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use panoptes_harness::{DiscoveredSession, HarnessRef};
use serde_json::Value;

use crate::map::user_text;

#[derive(Default)]
pub struct SessionCache {
    entries: HashMap<PathBuf, (u64, SystemTime, DiscoveredSession)>,
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl SessionCache {
    pub fn scan(&mut self, root: &Path) -> Vec<DiscoveredSession> {
        let mut seen = HashSet::new();
        let Ok(project_dirs) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        for project_dir in project_dirs.flatten() {
            let Ok(files) = std::fs::read_dir(project_dir.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_none_or(|ext| ext != "jsonl") {
                    continue;
                }
                let Ok(meta) = file.metadata() else { continue };
                let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                let fresh = self
                    .entries
                    .get(&path)
                    .is_some_and(|(size, mtime, _)| *size == meta.len() && *mtime == modified);
                if !fresh {
                    match read_summary(&path, &meta) {
                        Some(summary) => {
                            self.entries
                                .insert(path.clone(), (meta.len(), modified, summary));
                        }
                        None => {
                            self.entries.remove(&path);
                            continue;
                        }
                    }
                }
                seen.insert(path);
            }
        }
        self.entries.retain(|path, _| seen.contains(path));
        self.entries
            .values()
            .map(|(_, _, summary)| summary.clone())
            .collect()
    }
}

fn read_summary(path: &Path, meta: &std::fs::Metadata) -> Option<DiscoveredSession> {
    let mut lines = BufReader::new(File::open(path).ok()?).split(b'\n');
    let header: Value = serde_json::from_slice(&lines.next()?.ok()?).ok()?;
    if header.get("type")?.as_str()? != "session" {
        return None;
    }

    let mut name: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut message_count = 0u32;
    for line in lines.map_while(Result::ok) {
        // Pi writes compact JSON, so the entry type is always the first key.
        // Only the few lines that matter are parsed.
        if line.starts_with(br#"{"type":"message""#) {
            message_count += 1;
            if first_user.is_none() && contains(&line, br#""role":"user""#) {
                first_user = serde_json::from_slice::<Value>(&line)
                    .ok()
                    .and_then(|entry| user_text(&entry["message"]["content"]));
            }
        } else if line.starts_with(br#"{"type":"session_info""#)
            && let Ok(entry) = serde_json::from_slice::<Value>(&line)
        {
            // An absent name means it was cleared.
            name = entry["name"].as_str().map(str::to_string);
        }
    }

    let title = name
        .or(first_user.map(|text| first_line(&text)))
        .unwrap_or_else(|| "New session".to_string());
    Some(DiscoveredSession {
        harness_ref: HarnessRef {
            harness: "pi".to_string(),
            id: header["id"].as_str()?.to_string(),
            locator: path.to_string_lossy().into_owned(),
            cwd: header["cwd"].as_str().unwrap_or_default().to_string(),
        },
        title,
        created_at: millis(meta.created().unwrap_or(UNIX_EPOCH)),
        updated_at: millis(meta.modified().unwrap_or(UNIX_EPOCH)),
        message_count,
        parent_locator: header["parentSession"].as_str().map(str::to_string),
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/code/app"}"#;

    fn write_session(root: &Path, name: &str, lines: &[&str]) -> PathBuf {
        let dir = root.join("--code-app--");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn summarises_a_session_from_its_first_user_message() {
        let root = tempfile::tempdir().unwrap();
        write_session(
            root.path(),
            "a.jsonl",
            &[
                HEADER,
                r#"{"type":"message","id":"a","parentId":null,"message":{"role":"system","content":""}}"#,
                r#"{"type":"message","id":"b","parentId":"a","message":{"role":"user","content":"\n  Fix the login bug\nand more"}}"#,
                r#"{"type":"message","id":"c","parentId":"b","message":{"role":"assistant","content":[]}}"#,
            ],
        );
        let sessions = SessionCache::default().scan(root.path());
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.harness_ref.id, "s1");
        assert_eq!(session.harness_ref.cwd, "/code/app");
        assert_eq!(session.title, "Fix the login bug");
        assert_eq!(session.message_count, 3);
    }

    #[test]
    fn the_latest_session_name_wins_over_the_first_message() {
        let root = tempfile::tempdir().unwrap();
        write_session(
            root.path(),
            "a.jsonl",
            &[
                HEADER,
                r#"{"type":"message","id":"b","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#,
                r#"{"type":"session_info","id":"c","parentId":"b","name":"First name"}"#,
                r#"{"type":"session_info","id":"d","parentId":"c","name":"Auth refactor"}"#,
            ],
        );
        assert_eq!(
            SessionCache::default().scan(root.path())[0].title,
            "Auth refactor"
        );
    }

    #[test]
    fn skips_files_that_are_not_sessions() {
        let root = tempfile::tempdir().unwrap();
        write_session(root.path(), "notes.txt", &[HEADER]);
        write_session(root.path(), "broken.jsonl", &["not json"]);
        write_session(root.path(), "other.jsonl", &[r#"{"type":"message"}"#]);
        assert!(SessionCache::default().scan(root.path()).is_empty());
    }

    #[test]
    fn rescans_changed_files_and_forgets_deleted_ones() {
        let root = tempfile::tempdir().unwrap();
        let mut cache = SessionCache::default();
        let path = write_session(root.path(), "a.jsonl", &[HEADER]);
        assert_eq!(cache.scan(root.path())[0].title, "New session");

        write_session(
            root.path(),
            "a.jsonl",
            &[
                HEADER,
                r#"{"type":"session_info","id":"x","parentId":null,"name":"Renamed"}"#,
            ],
        );
        assert_eq!(cache.scan(root.path())[0].title, "Renamed");

        std::fs::remove_file(path).unwrap();
        assert!(cache.scan(root.path()).is_empty());
    }

    #[test]
    fn a_missing_root_lists_nothing() {
        assert!(
            SessionCache::default()
                .scan(Path::new("/nonexistent/panoptes"))
                .is_empty()
        );
    }
}
