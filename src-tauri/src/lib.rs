//! KenShell backend entry point.

mod commands;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::Manager;

/// Instant captured as early as possible in `run()`; used for startup metrics.
pub(crate) static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Label of the single main window.
pub(crate) const MAIN_WINDOW: &str = "main";

/// The window is created hidden (avoids a white flash) and shown when the
/// frontend reports it has rendered. If that never happens we show it anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

pub fn run() {
    PROCESS_START.get_or_init(Instant::now);

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app::app_ready,
            commands::app::app_info
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(SHOW_FALLBACK);
                if let Some(w) = handle.get_webview_window(MAIN_WINDOW)
                    && !w.is_visible().unwrap_or(true)
                {
                    let _ = w.show();
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running KenShell");
}
