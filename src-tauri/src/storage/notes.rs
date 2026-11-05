//! Device notes (P3-T08). A note belongs to a device once the device is known (the same
//! notes whether you connect by SSH or console), until then to the saved session; when a
//! session's device is identified its notes move to the device. A note that starts with an
//! interface name (`Gi1/0/24: camera`, `GigabitEthernet 0/24: camera`) is linked to that
//! interface. Bodies are indexed with FTS5 for the session search.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

use super::{Db, now_ms};
use crate::device::iface;
use crate::error::{AppError, Result};

pub const MAX_NOTE_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub session_id: Option<i64>,
    pub device_id: Option<i64>,
    /// Normalized interface key (`gigabitethernet1/0/24`) for interface notes.
    pub interface_key: Option<String>,
    /// Display form of that interface (`Gi1/0/24`).
    pub interface_short: Option<String>,
    pub body: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A pinned note's first line and owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedNote {
    pub session_id: Option<i64>,
    pub device_id: Option<i64>,
    pub text: String,
}

/// A session search hit from note content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteHit {
    pub session_id: Option<i64>,
    pub device_id: Option<i64>,
}

const SELECT: &str = "SELECT id, session_id, device_id, interface_key, body, pinned, created_at, \
                      updated_at FROM notes";

fn row(r: &Row<'_>) -> rusqlite::Result<Note> {
    let key: Option<String> = r.get(3)?;
    Ok(Note {
        id: r.get(0)?,
        session_id: r.get(1)?,
        device_id: r.get(2)?,
        interface_short: key.as_deref().and_then(iface::parse).map(|i| i.short),
        interface_key: key,
        body: r.get(4)?,
        pinned: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

/// Text before the first `:` on the first line, if it names an interface.
pub fn interface_of(body: &str) -> Option<iface::Iface> {
    let first = body.lines().next()?;
    let (head, _) = first.split_once(':')?;
    iface::parse(head)
}

fn clean_body(body: &str) -> Result<String> {
    let body = body.trim();
    if body.is_empty() {
        return Err(AppError::invalid("empty note"));
    }
    if body.chars().count() > MAX_NOTE_CHARS {
        return Err(AppError::invalid(format!(
            "notes are limited to {MAX_NOTE_CHARS} characters"
        )));
    }
    Ok(body.replace('\0', ""))
}

fn fetch(conn: &Connection, id: i64) -> Result<Note> {
    conn.prepare_cached(&format!("{SELECT} WHERE id = ?1"))?
        .query_row([id], row)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("note {id}")))
}

/// The device a session is linked to (if any).
fn session_device(conn: &Connection, session_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT device_id FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

/// FTS5 query from free text: every word as a prefix term, quoted (no operator injection).
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| w.replace('"', "").replace(['ı', 'İ'], "i"))
        .filter(|w| !w.is_empty())
        .take(8)
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

impl Db {
    /// Notes of a tab: its device's (when known) plus its session's not yet moved ones.
    /// Pinned first, newest first.
    pub fn list_notes(&self, session_id: Option<i64>, device_id: Option<i64>) -> Result<Vec<Note>> {
        let conn = self.conn();
        let device = match (device_id, session_id) {
            (Some(d), _) => Some(d),
            (None, Some(s)) => session_device(&conn, s)?,
            (None, None) => None,
        };
        let mut stmt = conn.prepare_cached(&format!(
            "{SELECT} WHERE (?1 IS NOT NULL AND device_id = ?1)
                 OR (?2 IS NOT NULL AND session_id = ?2 AND device_id IS NULL)
             ORDER BY pinned DESC, created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![device, session_id], row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds a note to the device (if known) or the saved session.
    pub fn create_note(
        &self,
        session_id: Option<i64>,
        device_id: Option<i64>,
        body: &str,
    ) -> Result<Note> {
        let body = clean_body(body)?;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = match (device_id, session_id) {
            (Some(d), _) => Some(d),
            (None, Some(s)) => session_device(&tx, s)?,
            (None, None) => None,
        };
        if device.is_none() && session_id.is_none() {
            return Err(AppError::invalid(
                "a note needs a saved session or an identified device",
            ));
        }
        let key = interface_of(&body).map(|i| i.key);
        let now = now_ms();
        tx.execute(
            "INSERT INTO notes (session_id, device_id, interface_key, body, pinned, created_at,
                 updated_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            params![session_id, device, key, body, now],
        )?;
        let note = fetch(&tx, tx.last_insert_rowid())?;
        tx.commit()?;
        Ok(note)
    }

    pub fn update_note(&self, id: i64, body: &str) -> Result<Note> {
        let body = clean_body(body)?;
        let conn = self.conn();
        let key = interface_of(&body).map(|i| i.key);
        let changed = conn.execute(
            "UPDATE notes SET body = ?2, interface_key = ?3,
                 updated_at = MAX(updated_at + 1, ?4) WHERE id = ?1",
            params![id, body, key, now_ms()],
        )?;
        if changed == 0 {
            return Err(AppError::not_found(format!("note {id}")));
        }
        fetch(&conn, id)
    }

    pub fn pin_note(&self, id: i64, pinned: bool) -> Result<Note> {
        let conn = self.conn();
        if conn.execute(
            "UPDATE notes SET pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )? == 0
        {
            return Err(AppError::not_found(format!("note {id}")));
        }
        fetch(&conn, id)
    }

    pub fn delete_note(&self, id: i64) -> Result<()> {
        if self
            .conn()
            .execute("DELETE FROM notes WHERE id = ?1", [id])?
            == 0
        {
            return Err(AppError::not_found(format!("note {id}")));
        }
        Ok(())
    }

    /// First line of every pinned note with its owner (subtitle in the session list),
    /// newest first.
    pub fn pinned_notes(&self) -> Result<Vec<PinnedNote>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT session_id, device_id, body FROM notes WHERE pinned = 1
             ORDER BY updated_at DESC, id DESC LIMIT 2000",
        )?;
        let rows = stmt.query_map([], |r| {
            let body: String = r.get(2)?;
            Ok(PinnedNote {
                session_id: r.get(0)?,
                device_id: r.get(1)?,
                text: body
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sessions / devices whose notes match `text` (for the session search).
    pub fn search_notes(&self, text: &str) -> Result<Vec<NoteHit>> {
        let Some(q) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT n.session_id, n.device_id FROM notes_fts f
             JOIN notes n ON n.id = f.rowid WHERE notes_fts MATCH ?1 LIMIT 500",
        )?;
        let rows = stmt.query_map([q], |r| {
            Ok(NoteHit {
                session_id: r.get(0)?,
                device_id: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// All notes as Markdown (export): one section per device / session.
    pub fn notes_markdown(&self) -> Result<String> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT n.body, n.pinned, n.created_at, d.hostname, d.model, d.serial, s.name
             FROM notes n
             LEFT JOIN devices d ON d.id = n.device_id
             LEFT JOIN sessions s ON s.id = n.session_id
             ORDER BY COALESCE(d.hostname, s.name, ''), n.device_id, n.session_id,
                      n.pinned DESC, n.created_at",
        )?;
        let mut out = String::from("# KenShell notes\n");
        let mut last = String::new();
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        for r in rows {
            let (body, pinned, at, host, model, serial, session) = r?;
            let title = match (&host, &session) {
                (Some(h), _) => {
                    let extra: Vec<&str> = [model.as_deref(), serial.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect();
                    if extra.is_empty() {
                        h.clone()
                    } else {
                        format!("{h} ({})", extra.join(", "))
                    }
                }
                (None, Some(s)) => s.clone(),
                (None, None) => "—".to_owned(),
            };
            if title != last {
                out.push_str(&format!("\n## {title}\n"));
                last = title;
            }
            let when = chrono_like(at);
            out.push_str(&format!(
                "\n- {}{when}\n\n{}\n",
                if pinned { "📌 " } else { "" },
                body.lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        Ok(out)
    }

    /// When a session's device becomes known, its notes go with the device.
    pub(super) fn move_session_notes(
        conn: &Connection,
        session_id: i64,
        device_id: i64,
    ) -> Result<()> {
        conn.execute(
            "UPDATE notes SET device_id = ?2 WHERE session_id = ?1 AND device_id IS NULL",
            params![session_id, device_id],
        )?;
        Ok(())
    }
}

/// `YYYY-MM-DD HH:MM` (UTC) without a date crate.
fn chrono_like(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
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

    fn facts(serial: &str) -> DeviceFacts {
        DeviceFacts {
            hostname: Some("access-sw-02".into()),
            model: Some("S5750".into()),
            version: None,
            serial: Some(serial.into()),
            uptime: None,
        }
    }

    #[test]
    fn session_notes_move_to_the_device_and_show_on_every_path() {
        let db = Db::open_in_memory().unwrap();
        let console = session(&db, "console");
        let ssh = session(&db, "ssh");
        let n = db.create_note(Some(console), None, "Rack 3, U12").unwrap();
        assert_eq!(n.device_id, None);
        assert_eq!(db.list_notes(Some(ssh), None).unwrap().len(), 0);

        let dev = db
            .record_device(Some(console), "ruijie-rgos", &facts("G1"))
            .unwrap();
        db.record_device(Some(ssh), "ruijie-rgos", &facts("G1"))
            .unwrap();
        let via_ssh = db.list_notes(Some(ssh), None).unwrap();
        assert_eq!(via_ssh.len(), 1, "same device, other path");
        assert_eq!(via_ssh[0].device_id, Some(dev.id));
        // New notes from an identified tab go to the device.
        let m = db
            .create_note(Some(ssh), Some(dev.id), "core uplink")
            .unwrap();
        assert_eq!(m.device_id, Some(dev.id));
        assert_eq!(db.list_notes(Some(console), None).unwrap().len(), 2);
        assert!(db.create_note(None, None, "orphan").is_err());
    }

    #[test]
    fn interface_notes_pin_order_edit_and_delete() {
        let db = Db::open_in_memory().unwrap();
        let s = session(&db, "sw");
        let a = db
            .create_note(Some(s), None, "GigabitEthernet 0/24: Kamera")
            .unwrap();
        assert_eq!(a.interface_key.as_deref(), Some("gigabitethernet0/24"));
        assert_eq!(a.interface_short.as_deref(), Some("Gi0/24"));
        let b = db
            .create_note(Some(s), None, "just text: not an interface")
            .unwrap();
        assert_eq!(b.interface_key, None);
        db.pin_note(a.id, true).unwrap();
        let pinned = db.pinned_notes().unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].text, "GigabitEthernet 0/24: Kamera");
        assert_eq!(pinned[0].session_id, Some(s));
        let list = db.list_notes(Some(s), None).unwrap();
        assert_eq!(list[0].id, a.id, "pinned first");
        let a2 = db.update_note(a.id, "Gi0/25: Yazıcı").unwrap();
        assert_eq!(a2.interface_key.as_deref(), Some("gigabitethernet0/25"));
        assert!(a2.updated_at > a.updated_at);
        assert!(db.create_note(Some(s), None, "   ").is_err());
        db.delete_note(b.id).unwrap();
        assert!(db.delete_note(b.id).is_err());
        assert_eq!(db.list_notes(Some(s), None).unwrap().len(), 1);
    }

    #[test]
    fn search_is_accent_insensitive_prefix_and_safe() {
        let db = Db::open_in_memory().unwrap();
        let s = session(&db, "sw");
        db.create_note(Some(s), None, "Güvenlik kamerası NVR bağlantısı")
            .unwrap();
        for q in ["guvenlik", "kame", "NVR", "bağlantı"] {
            let hits = db.search_notes(q).unwrap();
            assert_eq!(hits.len(), 1, "{q}");
            assert_eq!(hits[0].session_id, Some(s));
        }
        assert!(db.search_notes("yazıcı").unwrap().is_empty());
        // FTS5 syntax in user input never errors.
        for q in ["\"", "AND OR NOT", "kam* ^", "a:b", ")("] {
            assert!(db.search_notes(q).is_ok(), "{q}");
        }
        assert!(db.search_notes("   ").unwrap().is_empty());
        // Edits are re-indexed.
        let id = db.list_notes(Some(s), None).unwrap()[0].id;
        db.update_note(id, "Yazıcı odası").unwrap();
        assert!(db.search_notes("kamera").unwrap().is_empty());
        assert_eq!(db.search_notes("yazici").unwrap().len(), 1);
    }

    #[test]
    fn markdown_export_groups_by_device() {
        let db = Db::open_in_memory().unwrap();
        let s = session(&db, "edge-01");
        db.create_note(Some(s), None, "line one\nline two").unwrap();
        let md = db.notes_markdown().unwrap();
        assert!(md.starts_with("# KenShell notes\n"));
        assert!(md.contains("## edge-01"));
        assert!(md.contains("  line one\n  line two"));
        assert_eq!(chrono_like(0), "1970-01-01 00:00 UTC");
        assert_eq!(chrono_like(1_793_870_000_000), "2026-11-05 09:13 UTC");
    }
}
