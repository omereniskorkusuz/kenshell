//! SSH key generator and converter (P2-T16, PuTTYgen equivalent).
//!
//! Generation, OpenSSH reading/writing and PPK reading come from `ssh-key` (already in
//! the tree through russh). `ssh-key` cannot *write* PPK, so [`to_ppk`] implements
//! PuTTY's v3 format (verified byte-for-byte against PuTTYgen output in the tests).
//! Private key text is kept in `Zeroizing` buffers; nothing here is ever logged.

use argon2::{Algorithm as Argon, Argon2, Params, Version};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use russh::keys::ssh_key::encoding::Encode;
use russh::keys::ssh_key::encoding::base64::{Base64, Encoding};
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData, RsaKeypair};
use russh::keys::ssh_key::rand_core::{TryCryptoRng, TryRng, UnwrapErr};
use russh::keys::ssh_key::{self, Cipher, HashAlg, LineEnding, PrivateKey, PublicKey};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, ErrorCode, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyKind {
    Ed25519,
    Rsa3072,
    Rsa4096,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyFormat {
    /// OpenSSH private key (+ `.pub` file next to it).
    Openssh,
    /// PuTTY `.ppk` version 3.
    Ppk,
}

/// What the UI shows about a key; never contains private material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyInfo {
    pub algorithm: String,
    pub bits: Option<u32>,
    pub comment: String,
    /// `SHA256:…`
    pub fingerprint: String,
    /// `ssh-ed25519 AAAA… comment` (for authorized_keys / device config).
    pub public_line: String,
    pub encrypted: bool,
}

const MAX_COMMENT: usize = 255;

/// The OS random generator (through ring) for `ssh-key`, whose own `getrandom`
/// feature is not enabled in our tree.
struct OsRng;

#[derive(Debug)]
struct RngFailed;

impl std::fmt::Display for RngFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("system random generator failed")
    }
}

impl std::error::Error for RngFailed {}

