//! Tauri command surface (thin layer; real logic lives in domain modules).
//! Every command returns `Result<_, AppError>` so the frontend gets `{code, detail}`.

pub mod app;
pub mod clipboard;
pub mod console;
pub mod session;
pub mod storage;
