//! Folder / saved-session / settings commands.

use tauri::State;

use crate::error::Result;
use crate::model::{Folder, SessionInput, SessionPatch, SessionRecord};
use crate::secrets::forget_session;
use crate::state::AppState;
use crate::storage::{DeviceRecord, Note, NoteHit, PinnedNote};

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
pub fn folder_reorder(
    state: State<'_, AppState>,
    id: i64,
    parent_id: Option<i64>,
    before_id: Option<i64>,
) -> Result<()> {
    state.db.reorder_folder(id, parent_id, before_id)
}

#[tauri::command]
pub fn session_bulk_update(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    patch: SessionPatch,
) -> Result<Vec<SessionRecord>> {
    state.db.bulk_update_sessions(&ids, &patch)
}

#[tauri::command]
pub fn session_bulk_delete(state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize> {
    let n = state.db.delete_sessions(&ids)?;
    for id in ids {
        forget_session(state.secrets.as_ref(), id);
    }
    Ok(n)
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
    let rec = state.db.create_session(&input)?;
    forget_session(state.secrets.as_ref(), rec.id);
    Ok(rec)
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
    state.db.delete_session(id)?;
    forget_session(state.secrets.as_ref(), id);
    Ok(())
}

#[tauri::command]
pub fn session_duplicate(state: State<'_, AppState>, id: i64) -> Result<SessionRecord> {
    let rec = state.db.duplicate_session(id)?;
    forget_session(state.secrets.as_ref(), rec.id);
    Ok(rec)
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

/// Devices learned by the background identity command (P3-T04).
#[tauri::command]
pub fn device_list(state: State<'_, AppState>) -> Result<Vec<DeviceRecord>> {
    state.db.list_devices()
}

/// Notes of a tab: its device's (when known) plus its session's (P3-T08).
#[tauri::command]
pub fn notes_list(
    state: State<'_, AppState>,
    session_id: Option<i64>,
    device_id: Option<i64>,
) -> Result<Vec<Note>> {
    state.db.list_notes(session_id, device_id)
}

#[tauri::command]
pub fn note_create(
    state: State<'_, AppState>,
    session_id: Option<i64>,
    device_id: Option<i64>,
    body: String,
) -> Result<Note> {
    state.db.create_note(session_id, device_id, &body)
}

#[tauri::command]
pub fn note_update(state: State<'_, AppState>, id: i64, body: String) -> Result<Note> {
    state.db.update_note(id, &body)
}

#[tauri::command]
pub fn note_pin(state: State<'_, AppState>, id: i64, pinned: bool) -> Result<Note> {
    state.db.pin_note(id, pinned)
}

#[tauri::command]
pub fn note_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_note(id)
}

/// Sessions / devices whose notes match (session search).
#[tauri::command]
pub fn notes_search(state: State<'_, AppState>, text: String) -> Result<Vec<NoteHit>> {
    state.db.search_notes(&text)
}

/// Writes every note as Markdown to `path` (export).
#[tauri::command]
pub fn notes_export(state: State<'_, AppState>, path: String) -> Result<()> {
    let md = state.db.notes_markdown()?;
    std::fs::write(&path, md).map_err(crate::error::AppError::from)
}

/// Pinned notes' first lines (subtitles in the session list, P3-T09).
#[tauri::command]
pub fn notes_pinned(state: State<'_, AppState>) -> Result<Vec<PinnedNote>> {
    state.db.pinned_notes()
}
