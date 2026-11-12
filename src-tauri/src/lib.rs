//! KenShell backend entry point.

mod analyzer;
mod commands;
mod console_detect;
mod device;
mod diag;
mod error;
mod events;
mod flow;
mod handoff;
mod import;
mod keys;
mod logging;
mod model;
mod netscan;
mod platform;
mod secrets;
mod session;
mod snmp;
mod state;
mod storage;
mod transfer;
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

/// Label of the main window (detached session windows are `session-N`).
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
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::app::app_ready,
            commands::app::check_report,
            commands::app::app_info,
            commands::app::open_external,
            commands::clipboard::clipboard_read,
            commands::clipboard::clipboard_write,
            commands::storage::folder_list,
            commands::storage::folder_create,
            commands::storage::folder_rename,
            commands::storage::folder_move,
            commands::storage::folder_delete,
            commands::storage::folder_reorder,
            commands::storage::session_bulk_update,
            commands::storage::session_bulk_delete,
            commands::storage::device_list,
            commands::storage::notes_list,
            commands::storage::note_create,
            commands::storage::note_update,
            commands::storage::note_pin,
            commands::storage::note_delete,
            commands::storage::notes_search,
            commands::storage::notes_export,
            commands::storage::notes_pinned,
            commands::storage::history_list,
            commands::analyzer::analyze_output,
            commands::analyzer::analyzer_parsers,
            commands::analyzer::analyzer_export_csv,
            commands::analyzer::diagnostics_commands,
            commands::analyzer::diagnostics_evaluate,
            commands::analyzer::config_audit,
            commands::analyzer::logfile_analyze,
            commands::analyzer::logfile_lines,
            commands::analyzer::neighbors_discover,
            commands::analyzer::topology_discover,
            commands::backups::backup_take,
            commands::backups::backup_import,
            commands::backups::backups_list,
            commands::backups::backup_text,
            commands::backups::backup_delete,
            commands::backups::backup_export,
            commands::backups::config_diff,
            commands::backups::config_file_read,
            commands::netscan::netscan_networks,
            commands::netscan::oui_lookup,
            commands::console::serial_access,
            commands::update::update_status,
            commands::update::update_check,
            commands::update::update_install,
            commands::topology::snmp_community_status,
            commands::topology::snmp_community_set,
            commands::topology::snmp_community_clear,
            commands::topology::topology_snmp,
            commands::topology::flow_start,
            commands::topology::flow_stop,
            commands::topology::flow_snapshot,
            commands::ai::ai_key_status,
            commands::ai::ai_key_set,
            commands::ai::ai_key_clear,
            commands::ai::ai_key_get,
            commands::diag::diag_run,
            commands::diag::diag_stop,
            commands::netscan::netscan_check,
            commands::netscan::netscan_run,
            commands::netscan::netscan_cancel,
            commands::netscan::open_device_web,
            commands::transfer::scp_download,
            commands::transfer::scp_upload,
            commands::transfer::transfer_cancel,
            commands::transfer::sftp_open,
            commands::transfer::sftp_list,
            commands::transfer::sftp_download,
            commands::transfer::sftp_upload,
            commands::transfer::sftp_mkdir,
            commands::transfer::sftp_remove,
            commands::transfer::sftp_rename,
            commands::transfer::sftp_close,
            commands::transfer::server_start,
            commands::transfer::server_stop,
            commands::transfer::server_status,
            commands::backups::config_live_diff,
            commands::backups::backup_diff_live,
            commands::storage::snippets_list,
            commands::storage::snippet_create,
            commands::storage::snippet_update,
            commands::storage::snippet_delete,
            commands::storage::history_add,
            commands::storage::history_clear,
            commands::storage::session_list,
            commands::storage::session_create,
            commands::storage::session_update,
            commands::storage::session_delete,
            commands::storage::session_duplicate,
            commands::storage::session_reorder,
            commands::storage::settings_all,
            commands::storage::setting_set,
            commands::console::console_list,
            commands::console::console_rescan,
            commands::session::live_open,
            commands::session::live_write,
            commands::session::live_capture,
            commands::session::live_resize,
            commands::session::live_record_start,
            commands::session::live_record_stop,
            commands::session::recording_read,
            commands::session::text_export,
            commands::session::live_break,
            commands::session::live_close,
            commands::session::live_reconnect,
            commands::session::live_answer,
            commands::session::live_park,
            commands::session::live_attach,
            commands::window::window_detach,
            commands::window::window_adopt,
            commands::session::known_host_forget,
            commands::import::import_detect,
            commands::import::import_sessions,
            commands::import::export_sessions,
            commands::import::pick_path,
            commands::secrets::profile_list,
            commands::secrets::profile_create,
            commands::secrets::vault_export,
            commands::secrets::vault_import,
            commands::keys::key_generate,
            commands::keys::key_inspect,
            commands::keys::key_convert,
            commands::secrets::profile_update,
            commands::secrets::profile_set_password,
            commands::secrets::profile_delete,
            commands::secrets::session_password_set,
            commands::secrets::session_password_exists,
        ])
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                tracing::debug!(window = window.label(), ?event, "window event");
            }
            // macOS: destroying a detached window from inside the close-button click (Tauri's
            // default) can leave its frame on screen when its terminal was live (seen
            // 2026-11-05; not with AXPress). Destroy it a moment later instead.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label().starts_with(handoff::WINDOW_PREFIX)
            {
                api.prevent_close();
                let w = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    if let Err(e) = w.destroy() {
                        tracing::warn!("destroy window: {e}");
                    }
                });
            }
            if let tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed = event
                && let Some(state) = window.try_state::<AppState>()
            {
                let closed = state.sessions.close_owned_by(window.label());
                state.handoffs.take(window.label());
                if closed > 0 {
                    tracing::debug!(window = window.label(), closed, "window closed");
                }
            }
        })
        .setup(|app| {
            commands::update::install_plugin(app)?;
            // A self-check run works in a throw-away directory with in-memory secrets.
            let check_dir = commands::app::check_data_dir();
            let dir = match &check_dir {
                Some(d) => d.clone(),
                None => app.path().app_data_dir()?,
            };
            std::fs::create_dir_all(&dir)?;
            let db = Arc::new(Db::open(&dir.join(DB_FILE))?);
            let consoles = Arc::new(ConsoleRegistry::new(Arc::clone(&db)));
            let secrets: Arc<dyn secrets::SecretStore> = if check_dir.is_some() {
                Arc::new(secrets::MemoryStore::default())
            } else {
                Arc::new(secrets::Keychain)
            };
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
                handoffs: handoff::Handoffs::default(),
                flow: std::sync::Arc::default(),
            });

            show_later(app.handle().clone(), MAIN_WINDOW.to_owned());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running KenShell");
}

/// Shows window `label` after [`SHOW_FALLBACK`] if its frontend never reported ready.
pub(crate) fn show_later(handle: tauri::AppHandle, label: String) {
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        if let Some(w) = handle.get_webview_window(&label)
            && !w.is_visible().unwrap_or(true)
        {
            let _ = w.show();
        }
    });
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
