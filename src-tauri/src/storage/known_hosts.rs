//! SSH host keys (P2-T02). One key per (host, port): learned silently on first
//! connection, and a different key later blocks the connection until the user
//! explicitly forgets the stored one.

use rusqlite::{OptionalExtension, params};

use super::{Db, now_ms};
use crate::error::{AppError, Result};
use crate::model::KnownHost;

/// Hosts are matched case-insensitively (DNS names) and without surrounding spaces.
fn norm_host(host: &str) -> Result<String> {
    let h = host.trim().to_ascii_lowercase();
    if h.is_empty() || h.len() > 255 {
        return Err(AppError::invalid("host is empty or too long"));
    }
    Ok(h)
}

impl Db {
    pub fn get_known_host(&self, host: &str, port: u16) -> Result<Option<KnownHost>> {
        let host = norm_host(host)?;
        Ok(self
            .conn()
            .query_row(
                "SELECT host, port, algorithm, public_key, fingerprint, added_at
                 FROM known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port],
                |r| {
                    Ok(KnownHost {
                        host: r.get(0)?,
                        port: r.get(1)?,
                        algorithm: r.get(2)?,
                        public_key: r.get(3)?,
                        fingerprint: r.get(4)?,
                        added_at: r.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_known_hosts(&self) -> Result<Vec<KnownHost>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT host, port, algorithm, public_key, fingerprint, added_at
             FROM known_hosts ORDER BY host, port",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(KnownHost {
                    host: r.get(0)?,
                    port: r.get(1)?,
                    algorithm: r.get(2)?,
                    public_key: r.get(3)?,
                    fingerprint: r.get(4)?,
                    added_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Stores (or replaces) the key for host:port; `added_at` = now.
    pub fn save_known_host(&self, key: &KnownHost) -> Result<()> {
        let host = norm_host(&key.host)?;
        if key.port == 0 || key.public_key.trim().is_empty() {
            return Err(AppError::invalid("known host needs a port and a key"));
        }
        self.conn().execute(
            "INSERT INTO known_hosts (host, port, algorithm, public_key, fingerprint, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(host, port) DO UPDATE SET algorithm = excluded.algorithm,
               public_key = excluded.public_key, fingerprint = excluded.fingerprint,
               added_at = excluded.added_at",
            params![
                host,
                key.port,
                key.algorithm,
                key.public_key,
                key.fingerprint,
                now_ms()
            ],
        )?;
        Ok(())
    }

    /// Forgets the stored key so the next connection learns the new one. Idempotent.
    pub fn forget_known_host(&self, host: &str, port: u16) -> Result<()> {
        let host = norm_host(host)?;
        self.conn().execute(
            "DELETE FROM known_hosts WHERE host = ?1 AND port = ?2",
            params![host, port],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(host: &str, fp: &str) -> KnownHost {
        KnownHost {
            host: host.into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            public_key: format!("ssh-ed25519 AAAA{fp}"),
            fingerprint: format!("SHA256:{fp}"),
            added_at: 0,
        }
    }

    #[test]
    fn save_get_replace_forget() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.get_known_host("core-sw-01", 22).unwrap().is_none());
        db.save_known_host(&key("Core-SW-01 ", "a")).unwrap();
        let k = db.get_known_host("core-sw-01", 22).unwrap().unwrap();
        assert_eq!(
            (k.host.as_str(), k.fingerprint.as_str()),
            ("core-sw-01", "SHA256:a")
        );
        assert!(k.added_at > 0);
        assert!(db.get_known_host("core-sw-01", 2222).unwrap().is_none());
        db.save_known_host(&key("core-sw-01", "b")).unwrap();
        assert_eq!(
            db.get_known_host("CORE-SW-01", 22)
                .unwrap()
                .unwrap()
                .fingerprint,
            "SHA256:b"
        );
        db.forget_known_host("core-sw-01", 22).unwrap();
        db.forget_known_host("core-sw-01", 22).unwrap();
        assert!(db.get_known_host("core-sw-01", 22).unwrap().is_none());
    }

    #[test]
    fn rejects_blank_input() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.get_known_host(" ", 22).is_err());
        assert!(db.save_known_host(&key("", "x")).is_err());
    }
}