impl TryRng for OsRng {
    type Error = RngFailed;
    fn try_next_u32(&mut self) -> Result<u32, RngFailed> {
        let mut b = [0u8; 4];
        self.try_fill_bytes(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn try_next_u64(&mut self) -> Result<u64, RngFailed> {
        let mut b = [0u8; 8];
        self.try_fill_bytes(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), RngFailed> {
        SystemRandom::new().fill(dst).map_err(|_| RngFailed)
    }
}

impl TryCryptoRng for OsRng {}
const PPK_HEADER: &str = "PuTTY-User-Key-File-";

fn key_error(e: ssh_key::Error) -> AppError {
    AppError::invalid(format!("key: {e}"))
}

fn needs_passphrase() -> AppError {
    AppError::with(ErrorCode::BadPassword, "passphrase required or wrong")
}

fn clean_comment(comment: &str) -> Result<String> {
    let c = comment.trim();
    if c.chars().count() > MAX_COMMENT || c.chars().any(char::is_control) {
        return Err(AppError::invalid(
            "key comment is too long or has control characters",
        ));
    }
    Ok(c.to_owned())
}

pub fn generate(kind: KeyKind, comment: &str) -> Result<PrivateKey> {
    let comment = clean_comment(comment)?;
    let mut rng = UnwrapErr(OsRng);
    let data = match kind {
        KeyKind::Ed25519 => KeypairData::from(Ed25519Keypair::random(&mut rng)),
        KeyKind::Rsa3072 | KeyKind::Rsa4096 => {
            let bits = if kind == KeyKind::Rsa3072 { 3072 } else { 4096 };
            KeypairData::from(RsaKeypair::random(&mut rng, bits).map_err(key_error)?)
        }
    };
    PrivateKey::new(data, comment).map_err(key_error)
}

fn info_of(public: &PublicKey, encrypted: bool) -> Result<KeyInfo> {
    let bits = match public.key_data() {
        ssh_key::public::KeyData::Rsa(r) => Some(r.key_size()),
        ssh_key::public::KeyData::Ed25519(_) => Some(256),
        _ => None,
    };
    Ok(KeyInfo {
        algorithm: public.algorithm().as_str().to_owned(),
        bits,
        comment: public.comment().to_string(),
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        public_line: public.to_openssh().map_err(key_error)?,
        encrypted,
    })
}

pub fn info(key: &PrivateKey) -> Result<KeyInfo> {
    info_of(key.public_key(), key.is_encrypted())
}

fn is_ppk(text: &str) -> bool {
    text.trim_start().starts_with(PPK_HEADER)
}

/// PPK header fields without decrypting (public part is never encrypted).
fn ppk_public(text: &str) -> Result<(PublicKey, bool)> {
    let mut lines = text.lines();
    let mut comment = String::new();
    let mut encrypted = false;
    let mut public = None;
    while let Some(line) = lines.next() {
        if let Some(v) = line.strip_prefix("Encryption: ") {
            encrypted = v.trim() != "none";
        } else if let Some(v) = line.strip_prefix("Comment: ") {
            v.clone_into(&mut comment);
        } else if let Some(v) = line.strip_prefix("Public-Lines: ") {
            let n: usize = v
                .trim()
                .parse()
                .map_err(|_| AppError::invalid("ppk: bad Public-Lines"))?;
            let b64: String = lines.by_ref().take(n.min(64)).collect();
            let blob =
                Base64::decode_vec(&b64).map_err(|_| AppError::invalid("ppk: bad base64"))?;
            public = Some(PublicKey::from_bytes(&blob).map_err(key_error)?);
        }
    }
    let mut public = public.ok_or_else(|| AppError::invalid("ppk: no public key"))?;
    public.set_comment(comment);
    Ok((public, encrypted))
}

/// Public information of a key file without its passphrase.
pub fn inspect(text: &str) -> Result<KeyInfo> {
    if is_ppk(text) {
        let (public, encrypted) = ppk_public(text)?;
        return info_of(&public, encrypted);
    }
    if let Ok(k) = PrivateKey::from_openssh(text) {
        return info(&k);
    }
    // PKCS#1/PKCS#8 PEM: russh can read those (unencrypted ones without a passphrase).
    let k = russh::keys::decode_secret_key(text, None).map_err(|e| match e {
        russh::keys::Error::KeyIsEncrypted => needs_passphrase(),
        _ => AppError::invalid("not a supported private key file"),
    })?;
    info(&k)
}

/// Fully decoded (decrypted) private key from OpenSSH, PPK v2/v3 or PEM text.
pub fn parse(text: &str, passphrase: Option<&str>) -> Result<PrivateKey> {
    let pass = passphrase.filter(|p| !p.is_empty());
    if is_ppk(text) {
        let (_, encrypted) = ppk_public(text)?;
        if encrypted && pass.is_none() {
            return Err(needs_passphrase());
        }
        // Encrypted: any failure means a wrong passphrase (the MAC check fails).
        return PrivateKey::from_ppk(text, pass.map(str::to_owned)).map_err(|e| {
            if encrypted {
                needs_passphrase()
            } else {
                key_error(e)
            }
        });
    }
    if let Ok(k) = PrivateKey::from_openssh(text) {
        if !k.is_encrypted() {
            return Ok(k);
        }
        let pass = pass.ok_or_else(needs_passphrase)?;
        return k.decrypt(pass).map_err(|_| needs_passphrase());
    }
    russh::keys::decode_secret_key(text, pass).map_err(|e| match e {
        russh::keys::Error::KeyIsEncrypted => needs_passphrase(),
        _ if pass.is_some() => needs_passphrase(),
        _ => AppError::invalid("not a supported private key file"),
    })
}

fn random(buf: &mut [u8]) -> Result<()> {
    SystemRandom::new()
        .fill(buf)
        .map_err(|_| AppError::internal("system random generator failed"))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Base64 in PuTTY's 64-column lines.
fn ppk_lines(blob: &[u8]) -> (usize, String) {
    let b64 = Base64::encode_string(blob);
    let lines: Vec<&str> = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    (lines.len(), lines.join("\n"))
}

fn enc<T: Encode + ?Sized>(v: &T, out: &mut Vec<u8>) -> Result<()> {
    v.encode(out).map_err(|e| key_error(e.into()))
}

/// PuTTY private blob: ed25519 = string(seed); RSA = mpint d, p, q, iqmp.
fn ppk_private_blob(key: &PrivateKey) -> Result<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::new());
    match key.key_data() {
        KeypairData::Ed25519(k) => enc(k.private.to_bytes().as_slice(), &mut out)?,
        KeypairData::Rsa(k) => {
            let p = k.private();
            for m in [p.d(), p.p(), p.q(), p.iqmp()] {
                enc(m, &mut out)?;
            }
        }
        _ => {
            return Err(AppError::with(
                ErrorCode::Unsupported,
                "PPK export supports Ed25519 and RSA keys",
            ));
        }
    }
    Ok(out)
}

/// PuTTYgen's Argon2id settings for v3 files.
const PPK_ARGON_MEMORY: u32 = 8192;
const PPK_ARGON_PASSES: u32 = 21;

/// Encodes a (decrypted) key as PuTTY `.ppk` v3, optionally with a passphrase.
pub fn to_ppk(key: &PrivateKey, passphrase: Option<&str>) -> Result<Zeroizing<String>> {
    let pass = passphrase.filter(|p| !p.is_empty());
    let algorithm = key.algorithm();
    let comment = key.comment().to_string();
    let public = key.public_key().to_bytes().map_err(key_error)?;
    let mut private = ppk_private_blob(key)?;
    let encryption = if pass.is_some() { "aes256-cbc" } else { "none" };

    let mut kdf_lines = String::new();
    let mut mac_key = Zeroizing::new(Vec::new());
    let mut cipher_key = Zeroizing::new([0u8; 48]);
    if let Some(pass) = pass {
        let mut salt = [0u8; 16];
        random(&mut salt)?;
        let params = Params::new(PPK_ARGON_MEMORY, PPK_ARGON_PASSES, 1, Some(80))
            .map_err(|e| AppError::internal(format!("argon2: {e}")))?;
        let mut okm = Zeroizing::new([0u8; 80]);
        Argon2::new(Argon::Argon2id, Version::V0x13, params)
            .hash_password_into(pass.as_bytes(), &salt, okm.as_mut())
            .map_err(|e| AppError::internal(format!("argon2: {e}")))?;
        cipher_key.copy_from_slice(&okm[..48]);
        mac_key.extend_from_slice(&okm[48..]);
        kdf_lines = format!(
            "Key-Derivation: Argon2id\nArgon2-Memory: {PPK_ARGON_MEMORY}\nArgon2-Passes: {PPK_ARGON_PASSES}\nArgon2-Parallelism: 1\nArgon2-Salt: {}\n",
            hex(&salt)
        );
        // Pad to the AES block size with random bytes (covered by the MAC).
        let pad = (16 - private.len() % 16) % 16;
        let mut padding = vec![0u8; pad];
        random(&mut padding)?;
        private.extend_from_slice(&padding);
    }

    let mut mac_data = Zeroizing::new(Vec::new());
    enc(algorithm.as_str(), &mut mac_data)?;
    enc(encryption, &mut mac_data)?;
    enc(comment.as_str(), &mut mac_data)?;
    enc(public.as_slice(), &mut mac_data)?;
    enc(private.as_slice(), &mut mac_data)?;
    let mac = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &mac_key), &mac_data);

