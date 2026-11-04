//! Console cable memory (P1-T13): the last working baud (and device) per cable, keyed
//! by USB serial number or port path, so a re-plugged cable connects instantly.

use rusqlite::{OptionalExtension, params};

use super::{Db, now_ms, validate};
use crate::error::{AppError, Result};
use crate::model::PortMemory;

/// Keys are identities produced by console detection; they are matched exactly
/// (never trimmed) but must not be blank.
fn check_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(AppError::invalid("port memory key is empty"));
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "contract API; consumed by console_detect/ and session/ (P1)"
)]
impl Db {
    pub fn get_port_memory(&self, key: &str) -> Result<Option<PortMemory>> {
        check_key(key)?;
        Ok(self
            .conn()
            .query_row(
                "SELECT key, path, vid, pid, serial_number, baud, last_device, updated_at
                 FROM port_memory WHERE key = ?1",
                [key],
                |r| {
                    Ok(PortMemory {
                        key: r.get(0)?,
                        path: r.get(1)?,
                        vid: r.get(2)?,
                        pid: r.get(3)?,
                        serial_number: r.get(4)?,
                        baud: r.get(5)?,
                        last_device: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                },
            )
            .optional()?)
    }

    /// Insert or replace by `key`; sets `updated_at` = now (the passed value is ignored).
    pub fn upsert_port_memory(&self, mem: &PortMemory) -> Result<()> {
        check_key(&mem.key)?;
        if mem.path.trim().is_empty() {
            return Err(AppError::invalid("port path is empty"));
        }
        validate::baud(mem.baud)?;
        self.conn().execute(
            "INSERT INTO port_memory
                 (key, path, vid, pid, serial_number, baud, last_device, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(key) DO UPDATE SET
                 path = excluded.path,
                 vid = excluded.vid,
                 pid = excluded.pid,
                 serial_number = excluded.serial_number,
                 baud = excluded.baud,
                 last_device = excluded.last_device,
                 updated_at = excluded.updated_at",
            params![
                mem.key,
                mem.path,
                mem.vid,
                mem.pid,
                mem.serial_number,
                mem.baud,
                mem.last_device,
                now_ms(),
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn mem(key: &str, baud: u32) -> PortMemory {
        PortMemory {
            key: key.into(),
            path: "/dev/cu.usbserial-A10K3XYZ".into(),
            vid: Some(0x0403),
            pid: Some(0xFFFF),
            serial_number: Some("A10K3XYZ".into()),
            baud,
            last_device: Some("core-sw1".into()),
            updated_at: 0,
        }
    }

    #[test]
    fn missing_key_is_none() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get_port_memory("A10K3XYZ").unwrap(), None);
    }

    #[test]
    fn upsert_inserts_with_now_and_round_trips() {
        let db = Db::open_in_memory().unwrap();
        let before = now_ms();
        db.upsert_port_memory(&mem("A10K3XYZ", 9600)).unwrap();
        let got = db.get_port_memory("A10K3XYZ").unwrap().unwrap();
        assert!(got.updated_at >= before, "updated_at is set by the store");
        assert_eq!(
            got,
            PortMemory {
                updated_at: got.updated_at,
                ..mem("A10K3XYZ", 9600)
            }
        );
        assert_eq!(got.pid, Some(0xFFFF), "full u16 range survives");
    }

    #[test]
    fn upsert_replaces_existing_row() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_port_memory(&mem("COM4", 9600)).unwrap();
        let replacement = PortMemory {
            path: "COM4".into(),
            vid: None,
            pid: None,
            serial_number: None,
            baud: 115_200,
            last_device: None,
            ..mem("COM4", 0)
        };
        db.upsert_port_memory(&replacement).unwrap();
        let got = db.get_port_memory("COM4").unwrap().unwrap();
        assert_eq!(got.baud, 115_200);
        assert_eq!(got.path, "COM4");
        assert_eq!(got.vid, None);
        assert_eq!(got.last_device, None);
        let rows: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM port_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn keys_are_exact() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_port_memory(&mem("abc", 9600)).unwrap();
        assert_eq!(db.get_port_memory("ABC").unwrap(), None);
    }

    #[test]
    fn validation() {
        let db = Db::open_in_memory().unwrap();
        let code = |m: PortMemory| db.upsert_port_memory(&m).unwrap_err().code;
        assert_eq!(code(mem("  ", 9600)), ErrorCode::Invalid);
        assert_eq!(code(mem("k", 0)), ErrorCode::Invalid);
        assert_eq!(code(mem("k", 4_000_001)), ErrorCode::Invalid);
        assert_eq!(
            code(PortMemory {
                path: " ".into(),
                ..mem("k", 9600)
            }),
            ErrorCode::Invalid
        );
        assert_eq!(db.get_port_memory("").unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(db.get_port_memory("k").unwrap(), None);
    }
}
