//! App-level commands: lifecycle and build info.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{MAIN_WINDOW, PROCESS_START};

/// Env var: when set, `app_ready` prints the startup time to stdout
/// (`KENSHELL_READY_MS=<ms>`) so `bench/measure-app.mjs` can read it.
const BENCH_ENV: &str = "KENSHELL_BENCH";

/// Called once by the frontend after its first paint.
#[tauri::command]
pub fn app_ready(app: AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if std::env::var_os(BENCH_ENV).is_some() {
        let ms = PROCESS_START
            .get()
            .map(|t| t.elapsed().as_millis())
            .unwrap_or_default();
        println!("KENSHELL_READY_MS={ms}");
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}
