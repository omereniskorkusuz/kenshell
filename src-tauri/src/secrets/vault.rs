//! Encrypted vault file (P2-T15): a master password protects a portable backup.
//!
//! Layout (all integers little endian):
//! ```text
//! "KSVAULT1" | m_cost KiB u32 | t_cost u32 | p_cost u32 | salt [16] | nonce [12] | ciphertext+tag
//! ```
//! Key = Argon2id(password, salt, params) → 32 bytes; cipher = ChaCha20-Poly1305 (IETF)
//! with the whole header as associated data, so changing the parameters or the salt
//! makes decryption fail instead of silently weakening the file. A fresh random salt
//! per file means a fresh key, so a random 96-bit nonce is safe.

use argon2::{Algorithm, Argon2, Params, Version};
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use crate::error::{AppError, ErrorCode, Result};

pub const MAGIC: &[u8; 8] = b"KSVAULT1";
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = MAGIC.len() + 12 + SALT_LEN + NONCE_LEN;
pub const MIN_PASSWORD_CHARS: usize = 8;

/// Argon2id cost used for new files: 64 MiB, 3 passes, 1 lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

pub const DEFAULT_COST: Cost = Cost {
    m_kib: 64 * 1024,
    t: 3,
    p: 1,
};
// Never below the OWASP Argon2id minimum (19 MiB, 2 passes).
const _: () = assert!(DEFAULT_COST.m_kib >= 19 * 1024 && DEFAULT_COST.t >= 2);

/// Upper bounds accepted when opening: a crafted file must not make us allocate
/// gigabytes or spin for minutes.
const MAX_COST: Cost = Cost {
    m_kib: 1024 * 1024,
    t: 16,
    p: 8,
};

fn bad_password() -> AppError {
    AppError::with(
        ErrorCode::BadPassword,
        "wrong master password or damaged file",
    )
}

fn derive(password: &str, salt: &[u8], cost: Cost) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(cost.m_kib, cost.t, cost.p, Some(32))
        .map_err(|e| AppError::invalid(format!("vault parameters: {e}")))?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| AppError::internal(format!("argon2: {e}")))?;
    Ok(key)
}

fn cipher(key: &[u8; 32]) -> Result<LessSafeKey> {
    UnboundKey::new(&CHACHA20_POLY1305, key)
        .map(LessSafeKey::new)
        .map_err(|_| AppError::internal("cipher key"))
}

/// Encrypts `plaintext` with `password` (checked for minimum length); new files use
/// [`DEFAULT_COST`].
pub fn seal(password: &str, plaintext: &[u8], cost: Cost) -> Result<Vec<u8>> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::invalid(format!(
            "master password needs at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill(&mut salt)
        .and_then(|()| rng.fill(&mut nonce))
        .map_err(|_| AppError::internal("system random generator failed"))?;

    let mut out = Vec::with_capacity(HEADER_LEN + plaintext.len() + 16);
    out.extend_from_slice(MAGIC);
    for v in [cost.m_kib, cost.t, cost.p] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);

    let key = derive(password, &salt, cost)?;
    let mut body = plaintext.to_vec();
    let sealed = cipher(&key)?.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(&out[..HEADER_LEN]),
        &mut body,
    );
    if sealed.is_err() {
        // Wipe the copy of the plaintext before giving up.
        drop(Zeroizing::new(body));
        return Err(AppError::internal("encryption failed"));
    }
    out.extend_from_slice(&body);
    Ok(out)
}

/// True when `data` starts like a vault file (cheap check before asking a password).
pub fn is_vault(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

/// Decrypts a vault; wrong password and any tampering both give `badPassword`.
pub fn open(password: &str, data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if !is_vault(data) {
        return Err(AppError::invalid("not a KenShell vault file"));
    }
    if data.len() < HEADER_LEN + 16 {
        return Err(bad_password());
    }
    let u32_at = |i: usize| {
        let mut b = [0u8; 4];
        b.copy_from_slice(&data[i..i + 4]);
        u32::from_le_bytes(b)
    };
    let cost = Cost {
        m_kib: u32_at(8),
        t: u32_at(12),
        p: u32_at(16),
    };
    if cost.m_kib > MAX_COST.m_kib || cost.t > MAX_COST.t || cost.p > MAX_COST.p {
        return Err(AppError::invalid("vault parameters are out of range"));
    }
    let salt = &data[20..20 + SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&data[20 + SALT_LEN..HEADER_LEN]);
    let key = derive(password, salt, cost).map_err(|e| match e.code {
        ErrorCode::Invalid => e,
        _ => bad_password(),
    })?;
    let mut body = Zeroizing::new(data[HEADER_LEN..].to_vec());
    let len = cipher(&key)?
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(&data[..HEADER_LEN]),
            body.as_mut_slice(),
        )
        .map_err(|_| bad_password())?
        .len();
    body.truncate(len);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters keep the tests fast; the format is the same.
    const FAST: Cost = Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };

    #[test]
    fn round_trip_and_fresh_salt() {
        let a = seal("correct horse", b"secret payload", FAST).unwrap();
        let b = seal("correct horse", b"secret payload", FAST).unwrap();
        assert!(is_vault(&a));
        assert_ne!(a, b, "random salt + nonce per file");
        assert!(
            !a.windows(6).any(|w| w == b"secret"),
            "plaintext never in the file"
        );
        assert_eq!(
            open("correct horse", &a).unwrap().as_slice(),
            b"secret payload"
        );
    }

    #[test]
    fn wrong_password_and_tampering_fail_the_same_way() {
        let file = seal("correct horse", b"payload", FAST).unwrap();
        assert_eq!(
            open("wrong horse!", &file).unwrap_err().code,
            ErrorCode::BadPassword
        );
        for i in [9, 22, HEADER_LEN - 1, HEADER_LEN, file.len() - 1] {
            let mut bad = file.clone();
            bad[i] ^= 1;
            let code = open("correct horse", &bad).unwrap_err().code;
            assert!(
                matches!(code, ErrorCode::BadPassword | ErrorCode::Invalid),
                "byte {i}: {code:?}"
            );
        }
        assert_eq!(
            open("correct horse", &file[..HEADER_LEN + 3])
                .unwrap_err()
                .code,
            ErrorCode::BadPassword
        );
    }

    #[test]
    fn rejects_short_passwords_foreign_files_and_huge_costs() {
        assert_eq!(
            seal("short", b"x", FAST).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(
            open("whatever1", b"{\"json\":true}").unwrap_err().code,
            ErrorCode::Invalid
        );
        let mut file = seal("correct horse", b"x", FAST).unwrap();
        file[8..12].copy_from_slice(&(4 * 1024 * 1024u32).to_le_bytes());
        assert_eq!(
            open("correct horse", &file).unwrap_err().code,
            ErrorCode::Invalid
        );
    }
}