    if pass.is_some() {
        Cipher::Aes256Cbc
            .encrypt(&cipher_key[..32], &cipher_key[32..48], &mut private)
            .map_err(|_| AppError::internal("aes256-cbc failed"))?;
    }
    let (pub_n, pub_text) = ppk_lines(&public);
    let (priv_n, priv_text) = ppk_lines(&private);
    Ok(Zeroizing::new(format!(
        "{PPK_HEADER}3: {alg}\nEncryption: {encryption}\nComment: {comment}\nPublic-Lines: {pub_n}\n{pub_text}\n{kdf_lines}Private-Lines: {priv_n}\n{priv_text}\nPrivate-MAC: {mac}\n",
        alg = algorithm.as_str(),
        mac = hex(mac.as_ref()),
    )))
}

/// Private key file text in `format` (encrypted when a passphrase is given).
pub fn encode(
    key: &PrivateKey,
    format: KeyFormat,
    passphrase: Option<&str>,
) -> Result<Zeroizing<String>> {
    match format {
        KeyFormat::Ppk => to_ppk(key, passphrase),
        KeyFormat::Openssh => match passphrase.filter(|p| !p.is_empty()) {
            None => key.to_openssh(LineEnding::LF).map_err(key_error),
            Some(p) => key
                .encrypt(&mut OsRng, p)
                .and_then(|k| k.to_openssh(LineEnding::LF))
                .map_err(key_error),
        },
    }
}

