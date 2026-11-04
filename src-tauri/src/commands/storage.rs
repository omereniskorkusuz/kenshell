//! Folder / saved-session / settings commands.

use tauri::State;

use crate::error::Result;
use crate::model::{Folder, SessionInput, SessionRecord};
use crate::state::AppState;

#[tauri::command]
pub fn folder_list(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    state.db.list_folders()
}

#[tauri::command]
pub fn folder_create(
    state: State<'_, AppState>,
    name: String,
    parent_id: Option<i64>,
) -> Result<Folder> {
    state.db.create_folder(&name, parent_id)
}

#[tauri::command]
pub fn folder_rename(state: State<'_, AppState>, id: i64, name: String) -> Result<()> {
    state.db.rename_folder(id, &name)
}

#[tauri::command]
pub fn folder_move(state: State<'_, AppState>, id: i64, parent_id: Option<i64>) -> Result<()> {
    state.db.move_folder(id, parent_id)
}

#[tauri::command]
pub fn folder_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_folder(id)
}

#[tauri::command]
pub fn session_list(state: State<'_, AppState>) -> Result<Vec<SessionRecord>> {
    state.db.list_sessions()
}

#[tauri::command]
pub fn session_create(state: State<'_, AppState>, input: SessionInput) -> Result<SessionRecord> {
    state.db.create_session(&input)
}

#[tauri::command]
pub fn session_update(
    state: State<'_, AppState>,
    id: i64,
    input: SessionInput,
) -> Result<SessionRecord> {
    state.db.update_session(id, &input)
}

#[tauri::command]
pub fn session_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_session(id)
}

#[tauri::command]
pub fn session_duplicate(state: State<'_, AppState>, id: i64) -> Result<SessionRecord> {
    state.db.duplicate_session(id)
}

#[tauri::command]
pub fn settings_all(
    state: State<'_, AppState>,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    state.db.all_settings()
}

#[tauri::command]
pub fn setting_set(
    state: State<'_, AppState>,
    key: String,
    value: serde_json::Value,
) -> Result<()> {
    state.db.set_setting(&key, &value)
}

/// Drag & drop: put a session into `folder_id`, right before `before_id` (or last).
#[tauri::command]
pub fn session_reorder(
    state: State<'_, AppState>,
    id: i64,
    folder_id: Option<i64>,
    before_id: Option<i64>,
) -> Result<()> {
    state.db.reorder_session(id, folder_id, before_id)
}
