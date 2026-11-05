//! Command history for autocomplete (P3-T12). Like notes, a command belongs to the device
//! once it is known (the same history over SSH or console), until then to the saved
//! session, and moves to the device when the session's device is identified. At most
//! [`MAX_PER_OWNER`] commands per owner, most recently used first.
//!
//! Lines that look like they carry a secret (`password`, `secret`, `community`, `key` …)
//! are never stored; the frontend also only records lines the device echoed.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};

use super::{Db, now_ms};
use crate::error::{AppError, Result};

pub const MAX_PER_OWNER: i64 = 500;
pub const MAX_COMMAND_CHARS: usize = 300;

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)passw|secret|communit|key|psk|passphrase|token|credential|auth")
        .expect("secret pattern")
});

/// Whether `command` may be stored (not empty, no control characters, no secret words).
pub fn storable(command: &str) -> bool {
    let c = command.trim();
    !c.is_empty()
        && c.chars().count() <= MAX_COMMAND_CHARS
        && !c.chars().any(char::is_control)
        && !SECRET.is_match(c)
}

/// (device, session) owner of a tab's history: the device when known.
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
            "history needs a saved session or a known device",
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
    /// Most recently used first.
    pub fn list_history(
        &self,
        session_id: Option<i64>,
        device_id: Option<i64>,
    ) -> Result<Vec<String>> {
        let conn = self.conn();
        let (dev, sess) = owner(&conn, session_id, device_id)?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT command FROM command_history WHERE {OWNER} ORDER BY used_at DESC, id DESC LIMIT ?3"
        ))?;
        let rows = stmt
            .query_map(params![dev, sess, MAX_PER_OWNER], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// Records a command (ignored when it is not [`storable`]).
    pub fn add_history(
        &self,
        session_id: Option<i64>,
        device_id: Option<i64>,
        command: &str,
    ) -> Result<()> {
        let command = command.trim();
        if !storable(command) {
            return Ok(());
        }
        let conn = self.conn();
        let (dev, sess) = owner(&conn, session_id, device_id)?;
        let now = now_ms();
        let updated = conn.execute(
            &format!(
                "UPDATE command_history SET uses = uses + 1, used_at = ?4 WHERE {OWNER} AND command = ?3"
            ),
            params![dev, sess, command, now],
        )?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO command_history (device_id, session_id, command, uses, used_at)
                 VALUES (?1, ?2, ?3, 1, ?4)",
                params![dev, sess, command, now],
            )?;
            conn.execute(
                &format!(
                    "DELETE FROM command_history WHERE {OWNER} AND id NOT IN (
                        SELECT id FROM command_history WHERE {OWNER}
                        ORDER BY used_at DESC, id DESC LIMIT ?3)"
                ),
                params![dev, sess, MAX_PER_OWNER],
            )?;
        }
        Ok(())
    }

    pub fn clear_history(&self, session_id: Option<i64>, device_id: Option<i64>) -> Result<()> {
        let conn = self.conn();
        let (dev, sess) = owner(&conn, session_id, device_id)?;
        conn.execute(
            &format!("DELETE FROM command_history WHERE {OWNER}"),
            params![dev, sess],
        )?;
        Ok(())
    }

    /// When a session's device becomes known, its history joins the device's.
    pub(super) fn move_session_history(
        conn: &Connection,
        session_id: i64,
        device_id: i64,
    ) -> Result<()> {
        // Commands the device already has: keep the newer use, drop the session copy.
        conn.execute(
            "UPDATE command_history AS d SET
                 uses = d.uses + s.uses, used_at = MAX(d.used_at, s.used_at)
             FROM command_history AS s
             WHERE d.device_id = ?2 AND s.session_id = ?1 AND s.device_id IS NULL
               AND s.command = d.command",
            params![session_id, device_id],
        )?;
        conn.execute(
            "DELETE FROM command_history WHERE session_id = ?1 AND device_id IS NULL
               AND command IN (SELECT command FROM command_history WHERE device_id = ?2)",
            params![session_id, device_id],
        )?;
        conn.execute(
            "UPDATE command_history SET device_id = ?2, session_id = NULL
             WHERE session_id = ?1 AND device_id IS NULL",
            params![session_id, device_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::modes::DeviceFacts;
    use crate::model::Protocol;

    fn session(db: &Db, name: &str) -> i64 {
        let mut i = crate::import::blank_input(name, Protocol::Ssh);
        i.host = Some("10.0.0.1".into());
        db.create_session(&i).unwrap().id
    }

    #[test]
    fn secrets_and_junk_are_never_stored() {
        for bad in [
            "username admin privilege 15 secret 0 Abc123",
            "enable password cisco",
            "snmp-server community public RO",
            "crypto isakmp key s3cr3t address 0.0.0.0",
            "  ",
            "show\u{7}",
        ] {
            assert!(!storable(bad), "{bad}");
        }
        assert!(storable("show ip interface brief"));
        let db = Db::open_in_memory().unwrap();
        let s = session(&db, "a");
        db.add_history(Some(s), None, "enable secret x").unwrap();
        assert!(db.list_history(Some(s), None).unwrap().is_empty());
    }

    #[test]
    fn recent_first_dedup_and_moves_to_the_device() {
        let db = Db::open_in_memory().unwrap();
        let console = session(&db, "console");
        let ssh = session(&db, "ssh");
        db.add_history(Some(console), None, "show version").unwrap();
        db.add_history(Some(console), None, "show ip int brief")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        db.add_history(Some(console), None, "show version").unwrap();
        assert_eq!(
            db.list_history(Some(console), None).unwrap(),
            ["show version", "show ip int brief"]
        );
        assert!(db.list_history(Some(ssh), None).unwrap().is_empty());
        assert!(db.list_history(None, None).is_err());

        let facts = DeviceFacts {
            hostname: Some("sw1".into()),
            model: None,
            version: None,
            serial: Some("S1".into()),
            uptime: None,
        };
        let dev = db.record_device(Some(ssh), "cisco-ios", &facts).unwrap();
        db.add_history(None, Some(dev.id), "show version").unwrap();
        db.record_device(Some(console), "cisco-ios", &facts)
            .unwrap();
        // One history for the device, whichever way you connect; duplicates merged.
        let via_ssh = db.list_history(Some(ssh), None).unwrap();
        assert_eq!(via_ssh.len(), 2);
        assert_eq!(
            db.list_history(Some(console), Some(dev.id)).unwrap(),
            via_ssh
        );
        db.clear_history(None, Some(dev.id)).unwrap();
        assert!(db.list_history(Some(console), None).unwrap().is_empty());
    }

    #[test]
    fn capped_per_owner() {
        let db = Db::open_in_memory().unwrap();
        let s = session(&db, "a");
        for i in 0..(MAX_PER_OWNER + 20) {
            db.add_history(Some(s), None, &format!("show cmd {i}"))
                .unwrap();
        }
        let list = db.list_history(Some(s), None).unwrap();
        assert_eq!(list.len() as i64, MAX_PER_OWNER);
        assert_eq!(list[0], format!("show cmd {}", MAX_PER_OWNER + 19));
    }
}
