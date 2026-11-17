//! One quota snapshot, and how samples and errors land in it.

use crate::api::Sample;

/// Errors longer than this are cut; surfaces have little room.
pub const ERROR_MAX: usize = 100;

/// The app's whole view of the world: the last successful sample, or the
/// last error. A failed fetch keeps the previous sample (the user still
/// sees something) and records the error; the stale marker says how old
/// the numbers are.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaState {
    /// When this state was written (ms epoch); 0 = never fetched.
    pub fetched_at_ms: i64,
    /// Used quota in percent, once a sample is in.
    pub percentage: Option<f64>,
    /// When the quota resets (ms epoch, from the API).
    pub next_reset_ms: Option<i64>,
    /// The plan tier the API reports (`"lite"`, ...).
    pub level: Option<String>,
    /// The last error, flattened to one line and capped.
    pub error: Option<String>,
    /// When [`QuotaState::error`] was recorded.
    pub error_at_ms: Option<i64>,
}

impl Default for QuotaState {
    fn default() -> Self {
        Self::fresh()
    }
}

impl QuotaState {
    /// No data yet: nothing fetched, nothing failed.
    pub fn fresh() -> Self {
        QuotaState {
            fetched_at_ms: 0,
            percentage: None,
            next_reset_ms: None,
            level: None,
            error: None,
            error_at_ms: None,
        }
    }

    /// A sample came in: keep it, clear any error.
    pub fn apply_sample(&mut self, sample: Sample, now_ms: i64) {
        self.fetched_at_ms = now_ms;
        self.percentage = Some(sample.percentage);
        self.next_reset_ms = sample.next_reset_ms;
        self.level = sample.level;
        self.error = None;
        self.error_at_ms = None;
    }

    /// The fetch failed: keep the last good snapshot, record the error.
    pub fn apply_error(&mut self, err: impl Into<String>, now_ms: i64) {
        let mut err = err.into();
        one_line(&mut err, ERROR_MAX);
        self.error = Some(err);
        self.error_at_ms = Some(now_ms);
    }

    /// True when the render should carry the stale marker: never fetched,
    /// or the last fetch is at least two intervals old.
    pub fn is_stale(&self, interval_secs: i64, now_ms: i64) -> bool {
        if self.percentage.is_none() {
            return true;
        }
        now_ms.saturating_sub(self.fetched_at_ms) >= 2 * interval_secs * 1000
    }
}

/// Flattens newlines, tabs and other controls to single spaces, trims the
/// ends, and cuts the string at `max` characters (char-boundary safe) —
/// mirrors herdr-oc-quota's `one_line`.
pub fn one_line(s: &mut String, max: usize) {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let trimmed = out.trim();
    *s = trimmed.chars().take(max).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pct: f64) -> Sample {
        Sample {
            percentage: pct,
            next_reset_ms: Some(4_102_444_800_000),
            level: Some("lite".into()),
        }
    }

    #[test]
    fn fresh_state_is_stale() {
        let s = QuotaState::fresh();
        assert!(s.is_stale(300, 1_000));
        assert_eq!(s.percentage, None);
    }

    #[test]
    fn sample_clears_error() {
        let mut s = QuotaState::fresh();
        s.apply_error("HTTP 500", 1_000);
        s.apply_sample(sample(42.0), 2_000);
        assert_eq!(s.percentage, Some(42.0));
        assert_eq!(s.error, None);
        assert_eq!(s.fetched_at_ms, 2_000);
        assert!(!s.is_stale(300, 3_000));
    }

    #[test]
    fn error_keeps_last_snapshot() {
        let mut s = QuotaState::fresh();
        s.apply_sample(sample(42.0), 1_000);
        s.apply_error("API error: request too fast (code 1002)", 2_000);
        assert_eq!(s.percentage, Some(42.0));
        assert_eq!(
            s.error.as_deref(),
            Some("API error: request too fast (code 1002)")
        );
        assert_eq!(s.error_at_ms, Some(2_000));
    }

    #[test]
    fn stale_after_two_intervals() {
        let mut s = QuotaState::fresh();
        s.apply_sample(sample(42.0), 0);
        assert!(!s.is_stale(300, 299_999));
        assert!(s.is_stale(300, 600_000));
    }

    #[test]
    fn one_line_flattens_and_caps() {
        let mut s = "  a\nb\tc\u{1}  ".to_string();
        one_line(&mut s, 10);
        assert_eq!(s, "a b c");

        let mut long = "x".repeat(500);
        one_line(&mut long, 100);
        assert_eq!(long.chars().count(), 100);
    }

    #[test]
    fn one_line_respects_char_boundaries() {
        let mut s = "é".repeat(50);
        one_line(&mut s, 3);
        assert_eq!(s, "ééé");
    }
}
