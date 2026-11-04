//! SQLite storage: single file in the app data dir, WAL mode, versioned migrations,
//! repository methods on [`Db`]. Owner: P1-T04/T05 (+ port memory P1-T13).
//!
//! CONTRACT: the public method signatures below are used by commands and other modules.
//! Implementations may move into submodules (`migrations.rs`, `folders.rs`, …) but the
//! signatures must stay.
#![allow(unused_variables)] // TODO(P1-T04): remove once implemented

use std::path::Path;
use std::sync::Mutex;

use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Folder, PortMemory, SessionInput, SessionRecord};

pub struct Db {
    #[allow(dead_code)] // TODO(P1-T04)
    conn: Mutex<rusqlite::Connection>,
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::new(ErrorCode::NotFound),
            other => AppError::with(ErrorCode::Db, other.to_string()),
        }
    }
}

impl Db {
    /// Opens (creates) the database file, enables WAL + foreign keys, runs migrations.
    pub fn open(path: &Path) -> Result<Self> {
        todo!("P1-T04")
    }

    /// In-memory database with all migrations applied (tests).
    pub fn open_in_memory() -> Result<Self> {
        todo!("P1-T04")
    }

    // ---- folders
    pub fn list_folders(&self) -> Result<Vec<Folder>> {
        todo!("P1-T05")
    }
    pub fn create_folder(&self, name: &str, parent_id: Option<i64>) -> Result<Folder> {
        todo!("P1-T05")
    }
    pub fn rename_folder(&self, id: i64, name: &str) -> Result<()> {
        todo!("P1-T05")
    }
    /// Moves a folder under `parent_id` (None = root). Rejects cycles with `Invalid`.
    pub fn move_folder(&self, id: i64, parent_id: Option<i64>) -> Result<()> {
        todo!("P1-T05")
    }
    /// Deletes a folder; its sessions and sub-folders move to the folder's parent.
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        todo!("P1-T05")
    }

    // ---- sessions
    pub fn list_sessions(&self) -> Result<Vec<SessionRecord>> {
        todo!("P1-T05")
    }
    pub fn get_session(&self, id: i64) -> Result<SessionRecord> {
        todo!("P1-T05")
    }
    pub fn create_session(&self, input: &SessionInput) -> Result<SessionRecord> {
        todo!("P1-T05")
    }
    pub fn update_session(&self, id: i64, input: &SessionInput) -> Result<SessionRecord> {
        todo!("P1-T05")
    }
    pub fn delete_session(&self, id: i64) -> Result<()> {
        todo!("P1-T05")
    }
    /// Copy with name suffix " (2)", " (3)" … in the same folder.
    pub fn duplicate_session(&self, id: i64) -> Result<SessionRecord> {
        todo!("P1-T05")
    }
    /// Sets `last_connected_at` = now.
    pub fn touch_session(&self, id: i64) -> Result<()> {
        todo!("P1-T05")
    }

    // ---- settings (JSON values)
    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        todo!("P1-T04")
    }
    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        todo!("P1-T04")
    }
    pub fn all_settings(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        todo!("P1-T04")
    }

    // ---- port memory
    pub fn get_port_memory(&self, key: &str) -> Result<Option<PortMemory>> {
        todo!("P1-T13")
    }
    /// Insert or replace by `key`; sets `updated_at` = now.
    pub fn upsert_port_memory(&self, mem: &PortMemory) -> Result<()> {
        todo!("P1-T13")
    }
}

/// Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
