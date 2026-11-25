//! Network scanner commands (P4-T12…T14). The scope rules live in `netscan::scope` and are
//! applied to every request here (the UI only mirrors them for display).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::ipc::Channel;

use tauri::State;

use crate::netscan::probe::{self, NetProber, Options, Plan, Progress, ScanResult};
use crate::netscan::scope::{self, LocalNet, Range, ScopeError};
use crate::state::AppState;
use crate::storage::ScanSummary;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Networks {
    pub networks: Vec<NetworkView>,
    /// The first network's default target (its /24, or the subnet when smaller).
    pub default_target: Option<String>,
    pub max_addresses: u32,
    /// Up to this many addresses every address also gets TCP connects.
    pub full_tcp_limit: u64,
    pub concurrency: usize,
    pub ports: Vec<u16>,
    /// Prefixes in the embedded MAC vendor table (P4-T15).
    pub oui_size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    #[serde(flatten)]
    pub net: LocalNet,
    pub cidr: String,
    pub default_target: String,
}

/// This computer's connected private / link-local subnets (routed private networks can be
/// typed in as well).
#[tauri::command]
pub fn netscan_networks() -> Networks {
    let networks: Vec<NetworkView> = scope::local_networks()
        .into_iter()
        .map(|net| NetworkView {
            cidr: net.cidr(),
            default_target: scope::default_target(&net),
            net,
        })
        .collect();
    Networks {
        default_target: networks.first().map(|n| n.default_target.clone()),
        networks,
        max_addresses: scope::MAX_ADDRESSES,
        full_tcp_limit: scope::FULL_TCP_LIMIT,
        concurrency: scope::CONCURRENCY,
        ports: scope::PORTS.to_vec(),
        oui_size: crate::netscan::oui::size(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OuiLookup {
    pub vendor: Option<&'static str>,
    pub table_size: usize,
}

/// MAC vendor from the embedded OUI table (P5-T04 network tools).
#[tauri::command]
pub fn oui_lookup(mac: String) -> OuiLookup {
    OuiLookup {
        vendor: crate::netscan::oui::vendor(&mac),
        table_size: crate::netscan::oui::size(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub range: Range,
    pub count: u64,
    /// The connected interface; None = a routed network (through the gateway, no ARP).
    pub iface: Option<String>,
    /// Every address also gets TCP connects (otherwise only those that answer ping).
    pub full_tcp: bool,
}

/// Whether `target` may be scanned (and how many addresses), or why not.
#[tauri::command]
pub fn netscan_check(target: String) -> Result<CheckResult, ScopeError> {
    let (range, net) = scope::check(&target, &scope::local_networks())?;
    Ok(CheckResult {
        count: range.len(),
        full_tcp: range.len() <= scope::FULL_TCP_LIMIT,
        range,
        iface: net.map(|n| n.iface),
    })
}

/// Set while a scan runs; one scan at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: std::sync::LazyLock<Arc<AtomicBool>> =
    std::sync::LazyLock::new(|| Arc::new(AtomicBool::new(false)));

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ScanError {
    /// The target is outside the scope rules.
    Scope { reason: ScopeError },
    /// Another scan is running.
    Busy,
}

/// Scans `target` (checked against the scope rules here, whatever the UI did). Progress
/// goes to `on_progress`; the result is returned at the end and saved to the history.
#[tauri::command]
pub async fn netscan_run(
    state: State<'_, AppState>,
    target: String,
    options: Options,
    on_progress: Channel<Progress>,
) -> Result<ScanResult, ScanError> {
    let (range, net) = scope::check(&target, &scope::local_networks())
        .map_err(|reason| ScanError::Scope { reason })?;
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(ScanError::Busy);
    }
    CANCEL.store(false, Ordering::SeqCst);
    let gateway = tauri::async_runtime::spawn_blocking(crate::platform::net::default_gateway)
        .await
        .ok()
        .flatten();
    let plan = Plan {
        range,
        own: net.as_ref().map(|n: &LocalNet| n.ip),
        gateway,
        options,
    };
    let result = probe::run(plan, Arc::new(NetProber), Arc::clone(&CANCEL), |p| {
        let _ = on_progress.send(p);
    })
    .await;
    RUNNING.store(false, Ordering::SeqCst);
    // A history failure must not lose the result on screen.
    if let Err(e) = state.db.add_scan(&target, &result) {
        tracing::warn!(code = ?e.code, "could not save the scan to the history");
    }
    Ok(result)
}

/// The saved scans, newest first.
#[tauri::command]
pub fn netscan_history(state: State<'_, AppState>) -> crate::error::Result<Vec<ScanSummary>> {
    state.db.list_scans()
}

/// One saved scan's result.
#[tauri::command]
pub fn netscan_history_get(
    state: State<'_, AppState>,
    id: i64,
) -> crate::error::Result<Option<ScanResult>> {
    state.db.get_scan(id)
}

#[tauri::command]
pub fn netscan_history_delete(state: State<'_, AppState>, id: i64) -> crate::error::Result<()> {
    state.db.delete_scan(id)
}

/// Stops the running scan (results so far are returned).
#[tauri::command]
pub fn netscan_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Opens a scanned device's web interface (`http(s)://<ip>/`) in the system browser.
/// Only private / link-local IPv4 addresses: the URL is built here, not taken from the UI.
#[tauri::command]
pub fn open_device_web(
    app: tauri::AppHandle,
    ip: String,
    https: bool,
) -> Result<(), crate::error::AppError> {
    use tauri_plugin_opener::OpenerExt;
    let addr: std::net::Ipv4Addr = ip
        .trim()
        .parse()
        .map_err(|_| crate::error::AppError::invalid("not an IPv4 address"))?;
    if !scope::allowed(addr) {
        return Err(crate::error::AppError::invalid(
            "only local network addresses",
        ));
    }
    let url = format!("{}://{addr}/", if https { "https" } else { "http" });
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| crate::error::AppError::internal(e.to_string()))
}
