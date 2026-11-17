//! Where the Z.AI key comes from, in order: the key set in the app,
//! `ZAI_API_KEY`, OpenCode's credential DB (v2), its legacy `auth.json`.
//! The key itself never appears in errors, logs, or rendered output.

use std::path::PathBuf;

use serde_json::Value;

/// Where a resolved key came from — safe to show in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Settings,
    Env,
    OpenCodeDb,
    AuthJson,
}

impl Origin {
    /// The human-readable name of the source.
    pub fn label(self) -> &'static str {
        match self {
            Origin::Settings => "app settings",
            Origin::Env => "ZAI_API_KEY",
            Origin::OpenCodeDb => "OpenCode credential DB",
            Origin::AuthJson => "OpenCode auth.json (legacy)",
        }
    }
}

/// Inputs for one resolution; `None` paths mean the defaults. Callers
/// read the environment themselves (`ZAI_API_KEY`), so tests can inject.
#[derive(Debug, Clone, Default)]
pub struct KeyLookup {
    pub settings_key: Option<String>,
    pub env_key: Option<String>,
    pub opencode_db: Option<PathBuf>,
    pub auth_json: Option<PathBuf>,
}

/// A resolved key. Only `origin` is safe to render; never log `key`.
#[derive(Debug, Clone)]
pub struct KeySource {
    pub key: String,
    pub origin: Origin,
}

/// The OpenCode v2 credential DB, or `None` when no data dir exists.
pub fn opencode_db_path() -> Option<PathBuf> {
    opencode_data_dir().map(|d| d.join("opencode.db"))
}

/// The legacy `auth.json`, or `None` when no data dir exists.
pub fn auth_json_path() -> Option<PathBuf> {
    opencode_data_dir().map(|d| d.join("auth.json"))
}

fn opencode_data_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("opencode"));
        }
    }
    home_dir().map(|h| h.join(".local/share/opencode"))
}

fn home_dir() -> Option<PathBuf> {
    match std::env::var("HOME") {
        Ok(h) if !h.trim().is_empty() => Some(PathBuf::from(h)),
        _ => None,
    }
}

fn nonempty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Resolves the key in order: settings, environment, OpenCode credential
/// DB, legacy auth.json. The first source that yields a key wins; a
/// credential DB that exists but cannot be read is remembered and
/// reported if nothing else pans out.
pub fn resolve_key(l: &KeyLookup) -> Result<KeySource, String> {
    let mut db_err = None;

    if let Some(k) = nonempty(&l.settings_key) {
        return Ok(KeySource {
            key: k,
            origin: Origin::Settings,
        });
    }
    if let Some(k) = nonempty(&l.env_key) {
        return Ok(KeySource {
            key: k,
            origin: Origin::Env,
        });
    }

    if let Some(path) = l.opencode_db.clone().or_else(opencode_db_path) {
        match read_db_key(&path) {
            Ok(Some(k)) => {
                return Ok(KeySource {
                    key: k,
                    origin: Origin::OpenCodeDb,
                })
            }
            Ok(None) => {}
            Err(e) => db_err = Some(e),
        }
    }

    if let Some(path) = l.auth_json.clone().or_else(auth_json_path) {
        if let Ok(Some(k)) = read_auth_json_key(&path) {
            return Ok(KeySource {
                key: k,
                origin: Origin::AuthJson,
            });
        }
    }

    Err(db_err.unwrap_or_else(|| {
        "no Z.AI credential found (set a key in the app, or ZAI_API_KEY, \
or log in via opencode)"
            .into()
    }))
}

