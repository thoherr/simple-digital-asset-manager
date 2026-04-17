//! CLI output helpers: human-readable formatting and common progress/status messages.
//!
//! Centralizes patterns previously duplicated across `main.rs`, `model_manager.rs`,
//! `web/templates.rs`, and `web/routes.rs`.

use std::time::Duration;

/// Format a Duration as `h m s.ms`, dropping leading zero fields.
pub fn format_duration(d: Duration) -> String {
    let total_millis = d.as_millis();
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}.{millis:03}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}.{millis:03}s")
    } else {
        format!("{seconds}.{millis:03}s")
    }
}

/// Format a byte count as a human-readable string (B / KB / MB / GB).
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Print an indented per-item status line to stderr.
///
/// With duration:    `  {id} — {verb} ({duration})`
/// Without duration: `  {id} — {verb}`
///
/// Consolidates the dominant status-line pattern used across import, cleanup,
/// refresh, fix-* and similar commands.
pub fn item_status(id: &str, verb: &str, elapsed: Option<Duration>) {
    match elapsed {
        Some(d) => eprintln!("  {id} — {verb} ({})", format_duration(d)),
        None => eprintln!("  {id} — {verb}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_sub_minute() {
        assert_eq!(format_duration(Duration::from_millis(1234)), "1.234s");
        assert_eq!(format_duration(Duration::from_millis(0)), "0.000s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(Duration::from_secs(75)), "1m 15.000s");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(
            format_duration(Duration::from_secs(3725)),
            "1h 02m 05.000s"
        );
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1500), "1.5 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(94 * 1024 * 1024), "94.0 MB");
    }

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
}
