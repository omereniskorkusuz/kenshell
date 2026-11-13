//! Extra sources for the topology map: SNMP polling and the flow collector. The SNMP
//! community is a secret: stored in the OS keychain, never returned to the frontend,
//! never in the DB, logs or exports.

use tauri::State;

use crate::error::{AppError, ErrorCode, Result};
use crate::flow::FlowSnapshot;
use crate::snmp::discover::SnmpDevice;
use crate::state::AppState;

const ACCOUNT: &str = "snmp-community";
/// The SNMPv3 user (name, protocols and both passwords) as one keychain entry.
const V3_ACCOUNT: &str = "snmp-v3";

/// What the map page may know about the stored SNMP credentials: never a password.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnmpCredentials {
    pub community: bool,
    pub v3: Option<V3Summary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3Summary {
    pub user: String,
    pub auth: Option<crate::snmp::v3::AuthProto>,
    pub privacy: Option<crate::snmp::v3::PrivProto>,
}

fn v3_user(state: &AppState) -> Result<Option<crate::snmp::v3::V3User>> {
    let Some(raw) = state.secrets.get(V3_ACCOUNT)? else {
        return Ok(None);
    };
    // A damaged entry is treated as none (and says nothing about its content).
    Ok(serde_json::from_str(&raw).ok())
}

#[tauri::command]
pub fn snmp_credentials(state: State<'_, AppState>) -> Result<SnmpCredentials> {
    Ok(SnmpCredentials {
        community: state.secrets.get(ACCOUNT)?.is_some(),
        v3: v3_user(&state)?.map(|u| V3Summary {
            user: u.user,
            auth: u.auth,
            privacy: u.privacy,
        }),
    })
}

/// Stores the v3 user. Empty passwords keep the stored ones (the page never shows them).
#[tauri::command]
pub fn snmp_v3_set(state: State<'_, AppState>, user: crate::snmp::v3::V3User) -> Result<()> {
    let mut user = user;
    let name_ok = (1..=64).contains(&user.user.len()) && user.user.chars().all(|c| !c.is_control());
    if !name_ok || (user.privacy.is_some() && user.auth.is_none()) {
        return Err(AppError::invalid("SNMPv3 user"));
    }
    if let Some(old) = v3_user(&state)? {
        if user.auth_pass.is_empty() {
            user.auth_pass = old.auth_pass;
        }
        if user.priv_pass.is_empty() {
            user.priv_pass = old.priv_pass;
        }
    }
    // RFC 3414: passwords of at least 8 characters.
    let short = |p: &str| p.chars().count() < 8;
    if (user.auth.is_some() && short(&user.auth_pass))
        || (user.privacy.is_some() && short(&user.priv_pass))
    {
        return Err(AppError::invalid(
            "SNMPv3 password shorter than 8 characters",
        ));
    }
    let json = zeroize::Zeroizing::new(
        serde_json::to_string(&user).map_err(|e| AppError::internal(e.to_string()))?,
    );
    state.secrets.set(V3_ACCOUNT, &json)
}

#[tauri::command]
pub fn snmp_v3_clear(state: State<'_, AppState>) -> Result<()> {
    state.secrets.delete(V3_ACCOUNT)
}

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

/// Reads one device (`host` or `host:port`) with the stored credentials: SNMPv3 first when
/// a user is stored, then the v2c community.
#[tauri::command]
pub async fn topology_snmp(state: State<'_, AppState>, host: String) -> Result<SnmpDevice> {
    let community = state.secrets.get(ACCOUNT)?;
    let v3 = v3_user(&state)?;
    if community.is_none() && v3.is_none() {
        return Err(AppError::with(
            ErrorCode::NotFound,
            "no SNMP credentials stored",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let addr = crate::snmp::resolve(&host)?;
        let mut last = None;
        if let Some(user) = v3 {
            let mut client = crate::snmp::Client::connect_v3(addr, user)?;
            match crate::snmp::discover::discover(&mut client) {
                Ok(d) => return Ok(d),
                Err(e) => last = Some(e),
            }
        }
        if let Some(c) = community {
            let mut client = crate::snmp::Client::connect(addr, &c)?;
            return crate::snmp::discover::discover(&mut client);
        }
        Err(last.unwrap_or_else(|| AppError::internal("no attempt")))
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
