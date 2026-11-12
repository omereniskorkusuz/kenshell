//! App-level commands: lifecycle and build info.

use serde::Serialize;
use tauri::{AppHandle, WebviewWindow};

use crate::error::{AppError, Result};
use crate::{MAIN_WINDOW, PROCESS_START};

/// Env var: when set, `app_ready` prints the startup time to stdout
/// (`KENSHELL_READY_MS=<ms>`) so `bench/measure-app.mjs` can read it.
const BENCH_ENV: &str = "KENSHELL_BENCH";
/// Bench only (with `KENSHELL_BENCH`): `"<quick connect text>|<count>"`, e.g.
/// `telnet 127.0.0.1 2323|10` — the main window opens that many tabs at start-up so
/// `bench/measure-app.mjs --sessions N` can measure RAM with live sessions.
const BENCH_OPEN_ENV: &str = "KENSHELL_BENCH_OPEN";
const BENCH_MAX_TABS: u32 = 50;
/// Self-check runs only (with `KENSHELL_BENCH`): the scenario, as JSON, that the frontend
/// plays against the device simulator (`bench/check-topology.mjs`).
const CHECK_ENV: &str = "KENSHELL_CHECK";
/// Self-check runs only: a throw-away data directory. With it the app also keeps secrets
/// in memory, so a check never reads or writes the user's database or keychain.
const CHECK_DATA_DIR_ENV: &str = "KENSHELL_CHECK_DATA_DIR";
const CHECK_REPORT_MAX: usize = 512 * 1024;

/// The throw-away data directory of a self-check run, if this is one.
pub fn check_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os(BENCH_ENV)?;
    std::env::var_os(CHECK_DATA_DIR_ENV)
        .filter(|d| !d.is_empty())
        .map(Into::into)
}

/// Self-check runs only: the frontend hands back what it saw, printed as one line
/// (`KENSHELL_CHECK_RESULT=<json>`) for the script that started the app. Outside a
/// bench run this does nothing.
#[tauri::command]
pub fn check_report(report: String) {
    if std::env::var_os(BENCH_ENV).is_none() || std::env::var_os(CHECK_ENV).is_none() {
        return;
    }
    let line: String = report
        .chars()
        .filter(|c| !c.is_control())
        .take(CHECK_REPORT_MAX)
        .collect();
    println!("KENSHELL_CHECK_RESULT={line}");
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchOpen {
    target: String,
    count: u32,
    /// Show each tab once (default); `KENSHELL_BENCH_VISIT=0` leaves them unrendered.
    visit: bool,
    /// Self-check scenario (JSON) to play instead, see `KENSHELL_CHECK`.
    check: Option<String>,
}

fn parse_bench_open(raw: &str) -> Option<BenchOpen> {
    let (target, count) = raw.rsplit_once('|')?;
    let count: u32 = count.trim().parse().ok()?;
    let target = target.trim();
    (!target.is_empty() && count > 0).then(|| BenchOpen {
        target: target.to_owned(),
        count: count.min(BENCH_MAX_TABS),
        visit: std::env::var("KENSHELL_BENCH_VISIT").map_or(true, |v| v != "0"),
        check: None,
    })
}

/// Called once by each window's frontend after its first paint: shows that window.
/// Returns the benchmark tabs to open (bench runs only, main window only).
#[tauri::command]
pub fn app_ready(window: WebviewWindow) -> Option<BenchOpen> {
    let _ = window.show();
    let _ = window.set_focus();
    if window.label() != MAIN_WINDOW || std::env::var_os(BENCH_ENV).is_none() {
        return None;
    }
    let ms = PROCESS_START
        .get()
        .map(|t| t.elapsed().as_millis())
        .unwrap_or_default();
    println!("KENSHELL_READY_MS={ms}");
    if let Ok(check) = std::env::var(CHECK_ENV) {
        return Some(BenchOpen {
            target: String::new(),
            count: 0,
            visit: false,
            check: Some(check),
        });
    }
    std::env::var(BENCH_OPEN_ENV)
        .ok()
        .and_then(|raw| parse_bench_open(&raw))
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
    use super::{BenchOpen, is_safe_external_url, parse_bench_open};

    #[test]
    fn bench_open_is_parsed_and_capped() {
        assert_eq!(
            parse_bench_open("telnet 127.0.0.1 2323|10"),
            Some(BenchOpen {
                target: "telnet 127.0.0.1 2323".into(),
                count: 10,
                visit: true,
                check: None,
            })
        );
        assert_eq!(parse_bench_open("ssh x|999").map(|b| b.count), Some(50));
        for bad in ["", "telnet x", "|3", "x|0", "x|ten"] {
            assert_eq!(parse_bench_open(bad), None, "{bad}");
        }
    }

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
