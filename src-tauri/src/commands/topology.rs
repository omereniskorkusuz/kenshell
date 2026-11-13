//! Extra sources for the topology map: SNMP polling and the flow collector. The SNMP
//! community is a secret: stored in the OS keychain, never returned to the frontend,
//! never in the DB, logs or exports.

use tauri::State;

use crate::error::{AppError, ErrorCode, Result};
use crate::flow::FlowSnapshot;
use crate::secrets::SecretStore;
use crate::snmp::discover::SnmpDevice;
use crate::state::AppState;

/// Older builds stored a single community here; read once, then moved to [`LIST_ACCOUNT`].
const ACCOUNT: &str = "snmp-community";
/// The v2c communities (JSON array, tried together, at most [`MAX_COMMUNITIES`]).
const LIST_ACCOUNT: &str = "snmp-communities";
const MAX_COMMUNITIES: usize = 8;
/// The SNMPv3 user (name, protocols and both passwords) as one keychain entry.
const V3_ACCOUNT: &str = "snmp-v3";

/// What the map page may know about the stored SNMP credentials: never a password.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnmpCredentials {
    pub community: bool,
    /// How many communities are stored (never which).
    pub communities: usize,
    pub v3: Option<V3Summary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3Summary {
    pub user: String,
    pub auth: Option<crate::snmp::v3::AuthProto>,
    pub privacy: Option<crate::snmp::v3::PrivProto>,
}

type Secret = zeroize::Zeroizing<String>;

/// The stored communities, the legacy single entry included.
fn communities(secrets: &dyn SecretStore) -> Result<Vec<Secret>> {
    let mut out: Vec<Secret> = Vec::new();
    if let Some(raw) = secrets.get(LIST_ACCOUNT)? {
        // A damaged entry counts as empty (and says nothing about its content).
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
            out.extend(list.into_iter().map(Secret::new));
        }
    }
    if let Some(old) = secrets.get(ACCOUNT)?
        && !out.iter().any(|c| **c == *old)
    {
        out.insert(0, old);
    }
    Ok(out)
}

fn store_communities(secrets: &dyn SecretStore, list: &[Secret]) -> Result<()> {
    let plain: Vec<&str> = list.iter().map(|c| c.as_str()).collect();
    let json =
        Secret::new(serde_json::to_string(&plain).map_err(|e| AppError::internal(e.to_string()))?);
    secrets.set(LIST_ACCOUNT, &json)?;
    secrets.delete(ACCOUNT)
}

/// Adds `community` unless it is stored already.
fn add_community(secrets: &dyn SecretStore, community: Secret) -> Result<()> {
    if !valid(&community) {
        return Err(AppError::invalid("community"));
    }
    let mut list = communities(secrets)?;
    if !list.iter().any(|c| **c == *community) {
        if list.len() >= MAX_COMMUNITIES {
            return Err(AppError::invalid("more than 8 communities"));
        }
        list.push(community);
    }
    store_communities(secrets, &list)
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
    let count = communities(state.secrets.as_ref())?.len();
    Ok(SnmpCredentials {
        community: count > 0,
        communities: count,
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
    Ok(!communities(state.secrets.as_ref())?.is_empty())
}

/// Adds a community to the stored ones (a known one is not added twice). Devices answer
/// the one they know, so different devices may use different communities.
#[tauri::command]
pub fn snmp_community_set(state: State<'_, AppState>, community: String) -> Result<()> {
    add_community(state.secrets.as_ref(), Secret::new(community))
}

/// Forgets every stored community.
#[tauri::command]
pub fn snmp_community_clear(state: State<'_, AppState>) -> Result<()> {
    state.secrets.delete(LIST_ACCOUNT)?;
    state.secrets.delete(ACCOUNT)
}

/// Reads one device (`host` or `host:port`) with the stored credentials: SNMPv3 first when
/// a user is stored, then the v2c communities (all at once; the device answers its own).
#[tauri::command]
pub async fn topology_snmp(state: State<'_, AppState>, host: String) -> Result<SnmpDevice> {
    let list = communities(state.secrets.as_ref())?;
    let v3 = v3_user(&state)?;
    if list.is_empty() && v3.is_none() {
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
        if !list.is_empty() {
            let mut client = crate::snmp::Client::connect_any(addr, &list)?;
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

    #[test]
    fn several_communities_and_the_old_single_one() {
        use super::*;
        let store = crate::secrets::MemoryStore::default();
        let s: &dyn SecretStore = &store;
        // An older build's single community is kept and moved into the list.
        s.set(ACCOUNT, "OldOne").unwrap();
        add_community(s, Secret::new("Second".into())).unwrap();
        add_community(s, Secret::new("Second".into())).unwrap();
        let got: Vec<String> = communities(s)
            .unwrap()
            .iter()
            .map(|c| c.to_string())
            .collect();
        assert_eq!(got, ["OldOne", "Second"]);
        assert!(s.get(ACCOUNT).unwrap().is_none());
        for i in 0..6 {
            add_community(s, Secret::new(format!("c{i}"))).unwrap();
        }
        let e = add_community(s, Secret::new("ninth".into())).unwrap_err();
        assert!(!format!("{e:?}").contains("ninth"));
        assert!(add_community(s, Secret::new(String::new())).is_err());
        assert_eq!(communities(s).unwrap().len(), 8);
    }
}
