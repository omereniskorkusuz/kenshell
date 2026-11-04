//! Secrets (P2-T14): passwords live ONLY in the OS keychain (macOS Keychain, Windows
//! Credential Manager, Linux keyutils) — never in SQLite, logs, errors or exports.
//! SQLite only knows *that* a session/profile exists; the keychain account name is
//! derived from its id.

use std::fmt;

use zeroize::Zeroizing;

use crate::error::{AppError, ErrorCode, Result};

/// Keychain "service" (macOS) / target prefix (Windows).
pub const SERVICE: &str = "app.kenshell.desktop";

pub fn session_account(id: i64) -> String {
    format!("session:{id}")
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

/// In-memory store for tests.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(test)]
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
