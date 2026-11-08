//! Optional AI assistant (P5-T08): only the user's API key lives here, in the OS
//! keychain — never in the DB, logs or exports. The requests themselves run in the
//! assistant's lazily loaded frontend chunk (ADR-020).

use tauri::State;

use crate::error::{AppError, Result};
use crate::state::AppState;

const ACCOUNT: &str = "ai-api-key";

fn valid(key: &str) -> bool {
    (8..=400).contains(&key.len()) && key.chars().all(|c| c.is_ascii_graphic())
}

/// True when a key is stored.
#[tauri::command]
pub fn ai_key_status(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.secrets.get(ACCOUNT)?.is_some())
}

#[tauri::command]
pub fn ai_key_set(state: State<'_, AppState>, key: String) -> Result<()> {
    let key = key.trim();
    if !valid(key) {
        return Err(AppError::invalid("API key looks wrong"));
    }
    state.secrets.set(ACCOUNT, key)
}

#[tauri::command]
pub fn ai_key_clear(state: State<'_, AppState>) -> Result<()> {
    state.secrets.delete(ACCOUNT)
}

/// The key, for the assistant chunk at the moment it sends a request.
#[tauri::command]
pub fn ai_key_get(state: State<'_, AppState>) -> Result<Option<String>> {
    Ok(state.secrets.get(ACCOUNT)?.map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn key_shape() {
        assert!(super::valid("sk-ant-api03-abcdefgh"));
        assert!(!super::valid("short"));
        assert!(!super::valid("sk-ant with space"));
        assert!(!super::valid("sk-ant-\n-newline"));
    }
}
