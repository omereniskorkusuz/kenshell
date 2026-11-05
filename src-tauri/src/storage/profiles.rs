//! Credential profiles (P2-T14): a name + user name shared by many sessions. The
//! password is in the OS keychain under `profile:<id>`, never here.

use rusqlite::{OptionalExtension, params};

use super::{Db, now_ms, validate};
use crate::error::{AppError, Result};
use crate::model::CredentialProfile;

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CredentialProfile> {
    Ok(CredentialProfile {
        id: r.get(0)?,
        name: r.get(1)?,
        username: r.get(2)?,
        has_password: false,
    })
}

fn clean_user(u: Option<&str>) -> Result<Option<String>> {
    let u = u.map(str::trim).filter(|u| !u.is_empty());
    if let Some(u) = u
        && (u.len() > 255 || u.chars().any(char::is_control))
    {
        return Err(AppError::invalid("user name is invalid"));
    }
    Ok(u.map(str::to_owned))
}

impl Db {
    pub fn list_profiles(&self) -> Result<Vec<CredentialProfile>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, username FROM credential_profiles ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = stmt
            .query_map([], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_profile(&self, id: i64) -> Result<CredentialProfile> {
        self.conn()
            .query_row(
                "SELECT id, name, username FROM credential_profiles WHERE id = ?1",
                [id],
                row,
            )
            .optional()?
            .ok_or_else(|| AppError::not_found(format!("profile {id}")))
    }

    pub fn create_profile(&self, name: &str, username: Option<&str>) -> Result<CredentialProfile> {
        let name = validate::name(name, "profile")?;
        let username = clean_user(username)?;
        let now = now_ms();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO credential_profiles (name, username, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![name, username, now],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        self.get_profile(id)
    }

    pub fn update_profile(
        &self,
        id: i64,
        name: &str,
        username: Option<&str>,
    ) -> Result<CredentialProfile> {
        let name = validate::name(name, "profile")?;
        let username = clean_user(username)?;
        let n = self.conn().execute(
            "UPDATE credential_profiles SET name = ?2, username = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, name, username, now_ms()],
        )?;
        if n == 0 {
            return Err(AppError::not_found(format!("profile {id}")));
        }
        self.get_profile(id)
    }

    /// Sessions using it fall back to their own login (`ON DELETE SET NULL`).
    pub fn delete_profile(&self, id: i64) -> Result<()> {
        let n = self
            .conn()
            .execute("DELETE FROM credential_profiles WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(AppError::not_found(format!("profile {id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Protocol, SessionInput};

    #[test]
    fn profile_crud_and_session_link_is_cleared_on_delete() {
        let db = Db::open_in_memory().unwrap();
        let p = db.create_profile("  NOC admin ", Some(" admin ")).unwrap();
        assert_eq!(
            (p.name.as_str(), p.username.as_deref()),
            ("NOC admin", Some("admin"))
        );
        let s = db
            .create_session(&SessionInput {
                folder_id: None,
                name: "core".into(),
                protocol: Protocol::Ssh,
                host: Some("10.0.0.1".into()),
                port: None,
                username: None,
                serial: None,
                env_tag: None,
                tags: vec![],
                favorite: false,
                ssh: None,
                credential_profile_id: Some(p.id),
                auto_commands: vec![],
                identify: true,
            })
            .unwrap();
        assert_eq!(s.credential_profile_id, Some(p.id));
        let p = db.update_profile(p.id, "NOC", None).unwrap();
        assert_eq!(p.username, None);
        assert_eq!(db.list_profiles().unwrap().len(), 1);
        db.delete_profile(p.id).unwrap();
        assert_eq!(db.get_session(s.id).unwrap().credential_profile_id, None);
        assert!(db.delete_profile(p.id).is_err());
        assert!(db.create_profile(" ", None).is_err());
    }
}
