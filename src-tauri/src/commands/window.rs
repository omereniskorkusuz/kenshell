//! Detached session windows (P2-T19): move a live tab into its own window.
//!
//! Flow: old window `live_park` → waits for the end markers → `window_detach` (this file)
//! → new window `window_adopt` at startup → `live_attach` with its own channels.

use serde::Deserialize;
use tauri::{AppHandle, State, WebviewUrl, WebviewWindowBuilder, Window};

use crate::error::{AppError, Result};
use crate::handoff::{Handoff, window_title};
use crate::model::RuntimeId;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachRequest {
    runtime_id: RuntimeId,
    title: String,
    /// Opaque tab snapshot built by the frontend.
    payload: String,
    /// Screen position (logical px) where the tab was dropped, if dragged out.
    x: Option<f64>,
    y: Option<f64>,
}

const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 680.0;

/// Opens a new window for a parked session; returns its label. On failure nothing is
/// left behind and the caller re-attaches the session.
#[tauri::command]
pub fn window_detach(
    app: AppHandle,
    window: Window,
    state: State<'_, AppState>,
    request: DetachRequest,
) -> Result<String> {
    let label = state.handoffs.next_label();
    state.handoffs.put(
        &label,
        Handoff {
            runtime_id: request.runtime_id,
            payload: request.payload,
        },
    )?;
    // From here on the session belongs to the new window: if it dies, the session closes.
    if let Err(e) = state.sessions.set_owner(request.runtime_id, &label) {
        state.handoffs.take(&label);
        return Err(e);
    }

    let mut builder =
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html#detached".into()))
            .title(window_title(&request.title))
            .inner_size(WIDTH, HEIGHT)
            .min_inner_size(720.0, 480.0)
            .visible(false)
            .background_color(tauri::window::Color(15, 17, 21, 255));
    if let (Some(x), Some(y)) = (request.x, request.y) {
        // Center the title bar under the pointer, like dragging a browser tab out.
        builder = builder.position((x - WIDTH / 2.0).max(0.0), (y - 16.0).max(0.0));
    }
    match builder.build() {
        Ok(_) => {
            crate::show_later(app.clone(), label.clone());
            Ok(label)
        }
        Err(e) => {
            state.handoffs.take(&label);
            let _ = state.sessions.set_owner(request.runtime_id, window.label());
            Err(AppError::internal(format!("new window: {e}")))
        }
    }
}

/// The tab handed to the calling window, if any (detached windows call this once).
#[tauri::command]
pub fn window_adopt(window: Window, state: State<'_, AppState>) -> Option<Handoff> {
    state.handoffs.take(window.label())
}
