//! The Android foreground service owns polling; the UI only mirrors the
//! snapshot file the service writes. One poller, one clock — otherwise
//! the app and the persistent notification disagree about how old the
//! numbers are (they did, with two independent poll loops).

use std::path::PathBuf;

use crate::model::QuotaState;

/// Where the Android service writes its latest snapshot; `None` on
/// platforms without the service (desktop keeps its own poll loop).
pub fn state_file() -> Option<PathBuf> {
    crate::settings::service_state_file()
}

/// Where the UI drops the refresh request marker for the service;
/// `None` on platforms without the service.
pub fn refresh_marker() -> Option<PathBuf> {
    crate::settings::service_state_dir().map(|dir| dir.join("refresh"))
}

/// Asks the Android service for an out-of-band poll: create the marker
/// file its tick watches for. Best-effort — a dead service leaves it
/// unconsumed, and the chip's busy timeout covers that.
pub fn poke_refresh() {
    if let Some(path) = refresh_marker() {
        let _ = std::fs::write(path, b"");
    }
}

/// Parse the service's `state.json` into a snapshot. Lenient: a missing
/// or malformed file yields "never fetched", not an error.
pub fn parse_state(json: &str) -> QuotaState {
    let mut st = QuotaState::fresh();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return st;
    };
    st.fetched_at_ms = v["fetched_at_ms"].as_i64().unwrap_or(0);
    st.percentage = v["percentage"].as_f64();
    st.next_reset_ms = v["next_reset_ms"].as_i64();
    st.level = v["level"].as_str().map(str::to_owned);
    st.error = v["error"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if st.error.is_some() {
        st.error_at_ms = Some(st.fetched_at_ms);
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_snapshot() {
        let st = parse_state(
            r#"{"percentage":18.4,"next_reset_ms":1774362916000,
                "level":"lite","fetched_at_ms":1774344000000,"error":null}"#,
        );
        assert_eq!(st.percentage, Some(18.4));
        assert_eq!(st.next_reset_ms, Some(1774362916000));
        assert_eq!(st.level.as_deref(), Some("lite"));
        assert_eq!(st.fetched_at_ms, 1774344000000);
        assert_eq!(st.error, None);
        assert_eq!(st.error_at_ms, None);
    }

    #[test]
    fn parses_an_error_snapshot() {
        let st = parse_state(
            r#"{"percentage":null,"next_reset_ms":null,"level":null,
                "fetched_at_ms":1000,"error":"fetch failed"}"#,
        );
        assert_eq!(st.percentage, None);
        assert_eq!(st.error.as_deref(), Some("fetch failed"));
        assert_eq!(st.error_at_ms, Some(1000));
    }

    #[test]
    fn malformed_yields_never_fetched() {
        let st = parse_state("not json at all");
        assert_eq!(st, QuotaState::fresh());
    }
}