/// The active `zai-coding-plan` (or its `zai-coding-sleev` alias) key from
/// the OpenCode v2 credential DB: read-only, a point query on the tiny
/// `credential` table, safe next to a running server (WAL). `Ok(None)`
/// when the DB has no usable row; `Err` when it exists but cannot be read.
fn read_db_key(path: &std::path::Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("cannot read opencode.db: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| format!("cannot read opencode.db: {e}"))?;

    // Older builds named the column `connector_id`.
    let has_integration: bool = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('credential') WHERE name = 'integration_id'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| format!("cannot read opencode.db: {e}"))?;
    let col = if has_integration {
        "integration_id"
    } else {
        "connector_id"
    };

    // The one-time legacy import leaves `active` NULL; rows written by
    // `opencode auth login` carry `active = 1`. Prefer the canonical
    // provider id over the alias, then the newest row.
    let sql = format!(
        "SELECT value FROM credential WHERE {col} IN ('zai-coding-plan', \
'zai-coding-sleev') AND (active IS NULL OR active != 0) \
ORDER BY ({col} = 'zai-coding-plan') DESC, time_updated DESC LIMIT 1"
    );
    let value: Option<String> = conn
        .query_row(&sql, [], |r| r.get(0))
        .optional()
        .map_err(|e| format!("cannot read opencode.db: {e}"))?;
    let Some(value) = value else {
        return Ok(None);
    };

    // `value` is JSON: {"type":"key","key":...} or an OAuth blob whose
    // `access` (else `refresh`) token the API accepts.
    let parsed: Result<Value, _> = serde_json::from_str(&value);
    let parsed = match parsed {
        Ok(v) => v,
        Err(_) => return Ok(None), // unusable row; let later sources answer
    };
    let token = if parsed.get("type").and_then(|t| t.as_str()) == Some("oauth") {
        parsed
            .get("access")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("refresh").and_then(|v| v.as_str()))
    } else {
        parsed.get("key").and_then(|v| v.as_str())
    };
    Ok(token.map(str::to_owned).filter(|t| !t.trim().is_empty()))
}

/// The legacy store: `{"zai-coding-plan": {"key": ...}}` (or `.token` for
/// OAuth entries). `Ok(None)` when the file is absent or has no usable key.
fn read_auth_json_key(path: &std::path::Path) -> Result<Option<String>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let parsed: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let entry = parsed.get("zai-coding-plan");
    let key = entry
        .and_then(|e| e.get("key"))
        .and_then(|v| v.as_str())
        .filter(|k| !k.trim().is_empty());
    Ok(key.map(str::to_owned))
}

