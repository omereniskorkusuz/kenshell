//! Tauri command surface (thin layer; real logic lives in domain modules).
//! Every command returns `Result<_, AppError>` so the frontend gets `{code, detail}`.

pub mod ai;
pub mod analyzer;
pub mod app;
pub mod backups;
pub mod clipboard;
pub mod console;
pub mod diag;
pub mod import;
pub mod keys;
pub mod netscan;
pub mod secrets;
pub mod session;
pub mod storage;
pub mod update;
pub mod transfer;
pub mod window;
