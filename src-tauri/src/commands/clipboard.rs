//! Plain-text clipboard through the OS (arboard, text only). Reading from the webview
//! would pop a native "Paste" confirmation bubble on macOS for every right-click paste.

use crate::error::{AppError, ErrorCode, Result};

fn clipboard() -> Result<arboard::Clipboard> {
    arboard::Clipboard::new().map_err(|e| AppError::with(ErrorCode::Io, format!("clipboard: {e}")))
}

/// Returns the clipboard text ("" when it holds no text).
#[tauri::command]
pub fn clipboard_read() -> Result<String> {
    match clipboard()?.get_text() {
        Ok(t) => Ok(t),
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(e) => Err(AppError::with(ErrorCode::Io, format!("clipboard: {e}"))),
    }
}

#[tauri::command]
pub fn clipboard_write(text: String) -> Result<()> {
    clipboard()?
        .set_text(text)
        .map_err(|e| AppError::with(ErrorCode::Io, format!("clipboard: {e}")))
}
