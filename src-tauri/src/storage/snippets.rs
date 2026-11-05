//! Snippet library (P3-T15): the user's own command snippets, in folders, optionally for one
//! vendor family. The read-only starter sets (Cisco, Ruijie) ship with the frontend
//! (`src/data/snippets.json`) and can be copied here to edit.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Db, now_ms};
use crate::error::{AppError, Result};

pub const MAX_BODY_CHARS: usize = 20_000;
pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_FOLDER_CHARS: usize = 100;
/// Vendor families a snippet can be limited to (as in `src/data/modes.json`).
pub const FAMILIES: &[&str] = &[
    "cisco", "ruijie", "juniper", "alcatel", "aruba", "huawei", "fortinet",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: i64,
    /// `Access/Ports` style path; empty = top level.
    pub folder: String,
    pub name: String,
    pub body: String,
    /// Vendor family, or none = any device.
    pub vendor: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetInput {
    pub folder: String,
    pub name: String,
    pub body: String,
    pub vendor: Option<String>,
}

struct Clean {
    folder: String,
    name: String,
    body: String,
    vendor: Option<String>,
}

fn clean(i: &SnippetInput) -> Result<Clean> {
    let name = i.name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(AppError::invalid(
            "snippet name: 1–100 characters, one line",
        ));
    }
    let folder = i
        .folder
        .split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if folder.chars().count() > MAX_FOLDER_CHARS || folder.chars().any(char::is_control) {
        return Err(AppError::invalid(
            "snippet folder: up to 100 characters, one line",
        ));
    }
    let body = i.body.replace("\r\n", "\n").replace('\r', "\n");
    let body = body.trim_end().to_owned();
    if body.trim().is_empty() || body.chars().count() > MAX_BODY_CHARS {
        return Err(AppError::invalid("snippet body: 1–20000 characters"));
    }
    if body
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(AppError::invalid("snippet body: no control characters"));
    }
    let vendor = match i.vendor.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(v) if FAMILIES.contains(&v) => Some(v.to_owned()),
        Some(v) => return Err(AppError::invalid(format!("unknown vendor family {v}"))),
    };
    Ok(Clean {
        folder,
        name: name.to_owned(),
        body,
        vendor,
    })
}

const SELECT: &str = "SELECT id, folder, name, body, vendor, created_at, updated_at FROM snippets";

fn row(r: &Row<'_>) -> rusqlite::Result<Snippet> {
    Ok(Snippet {
        id: r.get(0)?,
        folder: r.get(1)?,
        name: r.get(2)?,
        body: r.get(3)?,
        vendor: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

fn fetch(conn: &Connection, id: i64) -> Result<Snippet> {
    conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)
        .optional()?
        .ok_or_else(|| AppError::not_found(format!("snippet {id}")))
}

impl Db {
    /// By folder, then name (case-insensitive).
    pub fn list_snippets(&self) -> Result<Vec<Snippet>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{SELECT} ORDER BY folder COLLATE NOCASE, name COLLATE NOCASE, id"
        ))?;
        let rows = stmt
            .query_map([], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn create_snippet(&self, input: &SnippetInput) -> Result<Snippet> {
        let c = clean(input)?;
        let conn = self.conn();
        let now = now_ms();
        conn.execute(
            "INSERT INTO snippets (folder, name, body, vendor, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![c.folder, c.name, c.body, c.vendor, now],
        )?;
        fetch(&conn, conn.last_insert_rowid())
    }

    pub fn update_snippet(&self, id: i64, input: &SnippetInput) -> Result<Snippet> {
        let c = clean(input)?;
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE snippets SET folder = ?2, name = ?3, body = ?4, vendor = ?5,
                 updated_at = MAX(updated_at + 1, ?6) WHERE id = ?1",
            params![id, c.folder, c.name, c.body, c.vendor, now_ms()],
        )?;
        if changed == 0 {
            return Err(AppError::not_found(format!("snippet {id}")));
        }
        fetch(&conn, id)
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        if self
            .conn()
            .execute("DELETE FROM snippets WHERE id = ?1", [id])?
            == 0
        {
            return Err(AppError::not_found(format!("snippet {id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(folder: &str, name: &str, body: &str, vendor: Option<&str>) -> SnippetInput {
        SnippetInput {
            folder: folder.into(),
            name: name.into(),
            body: body.into(),
            vendor: vendor.map(Into::into),
        }
    }

    #[test]
    fn crud_order_and_normalisation() {
        let db = Db::open_in_memory().unwrap();
        let b = db
            .create_snippet(&input(
                " Ports / Access ",
                "Access port",
                "conf t\r\nint {{if}}\r\n\n",
                Some("cisco"),
            ))
            .unwrap();
        assert_eq!(b.folder, "Ports/Access");
        assert_eq!(b.body, "conf t\nint {{if}}");
        let a = db
            .create_snippet(&input("", "backup", "copy run start", None))
            .unwrap();
        let names: Vec<_> = db
            .list_snippets()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            names,
            ["backup", "Access port"],
            "top level first, then folders"
        );
        let u = db
            .update_snippet(a.id, &input("Ops", "Backup", "wr mem", Some("ruijie")))
            .unwrap();
        assert_eq!(
            (u.folder.as_str(), u.vendor.as_deref()),
            ("Ops", Some("ruijie"))
        );
        assert!(u.updated_at > a.updated_at || u.updated_at == a.updated_at + 1);
        db.delete_snippet(a.id).unwrap();
        assert_eq!(
            db.delete_snippet(a.id).unwrap_err().code,
            crate::error::ErrorCode::NotFound
        );
        assert_eq!(db.list_snippets().unwrap().len(), 1);
    }

    #[test]
    fn invalid_input_is_rejected() {
        let db = Db::open_in_memory().unwrap();
        for bad in [
            input("", " ", "x", None),
            input("", "a\nb", "x", None),
            input("", "ok", "   ", None),
            input("", "ok", "show\u{1b}[A", None),
            input("", "ok", "x", Some("acme")),
            input(&"f".repeat(101), "ok", "x", None),
        ] {
            assert!(db.create_snippet(&bad).is_err(), "{:?}", bad.name);
        }
        assert!(
            db.create_snippet(&input("", "tabs ok", "a\tb", None))
                .is_ok()
        );
    }
}
