//! Versioned schema migrations tracked in `PRAGMA user_version`.
//!
//! Rules:
//! - `MIGRATIONS[i]` brings the schema from version `i` to `i + 1`.
//! - Never edit a step that has shipped; only append new ones.
//! - Each step runs in its own transaction together with the `user_version` bump, and a
//!   `foreign_key_check` runs before commit, so table rebuilds cannot leave dangling
//!   references behind. Statements that cannot run inside a transaction (e.g.
//!   `PRAGMA foreign_keys`, `VACUUM`) do not belong in a step.
//!
//! Planned: Phase 2 appends `known_hosts`; Phase 3 appends `devices` / `notes` with FTS5.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{AppError, ErrorCode, Result};

struct Migration {
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "v1 folders, sessions, settings, port_memory",
        sql: r#"
CREATE TABLE folders (
    id         INTEGER PRIMARY KEY,
    parent_id  INTEGER NULL REFERENCES folders(id),
    name       TEXT    NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_folders_parent ON folders(parent_id, sort_order, id);

CREATE TABLE sessions (
    id                INTEGER PRIMARY KEY,
    folder_id         INTEGER NULL REFERENCES folders(id),
    name              TEXT    NOT NULL,
    protocol          TEXT    NOT NULL
                      CHECK (protocol IN ('serial', 'ssh', 'telnet', 'rlogin', 'raw')),
    host              TEXT    NULL,
    port              INTEGER NULL CHECK (port IS NULL OR port BETWEEN 1 AND 65535),
    username          TEXT    NULL,
    serial_json       TEXT    NULL,
    env_tag           TEXT    NULL,
    tags_json         TEXT    NOT NULL DEFAULT '[]',
    favorite          INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    sort_order        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_connected_at INTEGER NULL
);
CREATE INDEX idx_sessions_folder ON sessions(folder_id, sort_order, id);
CREATE INDEX idx_sessions_recent ON sessions(last_connected_at)
    WHERE last_connected_at IS NOT NULL;

CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL
);

CREATE TABLE port_memory (
    key           TEXT PRIMARY KEY NOT NULL,
    path          TEXT    NOT NULL,
    vid           INTEGER NULL,
    pid           INTEGER NULL,
    serial_number TEXT    NULL,
    baud          INTEGER NOT NULL,
    last_device   TEXT    NULL,
    updated_at    INTEGER NOT NULL
);
"#,
    },
    Migration {
        name: "v2 known_hosts (SSH host keys)",
        sql: r#"
CREATE TABLE known_hosts (
    host       TEXT    NOT NULL,
    port       INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    algorithm  TEXT    NOT NULL,
    public_key TEXT    NOT NULL,
    fingerprint TEXT   NOT NULL,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (host, port)
);
"#,
    },
    Migration {
        name: "v3 sessions.ssh_json (SSH options)",
        sql: "ALTER TABLE sessions ADD COLUMN ssh_json TEXT NULL;",
    },
];

/// Schema version this build expects.
pub(super) fn latest_version() -> u32 {
    MIGRATIONS.len() as u32
}

/// Brings the database up to [`latest_version`]. Idempotent.
pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let latest = latest_version();
    if current > latest {
        // Opened by an older app after a newer one migrated it: refuse rather than
        // risk writing rows the newer schema considers invalid.
        return Err(AppError::with(
            ErrorCode::Db,
            format!("database schema v{current} is newer than this app (v{latest})"),
        ));
    }
    for (idx, step) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let version = idx as u32 + 1;
        apply(conn, version, step).map_err(|e| {
            AppError::with(
                ErrorCode::Db,
                format!(
                    "migration v{version} ({}) failed: {}",
                    step.name,
                    e.detail.unwrap_or_default()
                ),
            )
        })?;
        tracing::info!(version, name = step.name, "applied database migration");
    }
    Ok(())
}

fn apply(conn: &mut Connection, version: u32, step: &Migration) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(step.sql)?;
    let violations: i64 =
        tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if violations > 0 {
        return Err(AppError::with(
            ErrorCode::Db,
            format!("{violations} foreign key violations"),
        ));
    }
    tx.pragma_update(None, "user_version", version)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn fresh_db_reaches_latest_version_with_all_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(user_version(&conn), latest_version());
        assert_eq!(
            tables(&conn),
            vec![
                "folders",
                "known_hosts",
                "port_memory",
                "sessions",
                "settings"
            ]
        );
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(user_version(&conn), latest_version());
    }

    #[test]
    fn every_step_has_name_and_sql() {
        assert!(!MIGRATIONS.is_empty());
        for m in MIGRATIONS {
            assert!(!m.name.trim().is_empty());
            assert!(!m.sql.trim().is_empty());
        }
    }

    #[test]
    fn failed_step_rolls_back_and_keeps_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        let bad = Migration {
            name: "broken",
            sql: "CREATE TABLE ok_table (x INTEGER); THIS IS NOT SQL;",
        };
        assert!(apply(&mut conn, 1, &bad).is_err());
        assert_eq!(user_version(&conn), 0);
        assert!(tables(&conn).is_empty(), "partial step must roll back");
    }

    #[test]
    fn step_with_dangling_foreign_key_is_rejected() {
        // Enforcement is off on this bare connection (as during a table rebuild), so
        // the insert succeeds and only the pre-commit check can catch it.
        let mut conn = Connection::open_in_memory().unwrap();
        let bad = Migration {
            name: "dangling",
            sql: "CREATE TABLE p (id INTEGER PRIMARY KEY);
                  CREATE TABLE c (pid INTEGER REFERENCES p(id));
                  INSERT INTO c (pid) VALUES (42);",
        };
        assert!(apply(&mut conn, 1, &bad).is_err());
        assert_eq!(user_version(&conn), 0);
    }

    #[test]
    fn check_constraints_guard_protocol_and_port() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let bad_protocol = conn.execute(
            "INSERT INTO sessions (name, protocol, created_at, updated_at)
             VALUES ('x', 'ftp', 0, 0)",
            [],
        );
        assert!(bad_protocol.is_err());
        let bad_port = conn.execute(
            "INSERT INTO sessions (name, protocol, port, created_at, updated_at)
             VALUES ('x', 'ssh', 0, 0, 0)",
            [],
        );
        assert!(bad_port.is_err());
    }
}
