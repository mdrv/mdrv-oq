//! When to fire an alert: one configurable percent threshold (default
//! 90), once per cycle, plus an alert when fetching starts failing.

use crate::model::QuotaState;

/// The used percent first reached `threshold` in this cycle: the new
/// sample is at or above it and either the previous sample was below it,
/// the cycle changed (the quota reset meanwhile), or there was no
/// previous sample at all (fresh start, already past it).
pub fn threshold_crossed(
    prev_pct: Option<f64>,
    prev_reset_ms: Option<i64>,
    next: &QuotaState,
    threshold: f64,
) -> bool {
    let Some(pct) = next.percentage else {
        return false;
    };
    if pct < threshold {
        return false;
    }
    let Some(prev) = prev_pct else {
        return true;
    };
    prev < threshold || (prev_reset_ms.is_some() && prev_reset_ms != next.next_reset_ms)
}

/// Fetching just started failing after having been healthy.
pub fn error_onset(prev_error: Option<&str>, next_error: Option<&str>) -> bool {
    prev_error.is_none() && next_error.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Sample;

    fn state(pct: f64, reset: Option<i64>) -> QuotaState {
        let mut s = QuotaState::fresh();
        s.apply_sample(
            Sample {
                percentage: pct,
                next_reset_ms: reset,
                level: None,
            },
            1_000,
        );
        s
    }

    #[test]
    fn fires_on_crossing() {
        let before = state(89.0, Some(100));
        let after = state(90.0, Some(100));
        assert!(threshold_crossed(
            before.percentage,
            before.next_reset_ms,
            &after,
            90.0
        ));
    }

    #[test]
    fn does_not_refire_above_threshold() {
        let before = state(90.0, Some(100));
        let after = state(91.0, Some(100));
        assert!(!threshold_crossed(
            before.percentage,
            before.next_reset_ms,
            &after,
            90.0
        ));
    }

    #[test]
    fn refires_after_cycle_change() {
        let before = state(91.0, Some(100));
        let after = state(91.5, Some(200));
        assert!(threshold_crossed(
            before.percentage,
            before.next_reset_ms,
            &after,
            90.0
        ));
    }

    #[test]
    fn fires_on_first_data_already_past() {
        let after = state(95.0, Some(100));
        assert!(threshold_crossed(None, None, &after, 90.0));
        let below = state(50.0, Some(100));
        assert!(!threshold_crossed(None, None, &below, 90.0));
    }

    #[test]
    fn below_threshold_never_fires() {
        let before = state(10.0, Some(100));
        let after = state(89.9, Some(100));
        assert!(!threshold_crossed(
            before.percentage,
            before.next_reset_ms,
            &after,
            90.0
        ));
    }

    #[test]
    fn error_onset_only_from_healthy() {
        assert!(error_onset(None, Some("HTTP 500")));
        assert!(!error_onset(Some("HTTP 500"), Some("HTTP 500")));
        assert!(!error_onset(None, None));
        assert!(!error_onset(Some("x"), None));
    }
}
