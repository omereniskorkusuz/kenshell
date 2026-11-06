//! SFTP (P5-T01): an `sftp` subsystem channel on the session's SSH connection, browsed
//! with `russh-sftp` (ADR-019). Open SFTP sessions live in a small registry until closed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use russh_sftp::client::SftpSession;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, ErrorCode, Result};

static SESSIONS: LazyLock<Mutex<HashMap<u32, Arc<SftpSession>>>> = LazyLock::new(Default::default);
static NEXT: AtomicU32 = AtomicU32::new(1);
const CHUNK: usize = 64 * 1024;

pub fn err(e: russh_sftp::client::error::Error) -> AppError {
    AppError::with(ErrorCode::Io, format!("sftp: {e}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    /// Seconds since 1970 (0 = unknown).
    pub modified: u64,
}

/// Starts SFTP on an authenticated connection; returns the registry id and the start
/// directory.
pub async fn open(
    handle: &russh::client::Handle<crate::transport::ssh::Client>,
) -> Result<(u32, String)> {
    let ch = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::with(ErrorCode::Io, format!("ssh channel: {e}")))?;
    ch.request_subsystem(true, "sftp")
        .await
        .map_err(|e| AppError::with(ErrorCode::Unsupported, format!("sftp subsystem: {e}")))?;
    let sftp = SftpSession::new(ch.into_stream()).await.map_err(|e| {
        AppError::with(
            ErrorCode::Unsupported,
            format!("this device has no SFTP server ({e}); use SCP"),
        )
    })?;
    let home = sftp.canonicalize(".").await.unwrap_or_else(|_| "/".into());
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut m) = SESSIONS.lock() {
        m.insert(id, Arc::new(sftp));
    }
    Ok((id, home))
}

pub fn get(id: u32) -> Result<Arc<SftpSession>> {
    SESSIONS
        .lock()
        .ok()
        .and_then(|m| m.get(&id).cloned())
        .ok_or_else(|| AppError::not_found(format!("sftp {id}")))
}

pub async fn close(id: u32) {
    let s = SESSIONS.lock().ok().and_then(|mut m| m.remove(&id));
    if let Some(s) = s {
        let _ = s.close().await;
    }
}

/// `dir` + `name` with one `/`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Folders first, then by name (case-insensitive); `.` and `..` left out.
pub fn sort_entries(list: &mut [Entry]) {
    list.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub async fn list(s: &SftpSession, path: &str) -> Result<Vec<Entry>> {
    let dir = s.read_dir(path).await.map_err(err)?;
    let mut out: Vec<Entry> = dir
        .filter(|e| {
            let n = e.file_name();
            n != "." && n != ".."
        })
        .map(|e| {
            let m = e.metadata();
            let t = e.file_type();
            Entry {
                path: join(path, &e.file_name()),
                name: e.file_name(),
                is_dir: t.is_dir(),
                is_link: t.is_symlink(),
                size: m.size.unwrap_or(0),
                modified: u64::from(m.mtime.unwrap_or(0)),
            }
        })
        .collect();
    sort_entries(&mut out);
    Ok(out)
}

pub async fn download(
    s: &SftpSession,
    remote: &str,
    local: &mut tokio::fs::File,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64> {
    let total = s.metadata(remote).await.map_err(err)?.size.unwrap_or(0);
    let mut f = s.open(remote).await.map_err(err)?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    progress(0, total);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::with(ErrorCode::Io, "cancelled"));
        }
        let n = f.read(&mut buf).await.map_err(AppError::from)?;
        if n == 0 {
            break;
        }
        local.write_all(&buf[..n]).await.map_err(AppError::from)?;
        done += n as u64;
        progress(done, total.max(done));
    }
    local.flush().await.map_err(AppError::from)?;
    progress(done, done);
    Ok(done)
}

pub async fn upload(
    s: &SftpSession,
    local: &mut tokio::fs::File,
    size: u64,
    remote: &str,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<()> {
    let mut f = s.create(remote).await.map_err(err)?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    progress(0, size);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = s.remove_file(remote).await;
            return Err(AppError::with(ErrorCode::Io, "cancelled"));
        }
        let n = local.read(&mut buf).await.map_err(AppError::from)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).await.map_err(AppError::from)?;
        done += n as u64;
        progress(done, size.max(done));
    }
    f.shutdown().await.map_err(AppError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: join("/x", name),
            is_dir: dir,
            is_link: false,
            size: 0,
            modified: 0,
        }
    }

    #[test]
    fn paths_and_ordering() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        let mut l = vec![
            e("b.txt", false),
            e("Zdir", true),
            e("a.txt", false),
            e("adir", true),
        ];
        sort_entries(&mut l);
        assert_eq!(
            l.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["adir", "Zdir", "a.txt", "b.txt"]
        );
        assert!(get(999_999).is_err());
    }
}
