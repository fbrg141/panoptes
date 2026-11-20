//! Small text formats shared by the views. Ports of `formatTokens`
//! (`composer.ts`) and `relativeTime` (`grouping.ts`).

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
}
