//! Console detection commands. Changes are pushed as event `console://changed`.

use tauri::State;

use crate::model::ConsoleSnapshot;
use crate::state::AppState;

#[tauri::command]
pub fn console_list(state: State<'_, AppState>) -> ConsoleSnapshot {
    state.consoles.snapshot()
}

/// Manual refresh (e.g. after installing a driver).
#[tauri::command]
pub fn console_rescan(state: State<'_, AppState>) -> ConsoleSnapshot {
    state.consoles.rescan();
    state.consoles.snapshot()
}

/// Linux: whether this user may open serial devices (group `dialout` / `uucp`, P5-T12).
#[tauri::command]
pub fn serial_access() -> crate::platform::serial_access::SerialAccess {
    crate::platform::serial_access::check()
}
