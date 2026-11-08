//! App updates (P5-T10, ADR-021): "ask, then update". The updater plugin is registered
//! only in builds made with an updater config (release workflow, `--config`); in other
//! builds every command answers "not configured" and nothing is ever fetched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use tauri::AppHandle;
use tauri::ipc::Channel;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::{AppError, ErrorCode, Result};

static CONFIGURED: AtomicBool = AtomicBool::new(false);
static PENDING: LazyLock<Mutex<Option<Update>>> = LazyLock::new(Default::default);

/// Registers the updater when this build carries its config (endpoint + public key).
pub fn install_plugin(app: &tauri::App) -> tauri::Result<()> {
    if app.config().plugins.0.contains_key("updater") {
        app.handle()
            .plugin(tauri_plugin_updater::Builder::new().build())?;
        CONFIGURED.store(true, Ordering::Relaxed);
    }
    Ok(())
}

fn not_configured() -> AppError {
    AppError::with(
        ErrorCode::Unsupported,
        "updates are not configured in this build",
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub configured: bool,
    pub current: String,
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    UpdateStatus {
        configured: CONFIGURED.load(Ordering::Relaxed),
        current: app.package_info().version.to_string(),
    }
}

/// Asks the update server; `None` when this is the newest version.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>> {
    if !CONFIGURED.load(Ordering::Relaxed) {
        return Err(not_configured());
    }
    let updater = app
        .updater()
        .map_err(|e| AppError::with(ErrorCode::Io, format!("updater: {e}")))?;
    let update = updater
        .check()
        .await
        .map_err(|e| AppError::with(ErrorCode::Io, format!("update check: {e}")))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    if let Ok(mut p) = PENDING.lock() {
        *p = update;
    }
    Ok(info)
}

/// Downloads (signature verified by the plugin) and installs the update found by
/// `update_check`, reporting `(done, total)`, then restarts the app.
#[tauri::command]
pub async fn update_install(
    app: AppHandle,
    on_progress: Channel<(u64, Option<u64>)>,
) -> Result<()> {
    let update = PENDING
        .lock()
        .ok()
        .and_then(|mut p| p.take())
        .ok_or_else(|| AppError::invalid("check for updates first"))?;
    let mut done = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = on_progress.send((done, total));
            },
            || {},
        )
        .await
        .map_err(|e| AppError::with(ErrorCode::Io, format!("update install: {e}")))?;
    app.restart();
}
