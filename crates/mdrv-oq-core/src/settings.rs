//! The app's settings, in a local Turso (libSQL) file — the same store the
//! Android side will read later. Keys are the `settings` constants; every
//! access is synchronous from the caller's thread (Turso runs on the
//! handle's own current-thread runtime; one process per file).

use std::path::{Path, PathBuf};

use turso::{Builder, Value};

/// The API key set in the app (overrides env/OpenCode sources).
pub const API_KEY: &str = "api_key";
/// Poll interval in seconds.
pub const INTERVAL_SECS: &str = "interval_secs";
/// Alert threshold in used-percent.
pub const THRESHOLD_PCT: &str = "threshold_pct";
/// `"1"`/`"0"`: show the reset time in tooltip/window/notification.
pub const SHOW_RESET: &str = "show_reset";

/// Defaults, when the store has nothing (or the value is malformed).
pub const DEFAULT_INTERVAL_SECS: i64 = 300;
pub const DEFAULT_THRESHOLD_PCT: f64 = 90.0;
pub const DEFAULT_SHOW_RESET: bool = true;

/// A synchronous handle over one Turso file.
pub struct Settings {
    rt: tokio::runtime::Runtime,
    conn: turso::Connection,
}

impl Settings {
    /// Opens (creating if needed) the store at `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("settings runtime: {e}"))?;
        let db_str = path.display().to_string();
        let db = rt
            .block_on(Builder::new_local(&db_str).build())
            .map_err(|e| format!("cannot open {db_str}: {e}"))?;
        let conn = db.connect().map_err(|e| format!("connect: {e}"))?;
        rt.block_on(
            conn.execute(
                "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
                (),
            ),
        )
        .map_err(|e| format!("init settings: {e}"))?;
        Ok(Settings { rt, conn })
    }

    /// Reads one setting, `None` when unset.
    pub fn get(&self, key: &str) -> Option<String> {
        self.rt.block_on(async {
            let mut rows = self
                .conn
                .query("SELECT value FROM settings WHERE key = ?1", (key,))
                .await
                .ok()?;
            let Some(row) = rows.next().await.ok()? else {
                return None;
            };
            value_text(row.get_value(0).ok()?)
        })
    }

    /// Writes one setting; `None` deletes it.
    pub fn set(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        self.rt.block_on(async {
            match value {
                Some(v) => self
                    .conn
                    .execute(
                        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        (key, v),
                    )
                    .await,
                None => {
                    self.conn
                        .execute("DELETE FROM settings WHERE key = ?1", (key,))
                        .await
                }
            }
            .map(|_| ())
            .map_err(|e| format!("write {key}: {e}"))
        })
    }
}

fn value_text(v: Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s),
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(f) => Some(f.to_string()),
        _ => None,
    }
}

/// Parses a stored interval, falling back to the default.
pub fn interval_secs(s: &Settings) -> i64 {
    s.get(INTERVAL_SECS)
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v >= 30 && *v <= 86_400)
        .unwrap_or(DEFAULT_INTERVAL_SECS)
}

/// Parses a stored threshold, falling back to the default.
pub fn threshold_pct(s: &Settings) -> f64 {
    s.get(THRESHOLD_PCT)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v >= 50.0 && *v <= 99.0)
        .unwrap_or(DEFAULT_THRESHOLD_PCT)
}

/// Parses a stored show-reset flag, falling back to the default.
pub fn show_reset(s: &Settings) -> bool {
    match s.get(SHOW_RESET).as_deref() {
        Some("0") => false,
        Some("1") => true,
        _ => DEFAULT_SHOW_RESET,
    }
}

