//! Secrets (P2-T14): passwords live ONLY in the OS keychain (macOS Keychain, Windows
//! Credential Manager, Linux keyutils) — never in SQLite, logs, errors or exports.
//! SQLite only knows *that* a session/profile exists; the keychain account name is
//! derived from its id.

pub mod backup;
pub mod blob;
pub mod vault;

use std::fmt;

use zeroize::Zeroizing;

use crate::error::{AppError, ErrorCode, Result};

/// Keychain "service" (macOS) / target prefix (Windows).
pub const SERVICE: &str = "app.kenshell.desktop";

pub fn session_account(id: i64) -> String {
    format!("session:{id}")
}

/// Removes a session's saved password. Called whenever a session is deleted **and** right
/// after one is created: SQLite may give a new session the id of a deleted one, and it
/// must never inherit that session's password (which would then go to another host).
/// Keychain errors are logged, not raised: the session operation itself succeeded.
pub fn forget_session(store: &dyn SecretStore, id: i64) {
    if let Err(e) = store.delete(&session_account(id)) {
        tracing::warn!(session = id, code = ?e.code, "could not remove saved password");
    }
}

pub fn profile_account(id: i64) -> String {
    format!("profile:{id}")
}

/// A secret string that is wiped from memory when dropped and never printed.
pub type Secret = Zeroizing<String>;

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<Secret>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    /// Idempotent.
    fn delete(&self, account: &str) -> Result<()>;
}

/// The OS keychain.
pub struct Keychain;

fn keyring_error(e: keyring::Error) -> AppError {
    // keyring errors never contain the secret itself.
    AppError::with(ErrorCode::Io, format!("keychain: {e}"))
}

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(keyring_error)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(Zeroizing::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(e)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|e| e.set_password(secret))
            .map_err(keyring_error)
    }

    fn delete(&self, account: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(keyring_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(e)),
        }
    }
}

/// Keychain accounts read again and again by background work (the topology map reads the
/// SNMP credentials for every device, every refresh). macOS asks the user each time an
/// app it does not trust yet reads an item, so these are read once per run and kept in
/// memory; session passwords are not (they are read once per connect anyway).
fn cached_account(account: &str) -> bool {
    account.starts_with("snmp-")
}

/// A store that remembers [`cached_account`] values (and their absence) for the run.
pub struct Cached<S> {
    inner: S,
    memory: std::sync::Mutex<std::collections::HashMap<String, Option<Secret>>>,
}

impl<S: SecretStore> Cached<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            memory: std::sync::Mutex::default(),
        }
    }

    fn memory(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Option<Secret>>> {
        self.memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<S: SecretStore> SecretStore for Cached<S> {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        if !cached_account(account) {
            return self.inner.get(account);
        }
        if let Some(known) = self.memory().get(account) {
            return Ok(known.clone());
        }
        // An error (e.g. the user denied access) is not remembered: asked again next time.
        let value = self.inner.get(account)?;
        self.memory().insert(account.to_owned(), value.clone());
        Ok(value)
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.inner.set(account, secret)?;
        if cached_account(account) {
            self.memory()
                .insert(account.to_owned(), Some(Zeroizing::new(secret.to_owned())));
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.inner.delete(account)?;
        if cached_account(account) {
            self.memory().insert(account.to_owned(), None);
        }
        Ok(())
    }
}

/// In-memory store: tests, and self-check runs (`bench/check-topology.mjs`), which must
/// never touch the user's keychain.
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<()> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// Saved login for a session (resolved from its credential profile or its own
/// keychain entry). `Debug` never shows the password.
#[derive(Clone, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<Secret>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts reads that reach the underlying store.
    #[derive(Default)]
    struct Counting(MemoryStore, std::sync::atomic::AtomicUsize);
    impl SecretStore for Counting {
        fn get(&self, account: &str) -> Result<Option<Secret>> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.0.get(account)
        }
        fn set(&self, account: &str, secret: &str) -> Result<()> {
            self.0.set(account, secret)
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.0.delete(account)
        }
    }

    #[test]
    fn snmp_credentials_are_read_from_the_keychain_once_per_run() {
        let c = Cached::new(Counting::default());
        let reads = || c.inner.1.load(std::sync::atomic::Ordering::Relaxed);
        for _ in 0..5 {
            assert!(c.get("snmp-v3").unwrap().is_none());
        }
        assert_eq!(reads(), 1, "absence is remembered too");
        c.set("snmp-v3", "x").unwrap();
        assert_eq!(
            c.get("snmp-v3").unwrap().as_deref().map(String::as_str),
            Some("x")
        );
        c.delete("snmp-v3").unwrap();
        assert!(c.get("snmp-v3").unwrap().is_none());
        assert_eq!(reads(), 1);
        // Session passwords always go to the keychain.
        c.set("session:1", "p").unwrap();
        c.get("session:1").unwrap();
        c.get("session:1").unwrap();
        assert_eq!(reads(), 3);
    }

    #[test]
    fn debug_and_errors_never_show_the_password() {
        let c = Credentials {
            username: Some("admin".into()),
            password: Some(Zeroizing::new("Sup3rS3cret!".into())),
        };
        let shown = format!("{c:?}");
        assert!(
            shown.contains("admin") && !shown.contains("Sup3rS3cret"),
            "{shown}"
        );
    }

    #[test]
    fn a_new_session_never_inherits_a_deleted_sessions_password() {
        use crate::model::Protocol;
        let db = crate::storage::Db::open_in_memory().unwrap();
        let store = MemoryStore::default();
        let mut input = crate::import::blank_input("old-core", Protocol::Ssh);
        input.host = Some("10.0.0.1".into());
        let old = db.create_session(&input).unwrap();
        store.set(&session_account(old.id), "old-secret").unwrap();
        db.delete_session(old.id).unwrap();
        forget_session(&store, old.id);

        input.name = "new-box".into();
        input.host = Some("192.0.2.9".into());
        let new = db.create_session(&input).unwrap();
        assert_eq!(
            new.id, old.id,
            "SQLite reuses the id of the deleted last row"
        );
        assert!(store.get(&session_account(new.id)).unwrap().is_none());
        forget_session(&store, new.id); // idempotent
    }

    #[test]
    fn memory_store_round_trip_and_idempotent_delete() {
        let s = MemoryStore::default();
        assert!(s.get("session:1").unwrap().is_none());
        s.set("session:1", "pw").unwrap();
        assert_eq!(s.get("session:1").unwrap().unwrap().as_str(), "pw");
        s.delete("session:1").unwrap();
        s.delete("session:1").unwrap();
        assert!(s.get("session:1").unwrap().is_none());
    }

    #[test]
    #[ignore = "touches the real OS keychain"]
    fn real_keychain_round_trip() {
        let k = Keychain;
        let acc = "test:kenshell-roundtrip";
        k.set(acc, "x").unwrap();
        assert_eq!(k.get(acc).unwrap().unwrap().as_str(), "x");
        k.delete(acc).unwrap();
        assert!(k.get(acc).unwrap().is_none());
    }
}
