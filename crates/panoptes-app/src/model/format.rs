//! Small text formats shared by the views.

use chrono::{Datelike as _, TimeZone as _};

/// "950", "1.3k", "42k", "1.2M".
pub fn format_tokens(n: u64) -> String {
    fn short(value: f64, unit: &str) -> String {
        let digits = if value < 10.0 { 1 } else { 0 };
        let rounded = format!("{value:.digits$}");
        // Drop a trailing ".0", as parseFloat does.
        let trimmed = rounded.strip_suffix(".0").unwrap_or(&rounded);
        format!("{trimmed}{unit}")
    }
    if n < 1000 {
        n.to_string()
    } else if n < 1_000_000 {
        short(n as f64 / 1000.0, "k")
    } else {
        short(n as f64 / 1_000_000.0, "M")
    }
}

/// "just now", "12m ago", "3h ago", "3d ago", then a date.
pub fn relative_time(ms: i64, now: i64) -> String {
    let minutes = ((now - ms) as f64 / 60_000.0).round() as i64;
    if minutes < 1 {
        return "just now".into();
    }
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = (hours as f64 / 24.0).round() as i64;
    if days < 30 {
        return format!("{days}d ago");
    }
    local_date(ms)
}

/// A short date in the local time zone, as `toLocaleDateString` gives it
/// in an English locale.
pub fn local_date(ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) => format!("{}/{}/{}", t.month(), t.day(), t.year()),
        None => String::new(),
    }
}

/// "14:05": a time of day in the local time zone, for turn footers.
pub fn local_time(ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// Unix milliseconds now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

const KIB: f64 = 1024.0;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Binary units, matching the allocated blocks reported by `du -sk`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= KIB && unit < UNITS.len() - 1 {
        value /= KIB;
        unit += 1;
    }
    if unit > 0 && value < 10.0 && value.fract() != 0.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// "2 worktrees · 2 KB".
pub fn usage_label(worktrees: u64, bytes: u64) -> String {
    let noun = if worktrees == 1 {
        "worktree"
    } else {
        "worktrees"
    };
    format!("{worktrees} {noun} \u{b7} {}", format_bytes(bytes))
}

/// "4 days idle", in whole days.
pub fn idle_label(last_activity: i64, now: i64) -> String {
    let days = ((now - last_activity) / DAY_MS).max(0);
    format!("{days} {} idle", if days == 1 { "day" } else { "days" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_read_short() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(1_300), "1.3k");
        assert_eq!(format_tokens(10_000), "10k");
        assert_eq!(format_tokens(42_400), "42k");
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }

    #[test]
    fn relative_time_speaks_in_the_design_systems_voice() {
        let now = 10 * 86_400_000;
        assert_eq!(relative_time(now - 10_000, now), "just now");
        assert_eq!(relative_time(now - 12 * 60_000, now), "12m ago");
        assert_eq!(relative_time(now - 3 * 86_400_000, now), "3d ago");
        assert!(local_date(0).ends_with("/1970") || local_date(0).ends_with("/1969"));
        assert_eq!(local_time(0).len(), 5);
    }

    #[test]
    fn bytes_are_formatted_without_false_precision() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(27 * 1024u64.pow(3)), "27 GB");
        assert_eq!(format_bytes(2 * 1024u64.pow(4)), "2 TB");
    }

    #[test]
    fn worktree_counts_and_whole_idle_days_are_labelled() {
        assert_eq!(usage_label(1, 1024), "1 worktree \u{b7} 1 KB");
        assert_eq!(usage_label(2, 2048), "2 worktrees \u{b7} 2 KB");
        let now = 10 * DAY_MS;
        assert_eq!(idle_label(now - 4 * DAY_MS, now), "4 days idle");
        assert_eq!(idle_label(now - DAY_MS, now), "1 day idle");
    }
}
