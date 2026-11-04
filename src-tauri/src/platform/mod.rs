//! Platform-specific helpers. Keep OS conditionals here, not in domain modules.
//! P1-T11 adds `classify_open_error` (busy / not found / permission → friendly AppError).
#![allow(unused_imports)] // TODO(P1-T11)

pub use self::imp::*;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "linux.rs"]
mod imp;
