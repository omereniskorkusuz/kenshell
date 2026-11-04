//! Key generator / converter commands (P2-T16). Thin: logic lives in `keys`.
//! Passphrases arrive from the form, are wiped after use and never logged.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::error::{AppError, Result};
use crate::keys::{self, KeyFormat, KeyInfo, KeyKind};
use crate::platform::write_private;

const MAX_KEY_BYTES: u64 = 64 * 1024;

fn read_key(path: &Path) -> Result<Zeroizing<String>> {
    if std::fs::metadata(path)?.len() > MAX_KEY_BYTES {
        return Err(AppError::invalid("file is too large for a key"));
    }
    let bytes = Zeroizing::new(std::fs::read(path)?);
    String::from_utf8(bytes.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| AppError::invalid("not a text key file"))
}

/// Writes the private key; OpenSSH also gets `<path>.pub`.
fn save(key: &russh::keys::PrivateKey, path: &Path, format: KeyFormat, pass: &str) -> Result<()> {
    let text = keys::encode(key, format, Some(pass))?;
    write_private(path, text.as_bytes())?;
    if format == KeyFormat::Openssh {
        let mut pub_path = path.as_os_str().to_owned();
        pub_path.push(".pub");
        std::fs::write(
            PathBuf::from(pub_path),
            format!("{}\n", keys::public_line(key)?),
        )?;
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
pub async fn key_generate(
    kind: KeyKind,
    comment: String,
    format: KeyFormat,
    passphrase: String,
    path: String,
) -> Result<KeyInfo> {
    let pass = Zeroizing::new(passphrase);
    blocking(move || {
        let key = keys::generate(kind, &comment)?;
        save(&key, Path::new(&path), format, &pass)?;
        keys::info(&key).map(|mut i| {
            i.encrypted = !pass.is_empty();
            i
        })
    })
    .await
}

/// Public information without a passphrase.
#[tauri::command]
pub async fn key_inspect(path: String) -> Result<KeyInfo> {
    blocking(move || keys::inspect(&read_key(Path::new(&path))?)).await
}

/// Reads `path` (decrypting with `passphrase`) and saves it as `format` to `dest`.
#[tauri::command]
pub async fn key_convert(
    path: String,
    passphrase: String,
    format: KeyFormat,
    new_passphrase: String,
    dest: String,
) -> Result<KeyInfo> {
    let (pass, new_pass) = (Zeroizing::new(passphrase), Zeroizing::new(new_passphrase));
    blocking(move || {
        if Path::new(&path) == Path::new(&dest) {
            return Err(AppError::invalid(
                "choose a different file to save the copy",
            ));
        }
        let key = keys::parse(&read_key(Path::new(&path))?, Some(&pass))?;
        save(&key, Path::new(&dest), format, &new_pass)?;
        keys::info(&key).map(|mut i| {
            i.encrypted = !new_pass.is_empty();
            i
        })
    })
    .await
}
