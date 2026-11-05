//! Portable encrypted backup (P2-T15): library + saved passwords in one vault file.
//!
//! What goes in: folders, sessions (all settings), credential profiles, SSH known
//! hosts, and the keychain passwords of sessions and profiles. Private key *files*
//! are referenced by path only (Known Issues). The JSON lives only in memory and only
//! ever reaches the disk encrypted ([`super::vault`]).
//!
//! Restore recreates the folder tree at the top level, skips sessions that already
//! exist (same name in the same folder), reuses profiles with the same name and never
//! overwrites a password that is already in the keychain.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{SecretStore, profile_account, session_account, vault};
use crate::error::{AppError, Result};
use crate::import::{self, ImportReport, Imported};
use crate::model::{Folder, KnownHost, SessionInput, SessionRecord};
use crate::storage::Db;

const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileEntry {
    id: i64,
    name: String,
    username: Option<String>,
    password: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionEntry {
    folders: Vec<String>,
    session: SessionInput,
    password: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    format: u32,
    created_at: i64,
    /// Every folder path, so empty folders come back too.
    folders: Vec<Vec<String>>,
    profiles: Vec<ProfileEntry>,
    sessions: Vec<SessionEntry>,
    known_hosts: Vec<KnownHost>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        for p in &mut self.profiles {
            p.password.zeroize();
        }
        for s in &mut self.sessions {
            s.password.zeroize();
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub sessions: ImportReport,
    pub profiles_created: usize,
    pub passwords: usize,
    pub known_hosts: usize,
}

/// Folder names from the top down to `id`.
fn folder_parts(folders: &[Folder], mut id: Option<i64>) -> Vec<String> {
    let mut parts = Vec::new();
    while let Some(fid) = id {
        let Some(f) = folders.iter().find(|f| f.id == fid) else {
            break;
        };
        if parts.len() > 64 {
            break; // cycle guard
        }
        parts.push(f.name.clone());
        id = f.parent_id;
    }
    parts.reverse();
    parts
}

fn input_of(s: &SessionRecord) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: s.name.clone(),
        protocol: s.protocol,
        host: s.host.clone(),
        port: s.port,
        username: s.username.clone(),
        serial: s.serial.clone(),
        env_tag: s.env_tag.clone(),
        tags: s.tags.clone(),
        favorite: s.favorite,
        ssh: s.ssh.clone(),
        credential_profile_id: s.credential_profile_id,
        auto_commands: s.auto_commands.clone(),
        identify: s.identify,
    }
}

fn secret(store: &dyn SecretStore, account: &str) -> Result<Option<String>> {
    Ok(store.get(account)?.map(|s| String::clone(&s)))
}

fn known_hosts(db: &Db) -> Result<Vec<KnownHost>> {
    db.list_known_hosts()
}

/// Builds and encrypts the backup.
pub fn export(db: &Db, store: &dyn SecretStore, password: &str, now: i64) -> Result<Vec<u8>> {
    export_with(db, store, password, now, vault::DEFAULT_COST)
}

pub(crate) fn export_with(
    db: &Db,
    store: &dyn SecretStore,
    password: &str,
    now: i64,
    cost: vault::Cost,
) -> Result<Vec<u8>> {
    if password.chars().count() < vault::MIN_PASSWORD_CHARS {
        // Fail before touching the keychain (it may show OS prompts).
        return Err(AppError::invalid(format!(
            "master password needs at least {} characters",
            vault::MIN_PASSWORD_CHARS
        )));
    }
    let folders = db.list_folders()?;
    let mut payload = Payload {
        format: FORMAT,
        created_at: now,
        folders: folders
            .iter()
            .map(|f| folder_parts(&folders, Some(f.id)))
            .collect(),
        profiles: Vec::new(),
        sessions: Vec::new(),
        known_hosts: known_hosts(db)?,
    };
    for p in db.list_profiles()? {
        payload.profiles.push(ProfileEntry {
            password: secret(store, &profile_account(p.id))?,
            id: p.id,
            name: p.name,
            username: p.username,
        });
    }
    for s in db.list_sessions()? {
        payload.sessions.push(SessionEntry {
            folders: folder_parts(&folders, s.folder_id),
            session: input_of(&s),
            password: secret(store, &session_account(s.id))?,
        });
    }
    let json = Zeroizing::new(
        serde_json::to_vec(&payload).map_err(|e| AppError::internal(e.to_string()))?,
    );
    vault::seal(password, &json, cost)
}

