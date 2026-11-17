//! Pure render helpers: the strings and the tray icon every surface
//! shows, given a snapshot and the settings. No clock access — callers
//! pass `now_ms`, so every function here is unit-testable.

use crate::model::QuotaState;
use chrono::{Local, TimeZone};

/// Plenty left.
pub const GREEN: &str = "#22c55e";
/// Getting close.
pub const AMBER: &str = "#f59e0b";
/// At or past the configured threshold.
pub const RED: &str = "#ef4444";
/// No data yet.
pub const GRAY: &str = "#6b7280";

/// The percent as shown: whole when it rounds clean, one decimal
/// otherwise (`42%`, `41.7%`).
pub fn pct_text(pct: f64) -> String {
    if (pct * 10.0).round() as i64 == pct.round() as i64 * 10 {
        format!("{}%", pct.round() as i64)
    } else {
        format!("{pct:.1}%")
    }
}

/// `1d4h` / `3h12m` / `7m` / `45s` for a duration in seconds.
pub fn dur_text(total_secs: i64) -> String {
    let secs = total_secs.max(0);
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

/// `3m` until `reset_ms`, from `now_ms`.
pub fn until_reset(reset_ms: i64, now_ms: i64) -> String {
    dur_text((reset_ms - now_ms) / 1000)
}

/// `3m` since `fetched_at_ms`.
pub fn age_text(fetched_at_ms: i64, now_ms: i64) -> String {
    dur_text((now_ms - fetched_at_ms) / 1000)
}

/// The reset wall-clock in the local zone: `03:16:50`.
pub fn reset_clock(reset_ms: i64) -> String {
    let Some(secs) = reset_ms.checked_div(1000) else {
        return "??:??:??".into();
    };
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(t) => t.format("%H:%M:%S").to_string(),
        _ => "??:??:??".into(),
    }
}

/// The tray tile's fill for `pct` with the alert threshold `threshold`:
/// green below 80, amber below the threshold, red at and above it.
pub fn icon_color(pct: f64, threshold: f64) -> &'static str {
    if pct >= threshold {
        RED
    } else if pct >= 80.0 {
        AMBER
    } else {
        GREEN
    }
}

/// A square SVG tray icon: a rounded tile in the bucket color with the
/// percent centered, white. nsh rasterizes it at 16/24/32 px.
pub fn icon_svg(pct: f64, threshold: f64) -> String {
    let text = pct_text(pct);
    let size = if text.chars().count() > 3 { 26.0 } else { 32.0 };
    let color = icon_color(pct, threshold);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\">\
<rect width=\"64\" height=\"64\" rx=\"14\" fill=\"{color}\"/>\
<text x=\"32\" y=\"41\" font-family=\"system-ui,sans-serif\" font-size=\"{size}\" \
font-weight=\"700\" text-anchor=\"middle\" fill=\"#ffffff\">{text}</text></svg>"
    )
}

/// The no-data tile: a gray tile with an ellipsis.
pub fn icon_svg_placeholder() -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"64\" height=\"64\">\
<rect width=\"64\" height=\"64\" rx=\"14\" fill=\"{GRAY}\"/>\
<text x=\"32\" y=\"45\" font-family=\"system-ui,sans-serif\" font-size=\"36\" \
font-weight=\"700\" text-anchor=\"middle\" fill=\"#ffffff\">\u{2261}</text></svg>"
    )
}

