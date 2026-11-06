//! Network scanner commands (P4-T12…T14). The scope rules live in `netscan::scope` and are
//! applied to every request here (the UI only mirrors them for display).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::ipc::Channel;

use crate::netscan::probe::{self, NetProber, Progress, ScanResult};

use crate::netscan::scope::{self, LocalNet, Range, ScopeError};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Networks {
    pub networks: Vec<NetworkView>,
    /// The first network's default target (its /24, or the subnet when smaller).
    pub default_target: Option<String>,
    pub max_addresses: u32,
    pub concurrency: usize,
    pub ports: Vec<u16>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkView {
    #[serde(flatten)]
    pub net: LocalNet,
    pub cidr: String,
    pub default_target: String,
}

/// This computer's scannable subnets (private / link-local, directly connected).
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
        concurrency: scope::CONCURRENCY,
        ports: scope::PORTS.to_vec(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub range: Range,
    pub count: u64,
    pub iface: String,
}

/// Whether `target` may be scanned (and how many addresses), or why not.
#[tauri::command]
pub fn netscan_check(target: String) -> Result<CheckResult, ScopeError> {
    let (range, net) = scope::check(&target, &scope::local_networks())?;
    Ok(CheckResult {
        count: range.len(),
        range,
        iface: net.iface,
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
/// goes to `on_progress`; the result is returned at the end.
#[tauri::command]
pub async fn netscan_run(
    target: String,
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
        .flatten()
        .filter(|g| net.contains(*g));
    let result = probe::run(
        range,
        net.ip,
        gateway,
        Arc::new(NetProber),
        Arc::clone(&CANCEL),
        |p| {
            let _ = on_progress.send(p);
        },
    )
    .await;
    RUNNING.store(false, Ordering::SeqCst);
    Ok(result)
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
