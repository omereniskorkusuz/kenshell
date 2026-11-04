//! Saved-session repository (P1-T05). Serial settings and tags are stored as JSON
//! columns (`serial_json`, `tags_json`) so new optional fields need no migration.

use std::collections::HashSet;

use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::de::DeserializeOwned;

use super::folders::require_folder;
use super::validate::{self, MAX_NAME_CHARS, ValidSession};
use super::{Db, now_ms};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Protocol, SessionInput, SessionRecord};

const SELECT_SESSION: &str = "SELECT id, folder_id, name, protocol, host, port, username, \
     serial_json, env_tag, tags_json, favorite, sort_order, created_at, updated_at, \
     last_connected_at, ssh_json, credential_profile_id, auto_commands_json FROM sessions";

/// `NULL` when there are no commands (keeps the common case compact).
fn auto_json(cmds: &[String]) -> Result<Option<String>> {
    if cmds.is_empty() {
        Ok(None)
    } else {
        to_json(&cmds).map(Some)
    }
}

fn require_profile(conn: &Connection, id: Option<i64>) -> Result<()> {
    let Some(id) = id else { return Ok(()) };
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM credential_profiles WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(AppError::not_found(format!("profile {id}")))
    }
}

fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Serial => "serial",
        Protocol::Ssh => "ssh",
        Protocol::Telnet => "telnet",
        Protocol::Rlogin => "rlogin",
        Protocol::Raw => "raw",
    }
}

fn parse_protocol(s: &str) -> Option<Protocol> {
    Some(match s {
        "serial" => Protocol::Serial,
        "ssh" => Protocol::Ssh,
        "telnet" => Protocol::Telnet,
        "rlogin" => Protocol::Rlogin,
        "raw" => Protocol::Raw,
        _ => return None,
    })
}

fn conversion_error(
    idx: usize,
    e: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(e))
}

fn json_col<T: DeserializeOwned>(r: &Row<'_>, idx: usize) -> rusqlite::Result<Option<T>> {
    let Some(text) = r.get::<_, Option<String>>(idx)? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| conversion_error(idx, e))
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| AppError::internal(format!("encode json: {e}")))
}

fn row_to_session(r: &Row<'_>) -> rusqlite::Result<SessionRecord> {
    let protocol: String = r.get(3)?;
    let protocol = parse_protocol(&protocol).ok_or_else(|| {
        conversion_error(
            3,
            std::io::Error::other(format!("unknown protocol `{protocol}`")),
        )
    })?;
    Ok(SessionRecord {
        id: r.get(0)?,
        folder_id: r.get(1)?,
        name: r.get(2)?,
        protocol,
        host: r.get(4)?,
        port: r.get(5)?,
        username: r.get(6)?,
        serial: json_col(r, 7)?,
        env_tag: r.get(8)?,
        tags: json_col(r, 9)?.unwrap_or_default(),
        favorite: r.get(10)?,
        sort_order: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
        last_connected_at: r.get(14)?,
        ssh: json_col(r, 15)?,
        credential_profile_id: r.get(16)?,
        auto_commands: json_col(r, 17)?.unwrap_or_default(),
    })
}

fn session_not_found(id: i64) -> AppError {
    AppError::not_found(format!("session {id}"))
}

fn fetch(conn: &Connection, id: i64) -> Result<SessionRecord> {
    conn.prepare_cached(&format!("{SELECT_SESSION} WHERE id = ?1"))?
        .query_row([id], row_to_session)
        .optional()?
        .ok_or_else(|| session_not_found(id))
}

/// `sort_order` that appends a session at the end of `folder`.
pub(super) fn next_session_order(conn: &Connection, folder: Option<i64>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM sessions WHERE folder_id IS ?1",
        [folder],
        |r| r.get(0),
    )?)
}

