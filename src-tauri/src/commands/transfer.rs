//! File transfer commands (P5-T01): SCP over the session's own SSH connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

use crate::error::{AppError, ErrorCode, Result};
use crate::model::RuntimeId;
use crate::state::AppState;
use crate::transfer::{scp, sftp};

/// Cancel flags of running transfers, by the UI's transfer id.
static RUNNING: LazyLock<Mutex<HashMap<u32, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub done: u64,
    pub total: u64,
}

fn ssh_err(e: russh::Error) -> AppError {
    AppError::with(ErrorCode::Io, format!("ssh channel: {e}"))
}

struct Registration(u32);
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut m) = RUNNING.lock() {
            m.remove(&self.0);
        }
    }
}

fn register(transfer: u32) -> (Arc<AtomicBool>, Registration) {
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut m) = RUNNING.lock() {
        m.insert(transfer, Arc::clone(&flag));
    }
    (flag, Registration(transfer))
}

/// Progress at most every 100 ms (and always the last one).
fn throttled(ch: Channel<TransferProgress>) -> impl FnMut(u64, u64) + Send {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |done, total| {
        if done == total || last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            let _ = ch.send(TransferProgress { done, total });
        }
    }
}

async fn exec_stream(
    state: &AppState,
    id: RuntimeId,
    command: String,
) -> Result<russh::ChannelStream<russh::client::Msg>> {
    let handle = state.sessions.ssh_handle(id)?;
    let channel = handle.channel_open_session().await.map_err(ssh_err)?;
    channel.exec(true, command).await.map_err(ssh_err)?;
    Ok(channel.into_stream())
}

/// Copies `remote` (e.g. `flash:startup-config`) from the device to `local`.
#[tauri::command]
pub async fn scp_download(
    state: State<'_, AppState>,
    id: RuntimeId,
    transfer: u32,
    remote: String,
    local: String,
    on_progress: Channel<TransferProgress>,
) -> Result<u64> {
    let (cancel, _reg) = register(transfer);
    let mut stream = exec_stream(&state, id, scp::command('f', remote.trim())).await?;
    let mut file = tokio::fs::File::create(&local)
        .await
        .map_err(AppError::from)?;
    let mut progress = throttled(on_progress);
    let result = scp::download(&mut stream, &mut file, &cancel, &mut progress).await;
    drop(file);
    if result.is_err() {
        // No half-written files left behind.
        let _ = tokio::fs::remove_file(&local).await;
    }
    result
}

/// Copies the local file to `remote_dir` (e.g. `flash:`, `/tmp`) on the device.
#[tauri::command]
pub async fn scp_upload(
    state: State<'_, AppState>,
    id: RuntimeId,
    transfer: u32,
    local: String,
    remote: String,
    on_progress: Channel<TransferProgress>,
) -> Result<()> {
    let (cancel, _reg) = register(transfer);
    let meta = tokio::fs::metadata(&local).await.map_err(AppError::from)?;
    if !meta.is_file() {
        return Err(AppError::invalid("not a file"));
    }
    let name = std::path::Path::new(&local)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::invalid("file name"))?
        .to_owned();
    let mut input = tokio::fs::File::open(&local)
        .await
        .map_err(AppError::from)?;
    let mut stream = exec_stream(&state, id, scp::command('t', remote.trim())).await?;
    let mut progress = throttled(on_progress);
    scp::upload(
        &mut stream,
        &mut input,
        meta.len(),
        &name,
        &cancel,
        &mut progress,
    )
    .await
}