/// `$XDG_STATE_HOME/mdrv-oq/settings.db`, defaulting to
/// `~/.local/state/mdrv-oq/settings.db`.
pub fn default_db_path() -> PathBuf {
    // Android: no HOME/TMPDIR and cwd "/" — find the app's private files
    // dir from the package name in /proc/self/cmdline, then require it to
    // be writable before trusting it.
    if cfg!(target_os = "android") {
        if let Some(path) = android_files_state_dir() {
            return path.join("mdrv-oq").join("settings.db");
        }
    }
    let base = match std::env::var("MDRV_OQ_STATE_DIR") {
        Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
        _ => match std::env::var("XDG_STATE_HOME") {
            Ok(s) if !s.trim().is_empty() => PathBuf::from(s),
            Ok(_) | Err(_) => match std::env::var("HOME") {
                Ok(h) if !h.trim().is_empty() => PathBuf::from(h).join(".local/state"),
                _ => PathBuf::from("."),
            },
        },
    };
    base.join("mdrv-oq").join("settings.db")
}

/// `/data/data/<package>/files/state` on Android, from the package name in
/// `/proc/self/cmdline`; `None` anywhere along the way or when the
/// directory cannot be created (then the caller falls back).
/// The Android service's latest-snapshot file (inside its files dir);
/// `None` on other platforms.
pub fn service_state_file() -> Option<PathBuf> {
    android_files_state_dir().map(|dir| dir.join("state.json"))
}

#[cfg(target_os = "android")]
fn android_files_state_dir() -> Option<PathBuf> {
    let cmdline = std::fs::read("/proc/self/cmdline").ok()?;
    let package = String::from_utf8_lossy(&cmdline)
        .split('\0')
        .next()?
        .to_owned();
    if package.is_empty() || package.contains('/') || package.starts_with('.') {
        return None;
    }
    let dir = PathBuf::from("/data/data")
        .join(package)
        .join("files/state");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Not Android: no private files dir, always fall back.
#[cfg(not(target_os = "android"))]
fn android_files_state_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mdrv-oq-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.db")
    }

    #[test]
    fn open_creates_and_round_trips() {
        let path = tmp("round");
        let s = Settings::open(&path).unwrap();
        assert_eq!(s.get(API_KEY), None);

        s.set(API_KEY, Some("k1")).unwrap();
        s.set(INTERVAL_SECS, Some("120")).unwrap();
        assert_eq!(s.get(API_KEY).as_deref(), Some("k1"));
        assert_eq!(s.get(INTERVAL_SECS).as_deref(), Some("120"));

        s.set(API_KEY, Some("k2")).unwrap();
        assert_eq!(s.get(API_KEY).as_deref(), Some("k2"));

        s.set(API_KEY, None).unwrap();
        assert_eq!(s.get(API_KEY), None);

        // Reopen: values persist.
        drop(s);
        let s = Settings::open(&path).unwrap();
        assert_eq!(s.get(INTERVAL_SECS).as_deref(), Some("120"));
        assert_eq!(s.get(API_KEY), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn defaults_and_bounds() {
        let path = tmp("defaults");
        let s = Settings::open(&path).unwrap();
        assert_eq!(interval_secs(&s), DEFAULT_INTERVAL_SECS);
        assert_eq!(threshold_pct(&s), DEFAULT_THRESHOLD_PCT);
        assert!(show_reset(&s));

        s.set(INTERVAL_SECS, Some("10")).unwrap(); // below the floor
        assert_eq!(interval_secs(&s), DEFAULT_INTERVAL_SECS);
        s.set(INTERVAL_SECS, Some("600")).unwrap();
        assert_eq!(interval_secs(&s), 600);

        s.set(THRESHOLD_PCT, Some("42")).unwrap(); // out of range
        assert_eq!(threshold_pct(&s), DEFAULT_THRESHOLD_PCT);
        s.set(THRESHOLD_PCT, Some("95")).unwrap();
        assert_eq!(threshold_pct(&s), 95.0);

        s.set(SHOW_RESET, Some("0")).unwrap();
        assert!(!show_reset(&s));
        s.set(SHOW_RESET, Some("junk")).unwrap();
        assert!(show_reset(&s));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