/// Decrypts `data` and merges it into the library and keychain.
pub fn restore(
    db: &Db,
    store: &dyn SecretStore,
    password: &str,
    data: &[u8],
) -> Result<RestoreReport> {
    let json = vault::open(password, data)?;
    let payload: Payload = serde_json::from_slice(&json)
        .map_err(|e| AppError::invalid(format!("backup content: {e}")))?;
    if payload.format != FORMAT {
        return Err(AppError::invalid(format!(
            "backup format {} is newer than this version",
            payload.format
        )));
    }
    let mut report = RestoreReport::default();

    // Folders first (empty ones included); sessions then find them by path.
    for path in &payload.folders {
        let mut parent = None;
        for name in path {
            match import::find_or_create_folder(db, parent, name) {
                Ok(id) => parent = Some(id),
                Err(e) => {
                    report.sessions.errors.push(format!("{name}: {e}"));
                    break;
                }
            }
        }
    }

    // Profiles: same name → reuse; keep an existing keychain password.
    let mut profile_ids: HashMap<i64, i64> = HashMap::new();
    let existing = db.list_profiles()?;
    for p in &payload.profiles {
        let id = match existing
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(p.name.trim()))
        {
            Some(e) => e.id,
            None => match db.create_profile(&p.name, p.username.as_deref()) {
                Ok(c) => {
                    report.profiles_created += 1;
                    c.id
                }
                Err(e) => {
                    report.sessions.errors.push(format!("{}: {e}", p.name));
                    continue;
                }
            },
        };
        profile_ids.insert(p.id, id);
        if let Some(pw) = p.password.as_deref()
            && store.get(&profile_account(id))?.is_none()
        {
            store.set(&profile_account(id), pw)?;
            report.passwords += 1;
        }
    }

    let items: Vec<Imported> = payload
        .sessions
        .iter()
        .map(|s| {
            let mut input = s.session.clone();
            input.credential_profile_id = input
                .credential_profile_id
                .and_then(|old| profile_ids.get(&old).copied());
            Imported {
                folders: s.folders.clone(),
                input,
            }
        })
        .collect();
    let mut created: Vec<(usize, i64)> = Vec::new();
    report.sessions = import::apply_with(
        db,
        None,
        items,
        std::mem::take(&mut report.sessions),
        |i, s| created.push((i, s.id)),
    );
    for (i, id) in created {
        // A fresh session: drop anything a deleted session with the same id left behind.
        super::forget_session(store, id);
        if let Some(pw) = payload.sessions[i].password.as_deref() {
            store.set(&session_account(id), pw)?;
            report.passwords += 1;
        }
    }

    for k in &payload.known_hosts {
        if db.get_known_host(&k.host, k.port)?.is_none() {
            db.save_known_host(k)?;
            report.known_hosts += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::model::{Protocol, SshOptions};
    use crate::secrets::MemoryStore;

    const FAST: vault::Cost = vault::Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    const PW: &str = "master-pass-1";

    fn session(name: &str, folder: Option<i64>, profile: Option<i64>) -> SessionInput {
        let mut i = import::blank_input(name, Protocol::Ssh);
        i.host = Some("10.0.0.1".into());
        i.folder_id = folder;
        i.credential_profile_id = profile;
        i.ssh = Some(SshOptions {
            legacy_algorithms: true,
            ..SshOptions::default()
        });
        i
    }

    /// Library: DC1/Core/{core-sw1 (own password)}, DC1/{edge (profile "ops")},
    /// empty folder LAB, one known host.
    fn source() -> (Db, MemoryStore) {
        let db = Db::open_in_memory().unwrap();
        let store = MemoryStore::default();
        let dc1 = db.create_folder("DC1", None).unwrap();
        let core = db.create_folder("Core", Some(dc1.id)).unwrap();
        db.create_folder("LAB", None).unwrap();
        let ops = db.create_profile("ops", Some("admin")).unwrap();
        store
            .set(&profile_account(ops.id), "profile-secret")
            .unwrap();
        let s1 = db
            .create_session(&session("core-sw1", Some(core.id), None))
            .unwrap();
        store
            .set(&session_account(s1.id), "session-secret")
            .unwrap();
        db.create_session(&session("edge", Some(dc1.id), Some(ops.id)))
            .unwrap();
        db.save_known_host(&KnownHost {
            host: "10.0.0.1".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAAC3Nza".into(),
            fingerprint: "SHA256:x".into(),
            added_at: 0,
        })
        .unwrap();
        (db, store)
    }

    #[test]
    fn round_trip_into_an_empty_install() {
        let (db, store) = source();
        let file = export_with(&db, &store, PW, 1, FAST).unwrap();
        for needle in ["session-secret", "profile-secret", "core-sw1"] {
            assert!(
                !file.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} must not be readable in the file"
            );
        }

        let (db2, store2) = (Db::open_in_memory().unwrap(), MemoryStore::default());
        let r = restore(&db2, &store2, PW, &file).unwrap();
        assert_eq!(r.sessions.created, 2);
        assert_eq!((r.profiles_created, r.passwords, r.known_hosts), (1, 2, 1));

        let folders = db2.list_folders().unwrap();
        assert!(
            folders.iter().any(|f| f.name == "LAB"),
            "empty folder restored"
        );
        let sessions = db2.list_sessions().unwrap();
        let core = sessions.iter().find(|s| s.name == "core-sw1").unwrap();
        assert_eq!(folder_parts(&folders, core.folder_id), ["DC1", "Core"]);
        assert!(core.ssh.as_ref().unwrap().legacy_algorithms);
        assert_eq!(
            store2
                .get(&session_account(core.id))
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("session-secret")
        );
        let edge = sessions.iter().find(|s| s.name == "edge").unwrap();
        let ops = db2
            .get_profile(edge.credential_profile_id.unwrap())
            .unwrap();
        assert_eq!(
            (ops.name.as_str(), ops.username.as_deref()),
            ("ops", Some("admin"))
        );
        assert_eq!(
            store2
                .get(&profile_account(ops.id))
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("profile-secret")
        );
        assert!(db2.get_known_host("10.0.0.1", 22).unwrap().is_some());
    }

    #[test]
    fn restoring_twice_changes_nothing_and_keeps_existing_passwords() {
        let (db, store) = source();
        let file = export_with(&db, &store, PW, 1, FAST).unwrap();
        // Change a password after the backup: restore must not overwrite it.
        let ops = db.list_profiles().unwrap()[0].id;
        store.set(&profile_account(ops), "newer").unwrap();
        let r = restore(&db, &store, PW, &file).unwrap();
        assert_eq!((r.sessions.created, r.sessions.skipped), (0, 2));
        assert_eq!((r.profiles_created, r.passwords, r.known_hosts), (0, 0, 0));
        assert_eq!(db.list_sessions().unwrap().len(), 2);
        assert_eq!(db.list_folders().unwrap().len(), 3);
        assert_eq!(
            store
                .get(&profile_account(ops))
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("newer")
        );
    }

    #[test]
    fn wrong_password_touches_nothing() {
        let (db, store) = source();
        let file = export_with(&db, &store, PW, 1, FAST).unwrap();
        let (db2, store2) = (Db::open_in_memory().unwrap(), MemoryStore::default());
        let e = restore(&db2, &store2, "not-the-password", &file).unwrap_err();
        assert_eq!(e.code, ErrorCode::BadPassword);
        assert!(db2.list_sessions().unwrap().is_empty());
        assert!(db2.list_folders().unwrap().is_empty());
        assert_eq!(
            export_with(&db, &store, "short", 1, FAST).unwrap_err().code,
            ErrorCode::Invalid
        );
    }
}