/// `authorized_keys` line for the `.pub` file.
pub fn public_line(key: &PrivateKey) -> Result<String> {
    key.public_key().to_openssh().map_err(key_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED_PPK: &str = include_str!("../../tests/fixtures/keys/id_ed25519.ppk");
    const ED_PPK_ENC: &str = include_str!("../../tests/fixtures/keys/id_ed25519_enc.ppk");
    const ED_OPENSSH: &str = include_str!("../../tests/fixtures/keys/id_ed25519");
    const RSA_PPK: &str = include_str!("../../tests/fixtures/keys/id_rsa_3072.ppk");
    const RSA_OPENSSH: &str = include_str!("../../tests/fixtures/keys/id_rsa_3072");

    #[test]
    fn ppk_writer_matches_puttygen_byte_for_byte() {
        for (ppk, openssh) in [(ED_PPK, ED_OPENSSH), (RSA_PPK, RSA_OPENSSH)] {
            let key = parse(openssh, None).unwrap();
            let ours = to_ppk(&key, None).unwrap();
            assert_eq!(ours.as_str(), ppk.replace("\r\n", "\n"));
        }
    }

    #[test]
    fn encrypted_ppk_round_trips_and_reads_puttygen_files() {
        let key = parse(ED_PPK_ENC, Some("123")).unwrap();
        assert_eq!(key, parse(ED_OPENSSH, None).unwrap());
        assert_eq!(
            parse(ED_PPK_ENC, None).unwrap_err().code,
            ErrorCode::BadPassword
        );
        assert_eq!(
            parse(ED_PPK_ENC, Some("nope")).unwrap_err().code,
            ErrorCode::BadPassword
        );

        let ours = to_ppk(&key, Some("s3cret-pass")).unwrap();
        assert!(ours.contains("Encryption: aes256-cbc"));
        assert_eq!(parse(&ours, Some("s3cret-pass")).unwrap(), key);
        assert_eq!(
            parse(&ours, Some("wrong")).unwrap_err().code,
            ErrorCode::BadPassword
        );
        // Public part is readable without the passphrase.
        let i = inspect(&ours).unwrap();
        assert!(i.encrypted);
        assert_eq!(i.fingerprint, info(&key).unwrap().fingerprint);
    }

    #[test]
    fn generate_and_convert_every_way() {
        let key = generate(KeyKind::Ed25519, " ops@kenshell ").unwrap();
        let i = info(&key).unwrap();
        assert_eq!(
            (i.algorithm.as_str(), i.bits, i.comment.as_str()),
            ("ssh-ed25519", Some(256), "ops@kenshell")
        );
        assert!(i.public_line.starts_with("ssh-ed25519 AAAA"));
        assert!(i.fingerprint.starts_with("SHA256:"));

        for format in [KeyFormat::Openssh, KeyFormat::Ppk] {
            for pass in [None, Some("pass phrase 1")] {
                let text = encode(&key, format, pass).unwrap();
                assert_eq!(inspect(&text).unwrap().encrypted, pass.is_some());
                assert_eq!(parse(&text, pass).unwrap(), key, "{format:?} {pass:?}");
                if pass.is_some() {
                    assert_eq!(parse(&text, None).unwrap_err().code, ErrorCode::BadPassword);
                }
            }
        }
        assert!(generate(KeyKind::Ed25519, "a\nb").is_err());
    }

    #[test]
    #[ignore = "RSA generation is slow in debug builds"]
    fn rsa_generation() {
        let key = generate(KeyKind::Rsa3072, "rsa").unwrap();
        assert_eq!(info(&key).unwrap().bits, Some(3072));
        let ppk = to_ppk(&key, None).unwrap();
        assert_eq!(parse(&ppk, None).unwrap(), key);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(inspect("hello").unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(parse("hello", None).unwrap_err().code, ErrorCode::Invalid);
    }
}
