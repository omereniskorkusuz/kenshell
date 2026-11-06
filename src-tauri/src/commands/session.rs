//! Live session commands. Output flows through two Tauri channels created by the
//! frontend: raw bytes (`ArrayBuffer`) and JSON `SessionEvent`s.

use std::sync::Arc;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{State, Window};

use crate::error::Result;
use crate::model::{OpenRequest, RuntimeId, SessionEvent};
use crate::session::SessionOutput;
use crate::state::AppState;

struct ChannelOutput {
    data: Channel<InvokeResponseBody>,
    events: Channel<SessionEvent>,
}

impl SessionOutput for ChannelOutput {
    fn data(&self, bytes: Vec<u8>) {
        if let Err(e) = self.data.send(InvokeResponseBody::Raw(bytes)) {
            tracing::debug!("session data channel closed: {e}");
        }
    }

    fn event(&self, event: SessionEvent) {
        if let Err(e) = self.events.send(event) {
            tracing::debug!("session event channel closed: {e}");
        }
    }
}

#[tauri::command]
pub fn live_open(
    window: Window,
    state: State<'_, AppState>,
    request: OpenRequest,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<SessionEvent>,
) -> Result<RuntimeId> {
    let out = Arc::new(ChannelOutput {
        data: on_data,
        events: on_event,
    });
    let id = state.sessions.open(request, out)?;
    state.sessions.set_owner(id, window.label())?;
    Ok(id)
}

/// Holds the session's output while its tab moves to another window (P2-T19). The
/// current channels receive end markers: an empty data batch and a `parked` event.
#[tauri::command]
pub fn live_park(state: State<'_, AppState>, id: RuntimeId) -> Result<()> {
    state.sessions.park(id)
}

/// Routes the session's output (held output first) to the calling window's channels.
#[tauri::command]
pub fn live_attach(
    window: Window,
    state: State<'_, AppState>,
    id: RuntimeId,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<SessionEvent>,
) -> Result<()> {
    let out = Arc::new(ChannelOutput {
        data: on_data,
        events: on_event,
    });
    state.sessions.attach(id, out, window.label())
}

/// `data` is what the terminal produced (UTF-8 text; charset mapping comes in P2-T18).
#[tauri::command]
pub fn live_write(state: State<'_, AppState>, id: RuntimeId, data: String) -> Result<()> {
    state.sessions.write(id, data.as_bytes())
}

/// Runs a read-only `show` / `display` command hidden from the terminal and returns its
/// output (diagnostics, config backup). `timeout_secs`: 1–300 (default 30).
#[tauri::command]
pub async fn live_capture(
    state: State<'_, AppState>,
    id: RuntimeId,
    command: String,
    timeout_secs: Option<u64>,
) -> Result<String> {
    let secs = timeout_secs.unwrap_or(30).clamp(1, 300);
    state
        .sessions
        .capture(id, &command, std::time::Duration::from_secs(secs))
        .await
}

#[tauri::command]
pub fn live_resize(state: State<'_, AppState>, id: RuntimeId, cols: u16, rows: u16) -> Result<()> {
    state.sessions.resize(id, cols, rows)
}

#[tauri::command]
pub fn live_break(state: State<'_, AppState>, id: RuntimeId) -> Result<()> {
    state.sessions.send_break(id)
}

#[tauri::command]
pub fn live_close(state: State<'_, AppState>, id: RuntimeId) -> Result<()> {
    state.sessions.close(id)
}

#[tauri::command]
pub fn live_reconnect(state: State<'_, AppState>, id: RuntimeId) -> Result<()> {
    state.sessions.reconnect(id)
}

/// Answers the pending login prompt. `answers` may contain passwords: never logged.
#[tauri::command]
pub fn live_answer(state: State<'_, AppState>, id: RuntimeId, answers: Vec<String>) -> Result<()> {
    state.sessions.answer(id, answers)
}

/// Forgets a stored SSH host key (after a legitimate key change, e.g. device replaced).
/// The next connection learns the new key.
#[tauri::command]
pub fn known_host_forget(state: State<'_, AppState>, host: String, port: u16) -> Result<()> {
    state.db.forget_known_host(&host, port)
}

// ------------------------------------------------------- Recording / export (P5-T06)

/// Recordings go to the log folder (`log.dir`, else the default) even when logging
/// itself is off.
fn record_dir(state: &AppState) -> std::path::PathBuf {
    state
        .db
        .get_setting("log.dir")
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.trim().is_empty())
        .map_or_else(crate::logging::default_dir, Into::into)
}

/// Starts recording a live session's output (asciicast v2); returns the file path.
#[tauri::command]
pub fn live_record_start(
    state: State<'_, AppState>,
    id: RuntimeId,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<String> {
    let dir = record_dir(&state);
    state.sessions.record_start(id, &dir, &name, cols, rows)
}

#[tauri::command]
pub fn live_record_stop(state: State<'_, AppState>, id: RuntimeId) -> Result<Option<String>> {
    state.sessions.record_stop(id)
}

/// A recording's contents for the replay player.
#[tauri::command]
pub async fn recording_read(path: String) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::logging::record::read(std::path::Path::new(&path))
    })
    .await
    .map_err(|_| crate::error::AppError::internal("read task"))?
}

/// Writes exported terminal text / HTML to a path the user chose in the save dialog.
#[tauri::command]
pub async fn text_export(path: String, text: String) -> Result<()> {
    tokio::fs::write(&path, text)
        .await
        .map_err(crate::error::AppError::from)
}
