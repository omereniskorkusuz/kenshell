//! Live session commands. Output flows through two Tauri channels created by the
//! frontend: raw bytes (`ArrayBuffer`) and JSON `SessionEvent`s.

use std::sync::Arc;

use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody};

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
    state: State<'_, AppState>,
    request: OpenRequest,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<SessionEvent>,
) -> Result<RuntimeId> {
    let out = Arc::new(ChannelOutput {
        data: on_data,
        events: on_event,
    });
    state.sessions.open(request, out)
}

/// `data` is what the terminal produced (UTF-8 text; charset mapping comes in P2-T18).
#[tauri::command]
pub fn live_write(state: State<'_, AppState>, id: RuntimeId, data: String) -> Result<()> {
    state.sessions.write(id, data.as_bytes())
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
