//! Encrypted local blobs (config backups, ADR-016): `nonce ‖ ChaCha20-Poly1305(deflate(data))`
//! under a random per-install key kept only in the OS keychain.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use super::SecretStore;
use crate::error::{AppError, Result};

/// Keychain account of the config backup key.
pub const BACKUP_KEY_ACCOUNT: &str = "config-backups-key";
/// Largest plaintext accepted when opening (decompression-bomb guard).
pub const MAX_PLAIN: u64 = 32 * 1024 * 1024;
/// Associated data: binds the ciphertext to this format.
const AAD: &[u8] = b"kenshell-blob-v1";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The backup key, created on first use. Never written anywhere but the keychain.
pub fn backup_key(store: &dyn SecretStore) -> Result<Zeroizing<[u8; 32]>> {
    if let Some(s) = store.get(BACKUP_KEY_ACCOUNT)? {
        return unhex(&s)
            .map(Zeroizing::new)
            .ok_or_else(|| AppError::internal("backup key in the keychain is malformed"));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(key.as_mut())
        .map_err(|_| AppError::internal("system random generator failed"))?;
    store.set(BACKUP_KEY_ACCOUNT, &Zeroizing::new(hex(key.as_ref())))?;
    Ok(key)
}

fn cipher(key: &[u8; 32]) -> Result<LessSafeKey> {
    UnboundKey::new(&CHACHA20_POLY1305, key)
        .map(LessSafeKey::new)
        .map_err(|_| AppError::internal("cipher key"))
}

/// Compresses and encrypts `data`.
pub fn seal(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).map_err(AppError::from)?;
    let mut body = Zeroizing::new(enc.finish().map_err(AppError::from)?);
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| AppError::internal("system random generator failed"))?;
    cipher(key)?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(AAD),
            &mut *body,
        )
        .map_err(|_| AppError::internal("encryption failed"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + body.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decrypts and decompresses a [`seal`]ed blob (fails on any change to it).
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if sealed.len() < NONCE_LEN + 16 {
        return Err(AppError::invalid("backup data is damaged"));
    }
    let (nonce, body) = sealed.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce)
        .map_err(|_| AppError::invalid("backup data is damaged"))?;
    let mut buf = Zeroizing::new(body.to_vec());
    let plain = cipher(key)?
        .open_in_place(nonce, Aad::from(AAD), &mut buf)
        .map_err(|_| AppError::invalid("backup data is damaged or from another installation"))?;
    let mut out = Zeroizing::new(Vec::new());
    DeflateDecoder::new(&plain[..])
        .take(MAX_PLAIN + 1)
        .read_to_end(&mut out)
        .map_err(|_| AppError::invalid("backup data is damaged"))?;
    if out.len() as u64 > MAX_PLAIN {
        return Err(AppError::invalid("backup is too large"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn key_is_created_once_and_round_trips() {
        let store = MemoryStore::default();
        let k1 = backup_key(&store).unwrap();
        let k2 = backup_key(&store).unwrap();
        assert_eq!(*k1, *k2);
        let cfg = "hostname sw1\nenable secret 9 $9$abc\n".repeat(200);
        let sealed = seal(&k1, cfg.as_bytes()).unwrap();
        assert!(sealed.len() < cfg.len() / 4, "compressed");
        assert!(
            !String::from_utf8_lossy(&sealed).contains("enable secret"),
            "encrypted"
        );
        assert_eq!(&open(&k1, &sealed).unwrap()[..], cfg.as_bytes());
        // Tampering and another key are both refused.
        let mut bad = sealed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(open(&k1, &bad).is_err());
        assert!(open(&[7u8; 32], &sealed).is_err());
        assert!(open(&k1, b"short").is_err());
    }
}