/// Used by tests to build fixture DBs in a temp dir (the same schema the
/// migration and `opencode auth login` write).
#[cfg(test)]
pub(crate) fn write_fixture_db(path: &std::path::Path, rows: &[(&str, Option<i64>, &str, i64)]) {
    use rusqlite::Connection;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE credential (id text PRIMARY KEY, integration_id text, \
label text NOT NULL, value text NOT NULL, connector_id text, method_id text, \
active integer, time_created integer NOT NULL, time_updated integer NOT NULL)",
        [],
    )
    .unwrap();
    for (i, (integ, active, key, updated)) in rows.iter().enumerate() {
        conn.execute(
            "INSERT INTO credential VALUES (?1, ?2, 'API key', ?3, NULL, NULL, ?4, 1700000000, ?5)",
            rusqlite::params![
                format!("id{i}"),
                integ,
                format!("{{\"type\":\"key\",\"key\":\"{key}\"}}"),
                *active,
                *updated
            ],
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdrv-oq-key-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_first_then_env() {
        let l = KeyLookup {
            settings_key: Some("  k-app  ".into()),
            env_key: Some("k-env".into()),
            ..Default::default()
        };
        let ks = resolve_key(&l).unwrap();
        assert_eq!(ks.key, "k-app");
        assert_eq!(ks.origin, Origin::Settings);

        let l = KeyLookup {
            settings_key: None,
            env_key: Some("  k-env ".into()),
            ..Default::default()
        };
        assert_eq!(resolve_key(&l).unwrap().origin, Origin::Env);

        // Empty/whitespace values are skipped; the auto sources are
        // pointed at nothing so the machine's real OpenCode store can't
        // leak into the test.
        let l = KeyLookup {
            settings_key: Some("   ".into()),
            env_key: Some("".into()),
            opencode_db: Some(PathBuf::from("/nonexistent/opencode.db")),
            auth_json: Some(PathBuf::from("/nonexistent/auth.json")),
        };
        assert!(resolve_key(&l).is_err());
    }

    #[test]
    fn db_row_wins_over_auth_json() {
        let dir = tmp("db-wins");
        let db = dir.join("opencode.db");
        write_fixture_db(
            &db,
            &[("zai-coding-plan", Some(1), "db-key", 1_700_000_001)],
        );
        std::fs::create_dir_all(dir.join("opencode")).unwrap();
        std::fs::write(
            dir.join("opencode/auth.json"),
            r#"{"zai-coding-plan":{"key":"json-key"}}"#,
        )
        .unwrap();
        let l = KeyLookup {
            opencode_db: Some(db),
            auth_json: Some(dir.join("opencode/auth.json")),
            ..Default::default()
        };
        let ks = resolve_key(&l).unwrap();
        assert_eq!(ks.key, "db-key");
        assert_eq!(ks.origin, Origin::OpenCodeDb);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn active_null_and_alias_rows() {
        let dir = tmp("active");
        let db = dir.join("opencode.db");
        // active = 0 excluded even though it is newest.
        write_fixture_db(
            &db,
            &[
                ("zai-coding-plan", Some(0), "stale", 1_700_000_009),
                ("zai-coding-plan", None, "imported", 1_700_000_001),
            ],
        );
        let ks = read_db_key(&db).unwrap().unwrap();
        assert_eq!(ks, "imported");

        let db2 = dir.join("alias.db");
        write_fixture_db(&db2, &[("zai-coding-sleev", Some(1), "aliased", 5)]);
        assert_eq!(read_db_key(&db2).unwrap().unwrap(), "aliased");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn newest_row_and_no_match_fallback() {
        let dir = tmp("newest");
        let db = dir.join("opencode.db");
        write_fixture_db(
            &db,
            &[
                ("zai-coding-plan", Some(1), "old", 5),
                ("zai-coding-plan", Some(1), "new", 9),
            ],
        );
        assert_eq!(read_db_key(&db).unwrap().unwrap(), "new");

        let empty = dir.join("empty.db");
        write_fixture_db(&empty, &[("other-provider", Some(1), "k", 1)]);
        assert_eq!(read_db_key(&empty).unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oauth_blob_uses_access_token() {
        let dir = tmp("oauth");
        let db = dir.join("opencode.db");
        write_fixture_db(&db, &[("zai-coding-plan", Some(1), "ignored", 1)]);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE credential SET value = ?1",
            [r#"{"type":"oauth","access":"tok","refresh":"r"}"#],
        )
        .unwrap();
        assert_eq!(read_db_key(&db).unwrap().unwrap(), "tok");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_db_reports_but_auth_json_can_rescue() {
        let dir = tmp("corrupt");
        let db = dir.join("opencode.db");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&db, b"this is not a database").unwrap();
        let auth = dir.join("auth.json");
        std::fs::write(&auth, r#"{"zai-coding-plan":{"key":"json-key"}}"#).unwrap();

        let l = KeyLookup {
            opencode_db: Some(db.clone()),
            auth_json: Some(auth),
            ..Default::default()
        };
        let ks = resolve_key(&l).unwrap();
        assert_eq!(ks.origin, Origin::AuthJson);

        // No auth.json: the DB error is the answer.
        let l = KeyLookup {
            opencode_db: Some(db),
            auth_json: Some(dir.join("nope.json")),
            ..Default::default()
        };
        let err = resolve_key(&l).unwrap_err();
        assert!(err.contains("cannot read opencode.db"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_everything_gives_guidance() {
        let l = KeyLookup {
            opencode_db: Some(PathBuf::from("/nonexistent/db.sqlite")),
            auth_json: Some(PathBuf::from("/nonexistent/auth.json")),
            ..Default::default()
        };
        let err = resolve_key(&l).unwrap_err();
        assert!(err.contains("no Z.AI credential found"), "{err}");
    }

    /// The real store on this machine, when present: resolution must find
    /// the key OpenCode v2 actually uses. Skipped when there is none.
    #[test]
    fn real_store_when_present() {
        let Some(db) = opencode_db_path() else { return };
        if !db.exists() {
            return;
        }
        let l = KeyLookup::default();
        let ks = resolve_key(&l).expect("real store should resolve");
        assert_eq!(ks.origin, Origin::OpenCodeDb);
        assert!(!ks.key.is_empty());
    }
}
