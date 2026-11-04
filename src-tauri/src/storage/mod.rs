//! SQLite storage: single file in the app data dir, WAL mode, versioned migrations,
//! repository methods on [`Db`]. Owner: P1-T04/T05 (+ port memory P1-T13).
//!
//! CONTRACT: the public method signatures on [`Db`] are used by commands and other
//! modules and must stay stable. They are implemented in submodules:
//!
//! | Area | Methods | File |
//! |---|---|---|
//! | open / migrations | `open`, `open_in_memory` | `mod.rs`, `migrations.rs` |
//! | folders | `list_folders`, `create_folder`, `rename_folder`, `move_folder`, `delete_folder` | `folders.rs` |
//! | sessions | `list_sessions`, `get_session`, `create_session`, `update_session`, `delete_session`, `duplicate_session`, `touch_session` | `sessions.rs` |
//! | settings | `get_setting`, `set_setting`, `all_settings` | `settings.rs` |
//! | port memory | `get_port_memory`, `upsert_port_memory` | `port_memory.rs` |
//!
//! Invariants:
//! - The connection lives behind a `Mutex`; every multi-statement write runs inside an
//!   `IMMEDIATE` transaction so a failure never leaves partial state behind.
//! - Secrets (passwords, keys, passphrases) are never stored here. A test guards the
//!   session DTOs and the `sessions` table against password-like fields.
//! - Lists are ordered deterministically: `(parent/folder id, sort_order, id)`.

mod folders;
mod known_hosts;
mod migrations;
mod port_memory;
mod profiles;
mod sessions;
mod settings;
mod validate;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{AppError, ErrorCode, Result};

/// How long a statement waits for a lock held by another connection (e.g. a second
/// app instance) before failing with `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Db {
    conn: Mutex<Connection>,
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::new(ErrorCode::NotFound),
            other => AppError::with(ErrorCode::Db, other.to_string()),
        }
    }
}

impl Db {
    /// Opens (creates) the database file, enables WAL + foreign keys, runs migrations.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| AppError::with(ErrorCode::Db, format!("create data dir: {e}")))?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, true)
    }

    /// In-memory database with all migrations applied (tests).
    #[allow(dead_code, reason = "contract API for other modules' tests")]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, false)
    }

    fn init(mut conn: Connection, file_backed: bool) -> Result<Self> {
        configure(&conn, file_backed)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Locks the connection. A poisoned mutex is recovered: a panic while holding the
    /// lock drops any open `Transaction`, which rolls back, so the connection is
    /// still consistent.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Per-connection pragmas. `foreign_keys` must be set outside a transaction, so this
/// runs before migrations.
fn configure(conn: &Connection, file_backed: bool) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    if file_backed {
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            // e.g. some network file systems; still usable, just less concurrent.
            tracing::warn!(mode = %mode, "SQLite WAL unavailable, using fallback journal mode");
        }
    }
    // NORMAL is durable in WAL mode except for the last transactions on power loss,
    // which is acceptable for a profile store and much faster than FULL.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let fk: bool = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
    if !fk {
        return Err(AppError::with(
            ErrorCode::Db,
            "SQLite foreign keys unsupported",
        ));
    }
    Ok(())
}

/// Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Protocol, SerialSettings, SessionInput, SessionRecord};

    fn serial_input(name: &str) -> SessionInput {
        SessionInput {
            folder_id: None,
            name: name.into(),
            protocol: Protocol::Serial,
            host: None,
            port: None,
            username: None,
            serial: Some(SerialSettings {
                port: "/dev/cu.usbserial-A1".into(),
                baud: Some(9600),
                ..SerialSettings::default()
            }),
            env_tag: None,
            tags: vec![],
            favorite: false,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
        }
    }

    #[test]
    fn db_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Db>();
    }

    #[test]
    fn in_memory_db_has_foreign_keys_on() {
        let db = Db::open_in_memory().unwrap();
        let fk: bool = db
            .conn()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn fts5_is_available() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE temp.fts_probe USING fts5(body);
             INSERT INTO temp.fts_probe(body) VALUES ('show running-config'), ('show ip route');",
        )
        .unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM temp.fts_probe WHERE fts_probe MATCH 'route'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
    }

    #[test]
    fn file_db_persists_and_reopens_at_latest_version() {
        let dir = tempfile::tempdir().unwrap();
        // Nested path proves the parent directory is created.
        let path = dir.path().join("nested").join("kenshell.db");
        let (folder_id, session_id) = {
            let db = Db::open(&path).unwrap();
            let mode: String = db
                .conn()
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .unwrap();
            assert_eq!(mode.to_ascii_lowercase(), "wal");
            let sync: i64 = db
                .conn()
                .pragma_query_value(None, "synchronous", |r| r.get(0))
                .unwrap();
            assert_eq!(sync, 1, "synchronous=NORMAL");
            let f = db.create_folder("Core", None).unwrap();
            let mut input = serial_input("Console A");
            input.folder_id = Some(f.id);
            let s = db.create_session(&input).unwrap();
            db.set_setting("theme", &serde_json::json!("dark")).unwrap();
            (f.id, s.id)
        };
        let db = Db::open(&path).unwrap();
        let version: u32 = db
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, migrations::latest_version());
        assert_eq!(db.list_folders().unwrap()[0].id, folder_id);
        let s = db.get_session(session_id).unwrap();
        assert_eq!(s.folder_id, Some(folder_id));
        assert_eq!(s.serial.unwrap().baud, Some(9600));
        assert_eq!(
            db.get_setting("theme").unwrap(),
            Some(serde_json::json!("dark"))
        );
    }

    #[test]
    fn open_rejects_database_from_newer_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kenshell.db");
        drop(Db::open(&path).unwrap());
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 999).unwrap();
        }
        let err = Db::open(&path)
            .err()
            .expect("newer schema must be rejected");
        assert_eq!(err.code, ErrorCode::Db);
    }

    #[test]
    fn concurrent_use_from_four_threads() {
        let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let db = std::sync::Arc::clone(&db);
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let s = db
                            .create_session(&serial_input(&format!("t{t}-{i}")))
                            .unwrap();
                        db.touch_session(s.id).unwrap();
                        db.set_setting(&format!("k{t}"), &serde_json::json!(i))
                            .unwrap();
                        db.list_sessions().unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions.len(), 100);
        // Appending under the lock yields unique, gap-free sort orders.
        let mut orders: Vec<i64> = sessions.iter().map(|s| s.sort_order).collect();
        orders.sort_unstable();
        assert_eq!(orders, (0..100).collect::<Vec<_>>());
        assert_eq!(db.all_settings().unwrap().len(), 4);
    }

    /// Spec rule: secrets never reach the DB. Guards the DTOs and the table schema.
    #[test]
    fn session_dtos_and_schema_have_no_secret_fields() {
        const FORBIDDEN: &[&str] = &[
            "password",
            "passwd",
            "secret",
            "passphrase",
            "token",
            "privatekey",
            "private_key",
            "credential",
        ];
        fn collect_keys(v: &serde_json::Value, out: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        out.push(k.to_ascii_lowercase());
                        collect_keys(v, out);
                    }
                }
                serde_json::Value::Array(items) => {
                    items.iter().for_each(|v| collect_keys(v, out));
                }
                _ => {}
            }
        }
        let mut input = serial_input("x");
        input.host = Some("h".into());
        input.port = Some(22);
        input.username = Some("u".into());
        input.env_tag = Some("lab".into());
        input.tags = vec!["t".into()];
        let record = SessionRecord {
            id: 1,
            folder_id: Some(1),
            name: input.name.clone(),
            protocol: input.protocol,
            host: input.host.clone(),
            port: input.port,
            username: input.username.clone(),
            serial: input.serial.clone(),
            env_tag: input.env_tag.clone(),
            tags: input.tags.clone(),
            favorite: true,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            last_connected_at: Some(0),
        };
        let mut keys = Vec::new();
        collect_keys(&serde_json::to_value(&input).unwrap(), &mut keys);
        collect_keys(&serde_json::to_value(&record).unwrap(), &mut keys);

        let db = Db::open_in_memory().unwrap();
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('sessions')")
            .unwrap();
        let cols = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(cols.len() > 10, "sessions table introspection failed");
        keys.extend(cols.into_iter().map(|c| c.to_ascii_lowercase()));

        // A *reference* to a credential profile is fine: the secret lives in the keychain.
        const ALLOWED: &[&str] = &["credentialprofileid", "credential_profile_id"];
        for key in keys.iter().filter(|k| !ALLOWED.contains(&k.as_str())) {
            for bad in FORBIDDEN {
                assert!(!key.contains(bad), "field `{key}` looks like a secret");
            }
        }
    }
}
