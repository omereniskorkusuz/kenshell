//! Network scanner commands (P4-T12…T14). The scope rules live in `netscan::scope` and are
//! applied to every request here (the UI only mirrors them for display).

use serde::Serialize;

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
