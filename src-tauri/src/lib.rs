//! KenShell backend entry point.

mod commands;
mod console_detect;
mod error;
mod events;
mod logging;
mod model;
mod platform;
mod secrets;
mod session;
mod state;
mod storage;
mod transport;

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

use crate::console_detect::ConsoleRegistry;
use crate::session::SessionManager;
use crate::state::AppState;
use crate::storage::Db;

/// Instant captured as early as possible in `run()`; used for startup metrics.
pub(crate) static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Label of the single main window.
pub(crate) const MAIN_WINDOW: &str = "main";

/// The window is created hidden (avoids a white flash) and shown when the
/// frontend reports it has rendered. If that never happens we show it anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

const DB_FILE: &str = "kenshell.db";

pub fn run() {
    PROCESS_START.get_or_init(Instant::now);
    init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::app::app_ready,
            commands::app::app_info,
            commands::app::open_external,
            commands::clipboard::clipboard_read,
            commands::clipboard::clipboard_write,
            commands::storage::folder_list,
            commands::storage::folder_create,
            commands::storage::folder_rename,
            commands::storage::folder_move,
            commands::storage::folder_delete,
            commands::storage::session_list,
            commands::storage::session_create,
            commands::storage::session_update,
            commands::storage::session_delete,
            commands::storage::session_duplicate,
            commands::storage::settings_all,
            commands::storage::setting_set,
            commands::console::console_list,
            commands::console::console_rescan,
            commands::session::live_open,
            commands::session::live_write,
            commands::session::live_resize,
            commands::session::live_break,
            commands::session::live_close,
            commands::session::live_reconnect,
            commands::session::live_answer,
            commands::session::known_host_forget,
            commands::secrets::profile_list,
            commands::secrets::profile_create,
            commands::secrets::profile_update,
            commands::secrets::profile_set_password,
            commands::secrets::profile_delete,
            commands::secrets::session_password_set,
            commands::secrets::session_password_exists,
        ])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Arc::new(Db::open(&dir.join(DB_FILE))?);
            let consoles = Arc::new(ConsoleRegistry::new(Arc::clone(&db)));
            let secrets: Arc<dyn secrets::SecretStore> = Arc::new(secrets::Keychain);
            let sessions = Arc::new(SessionManager::new(
                Arc::clone(&db),
                Arc::clone(&consoles),
                Arc::clone(&secrets),
            ));

            let handle = app.handle().clone();
            let sessions_for_watch = Arc::clone(&sessions);
            consoles.start_watch(Box::new(move |change| {
                sessions_for_watch.on_console_change(&change);
                if let Err(e) = handle.emit(events::CONSOLE_CHANGED, &change) {
                    tracing::warn!("emit console change: {e}");
                }
            }));

            app.manage(AppState {
                db,
                consoles,
                sessions,
                secrets,
            });

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

/// `KENSHELL_LOG=debug` (env-filter syntax) controls verbosity; default `warn`.
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("KENSHELL_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
