//! Devices learned from the identity command (P3-T04). A device is the hardware behind one
//! or more sessions (console + SSH of the same switch); it is found again by serial number.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

use super::{Db, now_ms};
use crate::device::modes::DeviceFacts;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: i64,
    /// Vendor profile id, e.g. `ruijie-rgos`.
    pub vendor: String,
    pub hostname: Option<String>,
    pub model: Option<String>,
    pub os_version: Option<String>,
    pub serial: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
}

const SELECT: &str = "SELECT id, vendor, hostname, model, os_version, serial, first_seen, \
                      last_seen FROM devices";
const MAX_FIELD: usize = 200;

fn row(r: &Row<'_>) -> rusqlite::Result<DeviceRecord> {
    Ok(DeviceRecord {
        id: r.get(0)?,
        vendor: r.get(1)?,
        hostname: r.get(2)?,
        model: r.get(3)?,
        os_version: r.get(4)?,
        serial: r.get(5)?,
        first_seen: r.get(6)?,
        last_seen: r.get(7)?,
    })
}

/// Device text from the network: printable, trimmed, bounded.
fn clean(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(MAX_FIELD)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|s| !s.is_empty())
}

fn by_id(conn: &Connection, id: i64) -> Result<Option<DeviceRecord>> {
    Ok(conn
        .prepare_cached(&format!("{SELECT} WHERE id = ?1"))?
        .query_row([id], row)
        .optional()?)
}

impl Db {
    /// Stores what a session learned: the same serial → the same device (updated), else the
    /// session's previous device, else a new one. Links the session (if saved) to it.
    pub fn record_device(
        &self,
        session_id: Option<i64>,
        vendor: &str,
        facts: &DeviceFacts,
    ) -> Result<DeviceRecord> {
        let (hostname, model, version, serial) = (
            clean(&facts.hostname),
            clean(&facts.model),
            clean(&facts.version),
            clean(&facts.serial),
        );
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now_ms();
        let by_serial: Option<i64> = match &serial {
            Some(sn) => tx
                .query_row("SELECT id FROM devices WHERE serial = ?1", [sn], |r| {
                    r.get(0)
                })
                .optional()?,
            None => None,
        };
        let from_session: Option<i64> = match session_id {
            Some(sid) => tx
                .query_row("SELECT device_id FROM sessions WHERE id = ?1", [sid], |r| {
                    r.get::<_, Option<i64>>(0)
                })
                .optional()?
                .flatten(),
            None => None,
        };
        // A session whose device now reports another serial got new hardware: new row.
        let reuse = by_serial.or_else(|| from_session.filter(|_| serial.is_none()));
        let id = match reuse {
            Some(id) => {
                tx.execute(
                    "UPDATE devices SET vendor = ?2,
                         hostname = COALESCE(?3, hostname), model = COALESCE(?4, model),
                         os_version = COALESCE(?5, os_version), serial = COALESCE(?6, serial),
                         last_seen = ?7
                     WHERE id = ?1",
                    params![id, vendor, hostname, model, version, serial, now],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO devices (vendor, hostname, model, os_version, serial,
                         first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    params![vendor, hostname, model, version, serial, now],
                )?;
                tx.last_insert_rowid()
            }
        };
        if let Some(sid) = session_id {
            tx.execute(
                "UPDATE sessions SET device_id = ?2 WHERE id = ?1",
                params![sid, id],
            )?;
            Db::move_session_notes(&tx, sid, id)?;
            Db::move_session_history(&tx, sid, id)?;
        }
        let rec = by_id(&tx, id)?.expect("device just written");
        tx.commit()?;
        Ok(rec)
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("{SELECT} ORDER BY last_seen DESC"))?;
        let rows = stmt.query_map([], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Protocol;

    fn facts(host: &str, serial: Option<&str>, version: &str) -> DeviceFacts {
        DeviceFacts {
            hostname: Some(host.into()),
            model: Some("S5750-48GT4XS-E".into()),
            version: Some(version.into()),
            serial: serial.map(Into::into),
            uptime: Some("1 day".into()),
        }
    }

    #[test]
    fn same_serial_is_the_same_device_and_sessions_are_linked() {
        let db = Db::open_in_memory().unwrap();
        let mut input = crate::import::blank_input("console", Protocol::Ssh);
        input.host = Some("10.0.0.1".into());
        let console = db.create_session(&input).unwrap();
        input.name = "ssh".into();
        let ssh = db.create_session(&input).unwrap();

        let a = db
            .record_device(
                Some(console.id),
                "ruijie-rgos",
                &facts("sw1", Some("G1MQ"), "11.4"),
            )
            .unwrap();
        let b = db
            .record_device(
                Some(ssh.id),
                "ruijie-rgos",
                &facts("sw1-new", Some("G1MQ"), "12.5"),
            )
            .unwrap();
        assert_eq!(a.id, b.id, "one device behind both sessions");
        assert_eq!(b.hostname.as_deref(), Some("sw1-new"));
        assert_eq!(b.os_version.as_deref(), Some("12.5"));
        assert_eq!(db.get_session(console.id).unwrap().device_id, Some(a.id));
        assert_eq!(db.get_session(ssh.id).unwrap().device_id, Some(a.id));

        // Replaced hardware (other serial) behind the same session: a new device.
        let c = db
            .record_device(
                Some(ssh.id),
                "ruijie-rgos",
                &facts("sw1", Some("ZZZ9"), "12.5"),
            )
            .unwrap();
        assert_ne!(c.id, a.id);
        assert_eq!(db.list_devices().unwrap().len(), 2);
    }

    #[test]
    fn no_serial_updates_the_sessions_device_and_text_is_cleaned() {
        let db = Db::open_in_memory().unwrap();
        let mut input = crate::import::blank_input("r1", Protocol::Telnet);
        input.host = Some("10.0.0.9".into());
        let s = db.create_session(&input).unwrap();
        let a = db
            .record_device(
                Some(s.id),
                "cisco-like",
                &facts("r1\u{1b}[0m", None, "15.2"),
            )
            .unwrap();
        assert_eq!(a.hostname.as_deref(), Some("r1[0m"));
        let b = db
            .record_device(Some(s.id), "cisco-ios", &facts("r1", None, "15.9"))
            .unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.vendor, "cisco-ios");
        // Quick-connect tab (no saved session): stored, not linked.
        let q = db
            .record_device(None, "cisco-like", &facts("q", None, "1"))
            .unwrap();
        assert_ne!(q.id, a.id);
    }
}