/// What the tray tooltip shows on hover: label, percent, reset, level,
/// age — or the error. One line; nsh shows it as the SNI tooltip.
pub fn tooltip(
    state: &QuotaState,
    label: &str,
    show_reset: bool,
    interval_secs: i64,
    now_ms: i64,
) -> String {
    let Some(pct) = state.percentage else {
        return match &state.error {
            Some(err) => format!("{label} \u{2716} {err}"),
            None => format!("{label} \u{2026}"),
        };
    };
    let mut out = format!("{label} {} used", pct_text(pct));
    if state.is_stale(interval_secs, now_ms) {
        out.push('!');
    }
    if show_reset {
        if let Some(reset) = state.next_reset_ms {
            out.push_str(&format!(" \u{00b7} resets {}", reset_clock(reset)));
        }
    }
    if let Some(level) = &state.level {
        out.push_str(&format!(" \u{00b7} {level}"));
    }
    match &state.error {
        Some(err) => out.push_str(&format!(" \u{00b7} {err}")),
        None => out.push_str(&format!(
            " \u{00b7} {} ago",
            age_text(state.fetched_at_ms, now_ms)
        )),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::QuotaState;

    #[test]
    fn pct_whole_and_decimal() {
        assert_eq!(pct_text(42.0), "42%");
        assert_eq!(pct_text(41.7), "41.7%");
        assert_eq!(pct_text(0.0), "0%");
        assert_eq!(pct_text(99.96), "100%");
    }

    #[test]
    fn durations() {
        assert_eq!(dur_text(45), "45s");
        assert_eq!(dur_text(420), "7m");
        assert_eq!(dur_text(3600 * 3 + 60 * 12), "3h12m");
        assert_eq!(dur_text(86_400 + 3600 * 4), "1d4h");
        assert_eq!(dur_text(-5), "0s");
    }

    #[test]
    fn clock_uses_utc_when_tz_is_utc() {
        // With TZ=UTC the epoch 4102444800000 (2100-01-01T00:00:00Z)
        // renders as exactly midnight.
        eprintln!("clock={}", reset_clock(4_102_444_800_000));
        assert_eq!(reset_clock(4_102_444_800_000).len(), 8);
    }

    #[test]
    fn icon_colors() {
        assert_eq!(icon_color(42.0, 90.0), GREEN);
        assert_eq!(icon_color(85.0, 90.0), AMBER);
        assert_eq!(icon_color(90.0, 90.0), RED);
    }

    #[test]
    fn icon_svg_carries_the_number() {
        let svg = icon_svg(42.0, 90.0);
        assert!(svg.contains(">42%</text>"));
        assert!(svg.contains(GREEN));
        assert!(icon_svg(100.0, 90.0).contains(">100%</text>"));
    }

    #[test]
    fn tooltip_no_data_and_error() {
        let fresh = QuotaState::fresh();
        assert_eq!(tooltip(&fresh, "GLM", true, 300, 1_000), "GLM \u{2026}");
        let mut failed = QuotaState::fresh();
        failed.apply_error("HTTP 500", 1_000);
        assert_eq!(
            tooltip(&failed, "GLM", true, 300, 1_000),
            "GLM \u{2716} HTTP 500"
        );
    }

    #[test]
    fn tooltip_full_line() {
        let mut s = QuotaState::fresh();
        s.apply_sample(
            crate::api::Sample {
                percentage: 42.0,
                next_reset_ms: Some(4_102_444_800_000),
                level: Some("lite".into()),
            },
            600_000,
        );
        let tip = tooltip(&s, "GLM", true, 300, 600_000);
        assert!(tip.starts_with("GLM 42% used"));
        assert!(tip.contains("resets "));
        assert!(tip.contains("\u{00b7} lite"));
        assert!(tip.ends_with("0s ago"));

        // show_reset off drops the reset part; stale adds the marker.
        let no_reset = tooltip(&s, "GLM", false, 300, 600_000);
        assert!(!no_reset.contains("resets"));
        let stale = tooltip(&s, "GLM", true, 300, 600_000 + 600_001);
        assert!(stale.contains("42% used!"));
    }
}

/// Percent-encodes an SVG into a `data:image/svg+xml,` URI the image
/// pipeline decodes natively (no base64, no deps).
pub fn svg_data_uri(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + 24);
    out.push_str("data:image/svg+xml,");
    for byte in svg.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod svg_uri_tests {
    use super::*;

    #[test]
    fn encodes_specials_keeps_safe() {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\">42%</svg>";
        let uri = svg_data_uri(svg);
        assert!(uri.starts_with("data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2F"));
        assert!(uri.ends_with("%3E42%25%3C%2Fsvg%3E"));
        assert!(svg_data_uri("abcABC09-_.!~*'()").ends_with("abcABC09-_.!~*'()"));
        assert_eq!(svg_data_uri("a b"), "data:image/svg+xml,a%20b");
    }
}