/// Stops a running transfer (a download's partial file is removed).
#[tauri::command]
pub fn transfer_cancel(transfer: u32) {
    if let Ok(m) = RUNNING.lock()
        && let Some(f) = m.get(&transfer)
    {
        f.store(true, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpOpened {
    pub sftp: u32,
    pub home: String,
}

/// Starts an SFTP session on a connected SSH session (fails on devices without SFTP:
/// the message says to use SCP).
#[tauri::command]
pub async fn sftp_open(state: State<'_, AppState>, id: RuntimeId) -> Result<SftpOpened> {
    let handle = state.sessions.ssh_handle(id)?;
    let (sftp, home) = sftp::open(&handle).await?;
    Ok(SftpOpened { sftp, home })
}

#[tauri::command]
pub async fn sftp_list(sftp: u32, path: String) -> Result<Vec<sftp::Entry>> {
    let s = sftp::get(sftp)?;
    sftp::list(&s, &path).await
}

#[tauri::command]
pub async fn sftp_download(
    sftp: u32,
    transfer: u32,
    remote: String,
    local: String,
    on_progress: Channel<TransferProgress>,
) -> Result<u64> {
    let s = sftp::get(sftp)?;
    let (cancel, _reg) = register(transfer);
    let mut file = tokio::fs::File::create(&local)
        .await
        .map_err(AppError::from)?;
    let mut progress = throttled(on_progress);
    let result = sftp::download(&s, &remote, &mut file, &cancel, &mut progress).await;
    drop(file);
    if result.is_err() {
        let _ = tokio::fs::remove_file(&local).await;
    }
    result
}

#[tauri::command]
pub async fn sftp_upload(
    sftp: u32,
    transfer: u32,
    local: String,
    remote_dir: String,
    on_progress: Channel<TransferProgress>,
) -> Result<()> {
    let s = sftp::get(sftp)?;
    let (cancel, _reg) = register(transfer);
    let meta = tokio::fs::metadata(&local).await.map_err(AppError::from)?;
    if !meta.is_file() {
        return Err(AppError::invalid("not a file"));
    }
    let name = std::path::Path::new(&local)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::invalid("file name"))?
        .to_owned();
    let mut input = tokio::fs::File::open(&local)
        .await
        .map_err(AppError::from)?;
    let mut progress = throttled(on_progress);
    sftp::upload(
        &s,
        &mut input,
        meta.len(),
        &sftp::join(&remote_dir, &name),
        &cancel,
        &mut progress,
    )
    .await
}

#[tauri::command]
pub async fn sftp_mkdir(sftp: u32, path: String) -> Result<()> {
    sftp::get(sftp)?.create_dir(path).await.map_err(sftp::err)
}

#[tauri::command]
pub async fn sftp_remove(sftp: u32, path: String, is_dir: bool) -> Result<()> {
    let s = sftp::get(sftp)?;
    if is_dir {
        s.remove_dir(path).await
    } else {
        s.remove_file(path).await
    }
    .map_err(sftp::err)
}

#[tauri::command]
pub async fn sftp_rename(sftp: u32, from: String, to: String) -> Result<()> {
    sftp::get(sftp)?.rename(from, to).await.map_err(sftp::err)
}

#[tauri::command]
pub async fn sftp_close(sftp: u32) {
    sftp::close(sftp).await;
}

// ------------------------------------------------------- File servers (P5-T02, P5-T03)

use crate::transfer::{ftp, scp_server, served, tftp};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    Tftp,
    Ftp,
    Scp,
}

struct Running {
    task: tokio::task::JoinHandle<()>,
    status: ServerStatus,
}

static SERVERS: LazyLock<Mutex<HashMap<ServerKind, Running>>> = LazyLock::new(Default::default);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub kind: ServerKind,
    pub addr: String,
    pub root: String,
    pub allow_upload: bool,
    /// FTP / SCP login (shown so it can be typed on the device; memory only).
    pub user: Option<String>,
    pub password: Option<String>,
    /// SCP host key fingerprints (SHA-256).
    pub fingerprints: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerOptions {
    pub kind: ServerKind,
    pub root: String,
    pub bind_ip: String,
    pub port: u16,
    pub allow_upload: bool,
    pub overwrite: bool,
    pub user: Option<String>,
    pub password: Option<String>,
}

fn servers_now() -> Vec<ServerStatus> {
    let g = SERVERS.lock().ok();
    let mut v: Vec<_> = g
        .iter()
        .flat_map(|g| g.values().map(|r| r.status.clone()))
        .collect();
    v.sort_by_key(|s| s.kind as u8);
    v
}

/// Starts a TFTP, FTP or SCP server on `bindIp:port` serving `root` (one per kind;
/// starting again replaces it).
#[tauri::command]
pub async fn server_start(
    opts: ServerOptions,
    on_event: Channel<served::Event>,
) -> Result<Vec<ServerStatus>> {
    let dir = std::path::PathBuf::from(&opts.root);
    if !dir.is_dir() {
        return Err(AppError::invalid("pick an existing folder"));
    }
    let ip: std::net::IpAddr = opts
        .bind_ip
        .trim()
        .parse()
        .map_err(|_| AppError::invalid("listen address"))?;
    let busy = |e: std::io::Error| {
        AppError::with(
            ErrorCode::PortBusy,
            format!("cannot listen on {ip}:{}: {e}", opts.port),
        )
    };
    let cfg = Arc::new(served::Config {
        root: dir,
        allow_upload: opts.allow_upload,
        overwrite: opts.overwrite,
    });
    let events: served::Events = Arc::new(move |e| {
        let _ = on_event.send(e);
    });
    let creds = (opts.kind != ServerKind::Tftp).then(|| {
        let user = opts
            .user
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or("kenshell")
            .to_owned();
        let password = opts
            .password
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| served::random_password(12));
        Arc::new(served::Credentials { user, password })
    });
    // Replace a running server of the same kind first, so its port is free again.
    if let Ok(mut g) = SERVERS.lock()
        && let Some(old) = g.remove(&opts.kind)
    {
        old.task.abort();
    }
    let mut fingerprints = Vec::new();
    let (task, addr) = match (opts.kind, &creds) {
        (ServerKind::Tftp, _) => {
            let sock = tokio::net::UdpSocket::bind((ip, opts.port))
                .await
                .map_err(busy)?;
            let addr = sock.local_addr().map_err(AppError::from)?;
            (tokio::spawn(tftp::serve(sock, cfg, events)), addr)
        }
        (ServerKind::Ftp, Some(c)) => {
            let l = tokio::net::TcpListener::bind((ip, opts.port))
                .await
                .map_err(busy)?;
            let addr = l.local_addr().map_err(AppError::from)?;
            (
                tokio::spawn(ftp::serve(l, cfg, Arc::clone(c), events)),
                addr,
            )
        }
        (ServerKind::Scp, Some(c)) => {
            let l = tokio::net::TcpListener::bind((ip, opts.port))
                .await
                .map_err(busy)?;
            let addr = l.local_addr().map_err(AppError::from)?;
            // RSA key generation takes a moment: off the async workers.
            let keys = tokio::task::spawn_blocking(|| scp_server::host_keys(true))
                .await
                .map_err(|_| AppError::internal("host key generation"))??;
            fingerprints = scp_server::fingerprints(&keys);
            (
                tokio::spawn(scp_server::serve(l, keys, cfg, Arc::clone(c), events)),
                addr,
            )
        }
        _ => return Err(AppError::internal("server credentials")),
    };
    let status = ServerStatus {
        kind: opts.kind,
        addr: addr.to_string(),
        root: opts.root,
        allow_upload: opts.allow_upload,
        user: creds.as_ref().map(|c| c.user.clone()),
        password: creds.as_ref().map(|c| c.password.clone()),
        fingerprints,
    };
    let mut g = SERVERS
        .lock()
        .map_err(|_| AppError::internal("server state"))?;
    if let Some(old) = g.insert(opts.kind, Running { task, status }) {
        old.task.abort();
    }
    drop(g);
    Ok(servers_now())
}

#[tauri::command]
pub fn server_stop(kind: ServerKind) -> Vec<ServerStatus> {
    if let Ok(mut g) = SERVERS.lock()
        && let Some(r) = g.remove(&kind)
    {
        r.task.abort();
    }
    servers_now()
}

#[tauri::command]
pub fn server_status() -> Vec<ServerStatus> {
    servers_now()
}
