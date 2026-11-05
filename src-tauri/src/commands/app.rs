//! App-level commands: lifecycle and build info.

use serde::Serialize;
use tauri::{AppHandle, WebviewWindow};

use crate::error::{AppError, Result};
use crate::{MAIN_WINDOW, PROCESS_START};

/// Env var: when set, `app_ready` prints the startup time to stdout
/// (`KENSHELL_READY_MS=<ms>`) so `bench/measure-app.mjs` can read it.
const BENCH_ENV: &str = "KENSHELL_BENCH";

/// Called once by each window's frontend after its first paint: shows that window.
#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
    if window.label() == MAIN_WINDOW && std::env::var_os(BENCH_ENV).is_some() {
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

/// Opens an external https link (e.g. official driver download page) in the system
/// browser. Only plain `https://` URLs are allowed.
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    if !is_safe_external_url(&url) {
        return Err(AppError::invalid("only https:// links can be opened"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::internal(e.to_string()))
}

fn is_safe_external_url(url: &str) -> bool {
    url.len() <= 2048
        && url.starts_with("https://")
        && url.len() > "https://".len()
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::is_safe_external_url;

    #[test]
    fn only_https_links_are_opened() {
        assert!(is_safe_external_url(
            "https://ftdichip.com/drivers/vcp-drivers/"
        ));
        assert!(!is_safe_external_url("http://example.com"));
        assert!(!is_safe_external_url("file:///etc/passwd"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("https://"));
        assert!(!is_safe_external_url("https://a.com/ x"));
    }
}