fn insert(conn: &Connection, v: &ValidSession, sort_order: i64, now: i64) -> Result<i64> {
    let serial_json = v.serial.as_ref().map(to_json).transpose()?;
    let ssh_json = v.ssh.as_ref().map(to_json).transpose()?;
    conn.execute(
        "INSERT INTO sessions (folder_id, name, protocol, host, port, username, serial_json,
             env_tag, tags_json, favorite, sort_order, created_at, updated_at, ssh_json,
             credential_profile_id, auto_commands_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14, ?15)",
        params![
            v.folder_id,
            v.name,
            protocol_str(v.protocol),
            v.host,
            v.port,
            v.username,
            serial_json,
            v.env_tag,
            to_json(&v.tags)?,
            v.favorite,
            sort_order,
            now,
            ssh_json,
            v.credential_profile_id,
            auto_json(&v.auto_commands)?,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// `"X (2)"` → `"X"` so duplicating a copy continues the numbering. Only small
/// counters (2–999) count as copy suffixes, so `"Lab (2024)"` stays intact.
fn strip_copy_suffix(name: &str) -> &str {
    let Some(inner) = name.strip_suffix(')') else {
        return name;
    };
    let Some(open) = inner.rfind(" (") else {
        return name;
    };
    let digits = &inner[open + 2..];
    let is_counter = (1..=3).contains(&digits.len())
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0')
        && digits != "1";
    let base = inner[..open].trim_end();
    if is_counter && !base.is_empty() {
        base
    } else {
        name
    }
}

/// `"{base} ({n})"`, truncating `base` so the result fits [`MAX_NAME_CHARS`].
fn copy_name(base: &str, n: u64) -> String {
    let suffix = format!(" ({n})");
    let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
    let base: String = base.chars().take(room).collect();
    format!("{}{suffix}", base.trim_end())
}

impl Db {
    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{SELECT_SESSION} ORDER BY folder_id, sort_order, id"
        ))?;
        let rows = stmt.query_map([], row_to_session)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_session(&self, id: i64) -> Result<SessionRecord> {
        fetch(&self.conn(), id)
    }

    /// Validates and inserts; the session is appended at the end of its folder.
    pub fn create_session(&self, input: &SessionInput) -> Result<SessionRecord> {
        let v = validate::session(input)?;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_folder(&tx, v.folder_id)?;
        require_profile(&tx, v.credential_profile_id)?;
        let sort_order = next_session_order(&tx, v.folder_id)?;
        let id = insert(&tx, &v, sort_order, now_ms())?;
        let record = fetch(&tx, id)?;
        tx.commit()?;
        Ok(record)
    }

    /// Replaces the user-editable fields. Keeps `id`, `created_at`,
    /// `last_connected_at` and the position (unless the folder changes, then the
    /// session is appended to the new folder). `updated_at` strictly increases.
    pub fn update_session(&self, id: i64, input: &SessionInput) -> Result<SessionRecord> {
        let v = validate::session(input)?;
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = fetch(&tx, id)?;
        require_folder(&tx, v.folder_id)?;
        require_profile(&tx, v.credential_profile_id)?;
        let sort_order = if existing.folder_id == v.folder_id {
            existing.sort_order
        } else {
            next_session_order(&tx, v.folder_id)?
        };
        let updated_at = now_ms().max(existing.updated_at + 1);
        let serial_json = v.serial.as_ref().map(to_json).transpose()?;
        let ssh_json = v.ssh.as_ref().map(to_json).transpose()?;
        tx.execute(
            "UPDATE sessions SET folder_id = ?2, name = ?3, protocol = ?4, host = ?5,
                 port = ?6, username = ?7, serial_json = ?8, env_tag = ?9, tags_json = ?10,
                 favorite = ?11, sort_order = ?12, updated_at = ?13, ssh_json = ?14,
                 credential_profile_id = ?15, auto_commands_json = ?16
             WHERE id = ?1",
            params![
                id,
                v.folder_id,
                v.name,
                protocol_str(v.protocol),
                v.host,
                v.port,
                v.username,
                serial_json,
                v.env_tag,
                to_json(&v.tags)?,
                v.favorite,
                sort_order,
                updated_at,
                ssh_json,
                v.credential_profile_id,
                auto_json(&v.auto_commands)?,
            ],
        )?;
        let record = fetch(&tx, id)?;
        tx.commit()?;
        Ok(record)
    }

    /// Moves a session into `folder_id` and places it right before `before_id`
    /// (`None` = at the end). Positions in that folder are renumbered 0..n.
    pub fn reorder_session(
        &self,
        id: i64,
        folder_id: Option<i64>,
        before_id: Option<i64>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        fetch(&tx, id)?;
        require_folder(&tx, folder_id)?;
        let mut ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM sessions WHERE folder_id IS ?1 AND id != ?2
                 ORDER BY sort_order, id",
            )?;
            stmt.query_map(params![folder_id, id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let at = before_id
            .and_then(|b| ids.iter().position(|x| *x == b))
            .unwrap_or(ids.len());
        ids.insert(at, id);
        {
            let mut upd =
                tx.prepare("UPDATE sessions SET folder_id = ?2, sort_order = ?3 WHERE id = ?1")?;
            for (i, sid) in ids.iter().enumerate() {
                upd.execute(params![sid, folder_id, i as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_session(&self, id: i64) -> Result<()> {
        let changed = self
            .conn()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        if changed == 0 {
            return Err(session_not_found(id));
        }
        Ok(())
    }

    /// Copy with name suffix " (2)", " (3)" … in the same folder (first free number),
    /// appended at the end of the folder. The copy has never been connected.
    pub fn duplicate_session(&self, id: i64) -> Result<SessionRecord> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let orig = fetch(&tx, id)?;

        let taken: HashSet<String> = {
            let mut stmt = tx.prepare("SELECT name FROM sessions WHERE folder_id IS ?1")?;
            stmt.query_map([orig.folder_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let base = strip_copy_suffix(&orig.name);
        // Terminates: at most `taken.len()` candidates can be taken.
        let name = (2..)
            .map(|n| copy_name(base, n))
            .find(|candidate| !taken.contains(candidate))
            .ok_or_else(|| AppError::with(ErrorCode::Internal, "no free copy name"))?;

        let copy = ValidSession {
            folder_id: orig.folder_id,
            name,
            protocol: orig.protocol,
            host: orig.host,
            port: orig.port,
            username: orig.username,
            serial: orig.serial,
            ssh: orig.ssh,
            credential_profile_id: orig.credential_profile_id,
            auto_commands: orig.auto_commands,
            env_tag: orig.env_tag,
            tags: orig.tags,
            favorite: orig.favorite,
        };
        let sort_order = next_session_order(&tx, copy.folder_id)?;
        let new_id = insert(&tx, &copy, sort_order, now_ms())?;
        let record = fetch(&tx, new_id)?;
        tx.commit()?;
        Ok(record)
    }

    /// Sets `last_connected_at` = now. Does not touch `updated_at` (not an edit).
    pub fn touch_session(&self, id: i64) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE sessions SET last_connected_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )?;
        if changed == 0 {
            return Err(session_not_found(id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FlowControl, Parity, SerialSettings};

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    fn ssh(name: &str, folder_id: Option<i64>) -> SessionInput {
        SessionInput {
            folder_id,
            name: name.into(),
            protocol: Protocol::Ssh,
            host: Some("core-sw1.lab".into()),
            port: Some(2222),
            username: Some("admin".into()),
            serial: None,
            env_tag: Some("Lab".into()),
            tags: vec!["core".into(), " dc1 ".into(), "CORE".into()],
            favorite: true,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
        }
    }

    #[test]
    fn ssh_options_round_trip_and_are_ssh_only() {
        let db = db();
        let mut input = ssh("legacy-box", None);
        input.ssh = Some(crate::model::SshOptions {
            legacy_algorithms: true,
            key_path: Some("  ~/.ssh/id_ed25519 ".into()),
            keepalive_secs: Some(15),
            use_agent: true,
        });
        let rec = db.create_session(&input).unwrap();
        let o = rec.ssh.clone().unwrap();
        assert!(o.legacy_algorithms);
        assert_eq!(o.key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(db.list_sessions().unwrap()[0].ssh, rec.ssh);
        let dup = db.duplicate_session(rec.id).unwrap();
        assert_eq!(dup.ssh, rec.ssh);

        let mut telnet = input.clone();
        telnet.protocol = Protocol::Telnet;
        let rec = db.update_session(rec.id, &telnet).unwrap();
        assert_eq!(rec.ssh, None, "non-SSH sessions keep no SSH options");
    }

    fn serial(name: &str) -> SessionInput {
        SessionInput {
            folder_id: None,
            name: name.into(),
            protocol: Protocol::Serial,
            host: None,
            port: None,
            username: None,
            serial: Some(SerialSettings {
                port: "/dev/cu.usbserial-A10K3XYZ".into(),
                baud: None,
                data_bits: 7,
                parity: Parity::Even,
                stop_bits: 2,
                flow_control: FlowControl::RtsCts,
                dtr: Some(false),
                rts: Some(true),
            }),
            env_tag: None,
            tags: vec![],
            favorite: false,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
        }
    }

    fn names_in(db: &Db, folder: Option<i64>) -> Vec<String> {
        db.list_sessions()
            .unwrap()
            .into_iter()
            .filter(|s| s.folder_id == folder)
            .map(|s| s.name)
            .collect()
    }

    #[test]
    fn create_normalizes_and_round_trips() {
        let db = db();
        let s = db.create_session(&ssh("  core-sw1 ", None)).unwrap();
        assert_eq!(s.name, "core-sw1");
        assert_eq!(s.protocol, Protocol::Ssh);
        assert_eq!(s.port, Some(2222));
        assert_eq!(s.env_tag.as_deref(), Some("lab"));
        assert_eq!(s.tags, vec!["core", "dc1"]);
        assert!(s.favorite);
        assert_eq!(s.created_at, s.updated_at);
        assert!(s.created_at > 0);
        assert_eq!(s.last_connected_at, None);
        assert_eq!(db.get_session(s.id).unwrap(), s);
    }

    #[test]
    fn serial_settings_json_round_trip() {
        let db = db();
        let input = serial("Console");
        let s = db.create_session(&input).unwrap();
        assert_eq!(s.serial, input.serial);
        assert_eq!(db.get_session(s.id).unwrap().serial, input.serial);
        let raw: String = db
            .conn()
            .query_row(
                "SELECT serial_json FROM sessions WHERE id = ?1",
                [s.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(raw.contains("\"flowControl\":\"rtscts\""), "{raw}");
    }

    #[test]
    fn unicode_tags_round_trip() {
        let db = db();
        let mut input = ssh("x", None);
        input.tags = vec!["çekirdek".into(), "日本".into(), "a\"b".into()];
        let s = db.create_session(&input).unwrap();
        assert_eq!(db.get_session(s.id).unwrap().tags, input.tags);
    }

    #[test]
    fn create_appends_per_folder() {
        let db = db();
        let f = db.create_folder("F", None).unwrap();
        let a = db.create_session(&ssh("a", None)).unwrap();
        let b = db.create_session(&ssh("b", Some(f.id))).unwrap();
        let c = db.create_session(&ssh("c", None)).unwrap();
        assert_eq!((a.sort_order, b.sort_order, c.sort_order), (0, 0, 1));
    }

    #[test]
    fn create_rejects_missing_folder_and_invalid_input() {
        let db = db();
        let err = db.create_session(&ssh("a", Some(5))).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        let mut bad = ssh("a", None);
        bad.host = None;
        assert_eq!(
            db.create_session(&bad).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert!(db.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn list_is_ordered_by_folder_then_sort_order() {
        let db = db();
        let f1 = db.create_folder("F1", None).unwrap();
        let f2 = db.create_folder("F2", None).unwrap();
        db.create_session(&ssh("f2-a", Some(f2.id))).unwrap();
        db.create_session(&ssh("root-a", None)).unwrap();
        db.create_session(&ssh("f1-a", Some(f1.id))).unwrap();
        db.create_session(&ssh("root-b", None)).unwrap();
        db.create_session(&ssh("f1-b", Some(f1.id))).unwrap();
        let names: Vec<String> = db
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["root-a", "root-b", "f1-a", "f1-b", "f2-a"]);
    }

    #[test]
    fn get_missing_is_not_found() {
        let err = db().get_session(42).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(err.detail.as_deref(), Some("session 42"));
    }

    #[test]
    fn update_keeps_identity_and_bumps_updated_at() {
        let db = db();
        db.create_session(&ssh("other", None)).unwrap();
        let s = db.create_session(&ssh("old", None)).unwrap();
        db.touch_session(s.id).unwrap();
        let touched = db.get_session(s.id).unwrap().last_connected_at;
        let mut input = serial("new");
        input.tags = vec!["edge".into()];
        let u = db.update_session(s.id, &input).unwrap();
        assert_eq!(u.id, s.id);
        assert_eq!(u.created_at, s.created_at);
        assert!(u.updated_at > s.updated_at);
        assert_eq!(u.sort_order, s.sort_order, "same folder keeps position");
        assert_eq!(u.last_connected_at, touched);
        assert_eq!(u.name, "new");
        assert_eq!(u.protocol, Protocol::Serial);
        assert_eq!(u.host, None);
        assert_eq!(u.tags, vec!["edge"]);
        assert_eq!(db.get_session(s.id).unwrap(), u);
        // Repeated updates within the same millisecond still increase.
        let u2 = db.update_session(s.id, &input).unwrap();
        assert!(u2.updated_at > u.updated_at);
    }

    #[test]
    fn update_moving_folder_appends() {
        let db = db();
        let f = db.create_folder("F", None).unwrap();
        db.create_session(&ssh("in-f", Some(f.id))).unwrap();
        let s = db.create_session(&ssh("s", None)).unwrap();
        let u = db.update_session(s.id, &ssh("s", Some(f.id))).unwrap();
        assert_eq!(u.folder_id, Some(f.id));
        assert_eq!(u.sort_order, 1);
    }

    #[test]
    fn update_errors_leave_record_untouched() {
        let db = db();
        let s = db.create_session(&ssh("s", None)).unwrap();
        assert_eq!(
            db.update_session(999, &ssh("x", None)).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            db.update_session(s.id, &ssh("x", Some(77)))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            db.update_session(s.id, &ssh("", None)).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(db.get_session(s.id).unwrap(), s);
    }

    #[test]
    fn delete() {
        let db = db();
        let s = db.create_session(&ssh("s", None)).unwrap();
        db.delete_session(s.id).unwrap();
        assert_eq!(db.get_session(s.id).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(
            db.delete_session(s.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn duplicate_uses_first_free_suffix_in_same_folder() {
        let db = db();
        let f = db.create_folder("F", None).unwrap();
        let orig = db.create_session(&ssh("X", Some(f.id))).unwrap();
        // A same-named session elsewhere does not block the name.
        db.create_session(&ssh("X (2)", None)).unwrap();

        let d2 = db.duplicate_session(orig.id).unwrap();
        let d3 = db.duplicate_session(orig.id).unwrap();
        assert_eq!(d2.name, "X (2)");
        assert_eq!(d3.name, "X (3)");
        // Duplicating a copy continues the numbering instead of "X (2) (2)".
        assert_eq!(db.duplicate_session(d2.id).unwrap().name, "X (4)");
        // Gaps are filled first.
        db.delete_session(d2.id).unwrap();
        assert_eq!(db.duplicate_session(orig.id).unwrap().name, "X (2)");
        assert_eq!(
            names_in(&db, Some(f.id)),
            vec!["X", "X (3)", "X (4)", "X (2)"]
        );
    }

    #[test]
    fn duplicate_copies_fields_but_not_history() {
        let db = db();
        let orig = db.create_session(&serial("Console")).unwrap();
        db.touch_session(orig.id).unwrap();
        let d = db.duplicate_session(orig.id).unwrap();
        assert_ne!(d.id, orig.id);
        assert_eq!(d.protocol, orig.protocol);
        assert_eq!(d.serial, orig.serial);
        assert_eq!(d.tags, orig.tags);
        assert_eq!(d.favorite, orig.favorite);
        assert_eq!(d.folder_id, orig.folder_id);
        assert_eq!(d.sort_order, orig.sort_order + 1);
        assert_eq!(d.last_connected_at, None);
        assert_eq!(
            db.duplicate_session(999).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn duplicate_respects_name_length_limit() {
        let db = db();
        let long = "n".repeat(MAX_NAME_CHARS);
        let orig = db.create_session(&ssh(&long, None)).unwrap();
        let d = db.duplicate_session(orig.id).unwrap();
        assert_eq!(d.name.chars().count(), MAX_NAME_CHARS);
        assert!(d.name.ends_with(" (2)"));
        let d3 = db.duplicate_session(d.id).unwrap();
        assert_eq!(d3.name.chars().count(), MAX_NAME_CHARS);
        assert!(d3.name.ends_with(" (3)"));
    }

    #[test]
    fn copy_suffix_parsing() {
        assert_eq!(strip_copy_suffix("X (2)"), "X");
        assert_eq!(strip_copy_suffix("X (999)"), "X");
        assert_eq!(strip_copy_suffix("Lab (2024)"), "Lab (2024)");
        assert_eq!(strip_copy_suffix("X (1)"), "X (1)");
        assert_eq!(strip_copy_suffix("X (02)"), "X (02)");
        assert_eq!(strip_copy_suffix("X (a)"), "X (a)");
        assert_eq!(strip_copy_suffix("X ()"), "X ()");
        assert_eq!(strip_copy_suffix("(2)"), "(2)");
        assert_eq!(strip_copy_suffix("X(2)"), "X(2)");
        assert_eq!(strip_copy_suffix("Ağ (3)"), "Ağ");
        assert_eq!(copy_name("Ağ", 12), "Ağ (12)");
    }

    #[test]
    fn touch_sets_last_connected_only() {
        let db = db();
        let s = db.create_session(&ssh("s", None)).unwrap();
        let before = now_ms();
        db.touch_session(s.id).unwrap();
        let t = db.get_session(s.id).unwrap();
        assert!(t.last_connected_at.unwrap() >= before);
        assert_eq!(t.updated_at, s.updated_at);
        assert_eq!(db.touch_session(999).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn corrupt_row_surfaces_db_error() {
        let db = db();
        let s = db.create_session(&serial("c")).unwrap();
        db.conn()
            .execute(
                "UPDATE sessions SET serial_json = '{not json' WHERE id = ?1",
                [s.id],
            )
            .unwrap();
        assert_eq!(db.get_session(s.id).unwrap_err().code, ErrorCode::Db);
        assert_eq!(db.list_sessions().unwrap_err().code, ErrorCode::Db);
    }

    #[test]
    fn reorder_places_before_target_and_moves_between_folders() {
        let db = db();
        let f = db.create_folder("LAB", None).unwrap();
        let a = db.create_session(&serial("a")).unwrap();
        let b = db.create_session(&serial("b")).unwrap();
        let c = db.create_session(&serial("c")).unwrap();
        let names = |db: &Db, folder: Option<i64>| {
            let mut v: Vec<_> = db
                .list_sessions()
                .unwrap()
                .into_iter()
                .filter(|s| s.folder_id == folder)
                .collect();
            v.sort_by_key(|s| s.sort_order);
            v.into_iter().map(|s| s.name).collect::<Vec<_>>()
        };
        db.reorder_session(c.id, None, Some(a.id)).unwrap();
        assert_eq!(names(&db, None), ["c", "a", "b"]);
        db.reorder_session(c.id, None, None).unwrap();
        assert_eq!(names(&db, None), ["a", "b", "c"]);
        db.reorder_session(b.id, Some(f.id), None).unwrap();
        assert_eq!(names(&db, None), ["a", "c"]);
        assert_eq!(names(&db, Some(f.id)), ["b"]);
        assert!(db.reorder_session(999, None, None).is_err());
        assert!(db.reorder_session(a.id, Some(999), None).is_err());
    }
}
