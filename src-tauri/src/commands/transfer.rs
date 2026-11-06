//! File transfer commands (P5-T01): SCP over the session's own SSH connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::error::{AppError, ErrorCode, Result};
use crate::model::RuntimeId;
use crate::state::AppState;
use crate::transfer::scp;

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
