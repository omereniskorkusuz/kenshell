//! Credential profiles and saved passwords. Passwords go straight to the OS keychain;
//! they are never stored in SQLite, logged, or returned to the frontend.

use tauri::State;

use crate::error::{AppError, Result};
use crate::model::CredentialProfile;
use crate::secrets::backup::{self, RestoreReport};
use crate::secrets::{profile_account, session_account};
use crate::state::AppState;

fn with_flag(state: &AppState, mut p: CredentialProfile) -> CredentialProfile {
    p.has_password = matches!(state.secrets.get(&profile_account(p.id)), Ok(Some(_)));
    p
}

#[tauri::command]
pub fn profile_list(state: State<'_, AppState>) -> Result<Vec<CredentialProfile>> {
    Ok(state
        .db
        .list_profiles()?
        .into_iter()
        .map(|p| with_flag(&state, p))
        .collect())
}

#[tauri::command]
pub fn profile_create(
    state: State<'_, AppState>,
    name: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<CredentialProfile> {
    let p = state.db.create_profile(&name, username.as_deref())?;
    if let Some(pw) = password.filter(|p| !p.is_empty())
        && let Err(e) = state.secrets.set(&profile_account(p.id), &pw)
    {
        // Keep the DB consistent with the keychain.
        let _ = state.db.delete_profile(p.id);
        return Err(e);
    }
    Ok(with_flag(&state, p))
}

#[tauri::command]
pub fn profile_update(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    username: Option<String>,
) -> Result<CredentialProfile> {
    let p = state.db.update_profile(id, &name, username.as_deref())?;
    Ok(with_flag(&state, p))
}

/// `password: null` (or empty) forgets the saved password.
#[tauri::command]
pub fn profile_set_password(
    state: State<'_, AppState>,
    id: i64,
    password: Option<String>,
) -> Result<()> {
    state.db.get_profile(id)?;
    match password.filter(|p| !p.is_empty()) {
        Some(pw) => state.secrets.set(&profile_account(id), &pw),
        None => state.secrets.delete(&profile_account(id)),
    }
}

#[tauri::command]
pub fn profile_delete(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.delete_profile(id)?;
    state.secrets.delete(&profile_account(id))
}

/// Saves (or with `null` forgets) the password of one saved session.
#[tauri::command]
pub fn session_password_set(
    state: State<'_, AppState>,
    id: i64,
    password: Option<String>,
) -> Result<()> {
    state.db.get_session(id)?;
    match password.filter(|p| !p.is_empty()) {
        Some(pw) => state.secrets.set(&session_account(id), &pw),
        None => state.secrets.delete(&session_account(id)),
    }
}

#[tauri::command]
pub fn session_password_exists(state: State<'_, AppState>, id: i64) -> Result<bool> {
    Ok(state.secrets.get(&session_account(id))?.is_some())
}

/// Writes an encrypted backup (library + saved passwords) to `path`.
#[tauri::command]
pub async fn vault_export(
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> Result<()> {
    let (db, store) = (state.db.clone(), state.secrets.clone());
    let password = zeroize::Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp_millis();
        let data = backup::export(&db, store.as_ref(), &password, now)?;
        std::fs::write(&path, data)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// Restores an encrypted backup from `path` (merges; never overwrites).
#[tauri::command]
pub async fn vault_import(
    state: State<'_, AppState>,
    path: String,
    password: String,
) -> Result<RestoreReport> {
    let (db, store) = (state.db.clone(), state.secrets.clone());
    let password = zeroize::Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path)?;
        if meta.len() > MAX_VAULT_BYTES {
            return Err(AppError::invalid("file is too large for a backup"));
        }
        let data = std::fs::read(&path)?;
        backup::restore(&db, store.as_ref(), &password, &data)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

const MAX_VAULT_BYTES: u64 = 64 * 1024 * 1024;
