//! CDP / LLDP neighbours for the topology view (P4-T10): the detail outputs parsed with the
//! analyzer parsers and merged (a neighbour seen by both protocols on the same local port
//! is one entry).

use serde::Serialize;

use super::parsers::ALL;
use crate::device::iface;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Neighbor {
    pub name: String,
    /// Management address, when advertised.
    pub ip: Option<String>,
    pub platform: Option<String>,
    pub capability: Option<String>,
    pub local_interface: String,
    pub remote_interface: Option<String>,
    pub cdp: bool,
    pub lldp: bool,
    /// LLDP chassis id when it is a MAC address.
    pub chassis: Option<String>,
    /// Manufacturer of the chassis MAC, from the IEEE OUI table.
    pub vendor: Option<String>,
}

/// The read-only commands to run for a vendor family (Cisco-like devices speak CDP too).
pub fn commands(family: Option<&str>) -> &'static [&'static str] {
    match family {
        Some("cisco") | None | Some("generic") => {
            &["show cdp neighbors detail", "show lldp neighbors detail"]
        }
        _ => &["show lldp neighbors detail"],
    }
}

fn parser_rows(id: &str, text: &str) -> Vec<Vec<String>> {
    ALL.iter()
        .find(|p| p.id == id)
        .map(|p| (p.parse)(text))
        .unwrap_or_default()
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// The chassis id as a MAC (`aabb.ccdd.eeff`, `aa:bb:…`, `aa-bb-…`), if it is one.
fn chassis_mac(s: &str) -> Option<String> {
    let t = s.trim();
    super::parsers::mac_address_table::is_mac(t).then(|| t.to_lowercase())
}

/// Matching key: the local port (normalised) + the neighbour's name without its domain.
fn key(n: &Neighbor) -> (String, String) {
    let port = iface::parse(&n.local_interface)
        .map_or_else(|| n.local_interface.to_lowercase(), |i| i.key);
    let name = n.name.split('.').next().unwrap_or("").to_lowercase();
    (port, name)
}

/// Neighbours from `(command, output)` pairs, sorted by local port.
pub fn neighbors(family: Option<&str>, outputs: &[(String, String)]) -> Vec<Neighbor> {
    let mut list: Vec<Neighbor> = Vec::new();
    let add = |n: Neighbor, list: &mut Vec<Neighbor>| {
        if n.name.is_empty() && n.ip.is_none() {
            return;
        }
        let k = key(&n);
        if let Some(e) = list.iter_mut().find(|e| key(e) == k) {
            e.cdp |= n.cdp;
            e.lldp |= n.lldp;
            if e.ip.is_none() {
                e.ip = n.ip;
            }
            if e.platform.is_none() {
                e.platform = n.platform;
            }
            if e.capability.is_none() {
                e.capability = n.capability;
            }
            if e.remote_interface.is_none() {
                e.remote_interface = n.remote_interface;
            }
            if e.chassis.is_none() {
                e.chassis = n.chassis;
                e.vendor = n.vendor;
            }
        } else {
            list.push(n);
        }
    };
    let ruijie = family == Some("ruijie");
    for (command, text) in outputs {
        if command.contains("cdp") {
            // device, ip, platform, capability, local, remote, version
            for r in parser_rows("cisco.show_cdp_neighbors_detail", text) {
                add(
                    Neighbor {
                        name: r[0].clone(),
                        ip: opt(&r[1]),
                        platform: opt(&r[2]),
                        capability: opt(&r[3]),
                        local_interface: r[4].clone(),
                        remote_interface: opt(&r[5]),
                        cdp: true,
                        lldp: false,
                        chassis: None,
                        vendor: None,
                    },
                    &mut list,
                );
            }
        } else if command.contains("lldp") {
            let id = if ruijie {
                "ruijie.show_lldp_neighbors_detail"
            } else {
                "cisco.show_lldp_neighbors_detail"
            };
            // device, ip, local, remote, port description, chassis, capability, description
            for r in parser_rows(id, text) {
                add(
                    Neighbor {
                        name: if r[0].is_empty() {
                            r[5].clone()
                        } else {
                            r[0].clone()
                        },
                        ip: opt(&r[1]),
                        platform: opt(&r[7]).map(|d| d.chars().take(80).collect()),
                        capability: opt(&r[6]),
                        local_interface: r[2].clone(),
                        remote_interface: opt(&r[3]),
                        cdp: false,
                        lldp: true,
                        chassis: chassis_mac(&r[5]),
                        vendor: chassis_mac(&r[5])
                            .and_then(|m| crate::netscan::oui::vendor(&m))
                            .map(str::to_owned),
                    },
                    &mut list,
                );
            }
        }
    }
    list.sort_by_key(key);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_and_lldp_are_merged_per_port() {
        let outputs = vec![
            (
                "show cdp neighbors detail".to_owned(),
                include_str!("fixtures/cisco_show_cdp_neighbors_detail.txt").to_owned(),
            ),
            (
                "show lldp neighbors detail".to_owned(),
                // core-sw1 is seen on Gi1/0/24 by CDP; this LLDP entry is on the same port.
                "------------------------------------------------\nLocal Intf: Gi1/0/24\nChassis id: 0011.2233.4400\nPort id: Gi1/0/1\nSystem Name: core-sw1\n\nSystem Description: \nCisco IOS Software\n\nManagement Addresses:\n    IP: 10.0.0.1\n"
                    .to_owned(),
            ),
        ];
        let n = neighbors(Some("cisco"), &outputs);
        assert_eq!(n.len(), 2, "{n:?}");
        let core = n.iter().find(|x| x.name.starts_with("core-sw1")).unwrap();
        assert!(core.cdp && core.lldp);
        assert_eq!(core.ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(core.local_interface, "GigabitEthernet1/0/24");
        let ap = n.iter().find(|x| x.name == "ap-floor2").unwrap();
        assert!(ap.cdp && !ap.lldp);
        assert_eq!(ap.ip.as_deref(), Some("10.20.0.15"));
    }

    #[test]
    fn ruijie_lldp_and_family_commands() {
        let n = neighbors(
            Some("ruijie"),
            &[(
                "show lldp neighbors detail".into(),
                include_str!("fixtures/ruijie_show_lldp_neighbors_detail.txt").into(),
            )],
        );
        assert_eq!(n.len(), 2);
        assert!(n.iter().all(|x| x.lldp && !x.cdp));
        assert_eq!(commands(Some("ruijie")), ["show lldp neighbors detail"]);
        assert_eq!(commands(None).len(), 2);
        assert!(
            neighbors(
                None,
                &[("show cdp neighbors detail".into(), "% Invalid input".into())]
            )
            .is_empty()
        );
    }
}
