//! Config backups (P4-T08): metadata + the encrypted, compressed config (ADR-016). Like
//! notes, a backup belongs to the device once it is known, otherwise to the saved session;
//! when the session's device is identified its backups move to the device.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Db, now_ms};
use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupSource {
    Running,
    Startup,
    /// Imported from a file (or restored with undo).
    File,
}

impl BackupSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Startup => "startup",
            Self::File => "file",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "startup" => Self::Startup,
            _ => Self::File,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub id: i64,
    pub session_id: Option<i64>,
    pub device_id: Option<i64>,
    pub hostname: Option<String>,
    pub source: BackupSource,
    pub created_at: i64,
    /// Plain config size in bytes and lines.
    pub size: i64,
    pub lines: i64,
    pub sha256: String,
    /// Set on the result of a save: the newest backup already had this content.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unchanged: bool,
}

/// What to store: the plain config's metadata and its sealed bytes.
pub struct NewBackup<'a> {
    pub session_id: Option<i64>,
    pub device_id: Option<i64>,
    pub hostname: Option<&'a str>,
    pub source: BackupSource,
    pub size: i64,
    pub lines: i64,
    pub sha256: &'a str,
    pub sealed: &'a [u8],
}

const SELECT: &str = "SELECT id, session_id, device_id, hostname, source, created_at, size, lines, sha256 FROM config_backups";

fn row(r: &Row<'_>) -> rusqlite::Result<BackupInfo> {
    let source: String = r.get(4)?;
    Ok(BackupInfo {
        id: r.get(0)?,
        session_id: r.get(1)?,
        device_id: r.get(2)?,
        hostname: r.get(3)?,
        source: BackupSource::parse(&source),
        created_at: r.get(5)?,
        size: r.get(6)?,
        lines: r.get(7)?,
        sha256: r.get(8)?,
        unchanged: false,
    })
}

/// (device, session) owner: the device when known (also through the session).
fn owner(
    conn: &Connection,
    session_id: Option<i64>,
    device_id: Option<i64>,
) -> Result<(Option<i64>, Option<i64>)> {
    if device_id.is_some() {
        return Ok((device_id, None));
    }
    let Some(sid) = session_id else {
        return Err(AppError::invalid(
            "backups need a saved session or an identified device",
        ));
    };
    let dev: Option<i64> = conn
        .query_row("SELECT device_id FROM sessions WHERE id = ?1", [sid], |r| {
            r.get(0)
        })
        .optional()?
        .flatten();
    Ok(match dev {
        Some(d) => (Some(d), None),
        None => (None, Some(sid)),
    })
}

const OWNER: &str = "((?1 IS NOT NULL AND device_id = ?1) OR \
                     (?1 IS NULL AND device_id IS NULL AND session_id = ?2))";

impl Db {
    /// Newest first.
    pub fn list_backups(
        &self,
        session_id: Option<i64>,
        device_id: Option<i64>,
    ) -> Result<Vec<BackupInfo>> {
        let conn = self.conn();
        let (dev, sess) = owner(&conn, session_id, device_id)?;
        let mut stmt = conn.prepare_cached(&format!(
            "{SELECT} WHERE {OWNER} ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt
            .query_map(params![dev, sess], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Stores a backup unless the owner's newest backup of the same source has the same
    /// content (then that one comes back with `unchanged`).
    pub fn save_backup(&self, b: &NewBackup<'_>) -> Result<BackupInfo> {
        let conn = self.conn();
        let (dev, sess) = owner(&conn, b.session_id, b.device_id)?;
        let latest: Option<BackupInfo> = conn
            .query_row(
                &format!("{SELECT} WHERE {OWNER} AND source = ?3 ORDER BY created_at DESC, id DESC LIMIT 1"),
                params![dev, sess, b.source.as_str()],
                row,
            )
            .optional()?;
        if let Some(mut l) = latest
            && l.sha256 == b.sha256
        {
            l.unchanged = true;
            return Ok(l);
        }
        conn.execute(
            "INSERT INTO config_backups (session_id, device_id, hostname, source, created_at, size, lines, sha256, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![sess, dev, b.hostname, b.source.as_str(), now_ms(), b.size, b.lines, b.sha256, b.sealed],
        )?;
        let id = conn.last_insert_rowid();
        Ok(conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)?)
    }

    /// The sealed config of a backup.
    pub fn backup_data(&self, id: i64) -> Result<Vec<u8>> {
        self.conn()
            .query_row("SELECT data FROM config_backups WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| AppError::not_found(format!("backup {id}")))
    }

    pub fn delete_backup(&self, id: i64) -> Result<()> {
        if self
            .conn()
            .execute("DELETE FROM config_backups WHERE id = ?1", [id])?
            == 0
        {
            return Err(AppError::not_found(format!("backup {id}")));
        }
        Ok(())
    }

    /// When a session's device becomes known, its backups go with the device.
    pub(super) fn move_session_backups(
        conn: &Connection,
        session_id: i64,
        device_id: i64,
    ) -> Result<()> {
        conn.execute(
            "UPDATE config_backups SET device_id = ?2, session_id = NULL
             WHERE session_id = ?1 AND device_id IS NULL",
            params![session_id, device_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Protocol;

    fn session(db: &Db) -> i64 {
        let mut i = crate::import::blank_input("sw", Protocol::Ssh);
        i.host = Some("10.0.0.1".into());
        db.create_session(&i).unwrap().id
    }

    fn new<'a>(sid: i64, sha: &'a str, source: BackupSource) -> NewBackup<'a> {
        NewBackup {
            session_id: Some(sid),
            device_id: None,
            hostname: Some("sw1"),
            source,
            size: 10,
            lines: 2,
            sha256: sha,
            sealed: b"sealed-bytes",
        }
    }

    #[test]
    fn dedup_list_move_and_delete() {
        let db = Db::open_in_memory().unwrap();
        let s = session(&db);
        let a = db
            .save_backup(&new(s, "aaa", BackupSource::Running))
            .unwrap();
        assert!(!a.unchanged);
        let again = db
            .save_backup(&new(s, "aaa", BackupSource::Running))
            .unwrap();
        assert!(again.unchanged);
        assert_eq!(again.id, a.id);
        // Same content, other source: stored.
        let st = db
            .save_backup(&new(s, "aaa", BackupSource::Startup))
            .unwrap();
        assert!(!st.unchanged);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = db
            .save_backup(&new(s, "bbb", BackupSource::Running))
            .unwrap();
        let list = db.list_backups(Some(s), None).unwrap();
        assert_eq!(
            list.iter().map(|x| x.id).collect::<Vec<_>>(),
            [b.id, st.id, a.id]
        );
        assert_eq!(db.backup_data(a.id).unwrap(), b"sealed-bytes");

        let facts = crate::device::modes::DeviceFacts {
            hostname: Some("sw1".into()),
            model: None,
            version: None,
            serial: Some("S9".into()),
            uptime: None,
        };
        let dev = db.record_device(Some(s), "cisco-ios", &facts).unwrap();
        assert_eq!(
            db.list_backups(None, Some(dev.id)).unwrap().len(),
            3,
            "moved to the device"
        );
        db.delete_backup(a.id).unwrap();
        assert!(db.delete_backup(a.id).is_err());
        assert!(db.list_backups(None, None).is_err());
    }
}
