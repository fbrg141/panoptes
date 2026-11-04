//! Listing Pi sessions by reading its session folder.
//!
//! Pi's RPC mode has no command that lists sessions, so this reads the
//! documented layout directly: `<sessions>/--<encoded cwd>--/<ts>_<id>.jsonl`,
//! a header line, then one entry per line. Results are cached per file by
//! size and modification time, so only changed sessions are re-read.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use panoptes_harness::SessionSummary;
use serde_json::Value;

use crate::map::user_text;

#[derive(Default)]
pub struct SessionCache {
    entries: HashMap<PathBuf, (u64, SystemTime, SessionSummary)>,
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl SessionCache {
    pub fn scan(&mut self, root: &Path) -> Vec<SessionSummary> {
        let mut seen = Vec::new();
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
                            self.entries.insert(path.clone(), (meta.len(), modified, summary));
                        }
                        None => {
                            self.entries.remove(&path);
                            continue;
                        }
                    }
                }
                seen.push(path);
            }
        }
        self.entries.retain(|path, _| seen.contains(path));
        self.entries.values().map(|(_, _, summary)| summary.clone()).collect()
    }
}

fn read_summary(path: &Path, meta: &std::fs::Metadata) -> Option<SessionSummary> {
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
        } else if line.starts_with(br#"{"type":"session_info""#) {
            if let Ok(entry) = serde_json::from_slice::<Value>(&line) {
                name = entry["name"].as_str().map(str::to_string);
            }
        }
    }

    let title = name
        .or(first_user.map(|text| first_line(&text)))
        .unwrap_or_else(|| "New session".to_string());
    Some(SessionSummary {
        harness: "pi".to_string(),
        id: header["id"].as_str()?.to_string(),
        locator: path.to_string_lossy().into_owned(),
        cwd: header["cwd"].as_str().unwrap_or_default().to_string(),
        title,
        created_at: millis(meta.created().unwrap_or(UNIX_EPOCH)),
        updated_at: millis(meta.modified().unwrap_or(UNIX_EPOCH)),
        message_count,
        parent_locator: header["parentSession"].as_str().map(str::to_string),
    })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}
