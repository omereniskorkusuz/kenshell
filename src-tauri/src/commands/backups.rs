//! Config backups and diff commands (P4-T08). Configs are stored encrypted (ADR-016); the
//! plain text only exists in memory while a backup is taken, shown or compared.

use tauri::State;

use crate::analyzer::backup::{clean_capture, sha256_hex};
use crate::analyzer::diff::{Diff, diff};
use crate::error::{AppError, Result};
use crate::model::RuntimeId;
use crate::secrets::blob;
use crate::state::AppState;
use crate::storage::{BackupInfo, BackupSource, NewBackup};

/// Seals and stores a config (blocking: keychain + compression).
fn store(
    state: &AppState,
    session_id: Option<i64>,
    device_id: Option<i64>,
    hostname: Option<String>,
    source: BackupSource,
    config: &str,
) -> Result<BackupInfo> {
    if config.trim().is_empty() {
        return Err(AppError::invalid("the device returned an empty config"));
    }
    let key = blob::backup_key(state.secrets.as_ref())?;
    let sealed = blob::seal(&key, config.as_bytes())?;
    let sha = sha256_hex(config.as_bytes());
    state.db.save_backup(&NewBackup {
        session_id,
        device_id,
        hostname: hostname.as_deref(),
        source,
        size: config.len() as i64,
        lines: config.lines().count() as i64,
        sha256: &sha,
        sealed: &sealed,
    })
}

fn text_of(state: &AppState, id: i64) -> Result<String> {
    let sealed = state.db.backup_data(id)?;
    let key = blob::backup_key(state.secrets.as_ref())?;
    let plain = blob::open(&key, &sealed)?;
    Ok(String::from_utf8_lossy(&plain).into_owned())
}

/// Reads the running (or startup) config hidden from the terminal and stores it.
#[tauri::command]
pub async fn backup_take(
    state: State<'_, AppState>,
    id: RuntimeId,
    session_id: Option<i64>,
    device_id: Option<i64>,
    hostname: Option<String>,
    source: BackupSource,
) -> Result<BackupInfo> {
    let command = match source {
        BackupSource::Startup => "show startup-config",
        _ => "show running-config",
    };
    let captured = state
        .sessions
        .capture(id, command, std::time::Duration::from_secs(180))
        .await?;
    let config = clean_capture(&captured);
    let source = if source == BackupSource::Startup {
        BackupSource::Startup
    } else {
        BackupSource::Running
    };
    store(&state, session_id, device_id, hostname, source, &config)
}

/// A config from a file / an undone delete, stored like a backup.
#[tauri::command]
pub fn backup_import(
    state: State<'_, AppState>,
    session_id: Option<i64>,
    device_id: Option<i64>,
    hostname: Option<String>,
    source: Option<BackupSource>,
    text: String,
) -> Result<BackupInfo> {
    if text.len() > blob::MAX_PLAIN as usize {
        return Err(AppError::invalid("config is too large"));
    }
    store(
        &state,
        session_id,
        device_id,
        hostname,
        source.unwrap_or(BackupSource::File),
        &text,
    )
}

#[tauri::command]
pub fn backups_list(
    state: State<'_, AppState>,
    session_id: Option<i64>,
    device_id: Option<i64>,
) -> Result<Vec<BackupInfo>> {
    state.db.list_backups(session_id, device_id)
}

/// The plain config of a backup (to show or compare).
#[tauri::command]
pub fn backup_text(state: State<'_, AppState>, id: i64) -> Result<String> {
    text_of(&state, id)
}

#[tauri::command]
pub fn backup_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_backup(id)
}

/// Writes a backup as plain text to a path the user picked (their explicit export).
#[tauri::command]
pub fn backup_export(state: State<'_, AppState>, id: i64, path: String) -> Result<()> {
    let text = text_of(&state, id)?;
    std::fs::write(&path, text).map_err(AppError::from)
}

/// Side-by-side diff of two configs (backups, running ↔ startup).
#[tauri::command]
pub fn config_diff(left: String, right: String) -> Result<Diff> {
    let max = blob::MAX_PLAIN as usize;
    if left.len() > max || right.len() > max {
        return Err(AppError::invalid("config is too large"));
    }
    Ok(diff(&left, &right))
}

/// Reads a config file the user picked (import).
#[tauri::command]
pub fn config_file_read(path: String) -> Result<String> {
    let meta = std::fs::metadata(&path).map_err(AppError::from)?;
    if meta.len() > blob::MAX_PLAIN {
        return Err(AppError::invalid("file is too large"));
    }
    let bytes = std::fs::read(&path).map_err(AppError::from)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Running ↔ startup config of a live session, both read hidden and cleaned.
#[tauri::command]
pub async fn config_live_diff(state: State<'_, AppState>, id: RuntimeId) -> Result<Diff> {
    let t = std::time::Duration::from_secs(180);
    let startup = clean_capture(&state.sessions.capture(id, "show startup-config", t).await?);
    let running = clean_capture(&state.sessions.capture(id, "show running-config", t).await?);
    Ok(diff(&startup, &running))
}

/// A stored backup ↔ the device's running config now.
#[tauri::command]
pub async fn backup_diff_live(
    state: State<'_, AppState>,
    backup_id: i64,
    id: RuntimeId,
) -> Result<Diff> {
    let stored = text_of(&state, backup_id)?;
    let t = std::time::Duration::from_secs(180);
    let running = clean_capture(&state.sessions.capture(id, "show running-config", t).await?);
    Ok(diff(&stored, &running))
}
