//! Extra sources for the topology map: SNMP polling and the flow collector. The SNMP
//! community is a secret: stored in the OS keychain, never returned to the frontend,
//! never in the DB, logs or exports.

use tauri::State;

use crate::error::{AppError, ErrorCode, Result};
use crate::flow::FlowSnapshot;
use crate::snmp::discover::SnmpDevice;
use crate::state::AppState;

const ACCOUNT: &str = "snmp-community";

fn valid(community: &str) -> bool {
    (1..=255).contains(&community.len()) && community.chars().all(|c| !c.is_control())
}

/// True when a community is stored.
#[tauri::command]
pub fn snmp_community_status(state: State<'_, AppState>) -> Result<bool> {
    Ok(state.secrets.get(ACCOUNT)?.is_some())
}

#[tauri::command]
pub fn snmp_community_set(state: State<'_, AppState>, community: String) -> Result<()> {
    if !valid(&community) {
        return Err(AppError::invalid("community"));
    }
    state.secrets.set(ACCOUNT, &community)
}

#[tauri::command]
pub fn snmp_community_clear(state: State<'_, AppState>) -> Result<()> {
    state.secrets.delete(ACCOUNT)
}

/// Reads one device over SNMPv2c (`host` or `host:port`) with the stored community.
#[tauri::command]
pub async fn topology_snmp(state: State<'_, AppState>, host: String) -> Result<SnmpDevice> {
    let community = state
        .secrets
        .get(ACCOUNT)?
        .ok_or_else(|| AppError::with(ErrorCode::NotFound, "no SNMP community stored"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let addr = crate::snmp::resolve(&host)?;
        let mut client = crate::snmp::Client::connect(addr, &community)?;
        crate::snmp::discover::discover(&mut client)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// Starts the sFlow / NetFlow listeners (0 = that protocol off).
#[tauri::command]
pub async fn flow_start(
    state: State<'_, AppState>,
    sflow_port: u16,
    netflow_port: u16,
) -> Result<FlowSnapshot> {
    let flow = std::sync::Arc::clone(&state.flow);
    tauri::async_runtime::spawn_blocking(move || {
        flow.start(sflow_port, netflow_port)?;
        Ok(flow.snapshot())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
pub async fn flow_stop(state: State<'_, AppState>) -> Result<FlowSnapshot> {
    let flow = std::sync::Arc::clone(&state.flow);
    tauri::async_runtime::spawn_blocking(move || {
        flow.stop();
        flow.clear();
        flow.snapshot()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))
}

#[tauri::command]
pub fn flow_snapshot(state: State<'_, AppState>) -> FlowSnapshot {
    state.flow.snapshot()
}

#[cfg(test)]
mod tests {
    #[test]
    fn community_shape() {
        assert!(super::valid("public"));
        assert!(super::valid("Rö-1 with space"));
        assert!(!super::valid(""));
        assert!(!super::valid("line\nbreak"));
        assert!(!super::valid(&"x".repeat(256)));
    }
}
