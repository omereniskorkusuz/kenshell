//! What an SNMP agent knows that the topology map needs: the same [`Discovery`] a CLI
//! session yields (neighbours from LLDP-MIB, learned addresses from the bridge MIBs, ARP
//! from the IP MIB), plus the device's own name and interface names.

use std::collections::HashMap;

use serde::Serialize;

use super::Client;
use super::ber::{Oid, Value};
use crate::analyzer::neighbors::Neighbor;
use crate::analyzer::topology::{ArpEntry, Discovery, MAX_ARP, MAX_MACS, MacEntry};
use crate::error::Result;
use crate::netscan::oui;

const SYS_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
const SYS_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 5, 0];
const IF_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1];
const IF_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
const LLDP_LOC_CHASSIS: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 2, 0];
const LLDP_LOC_PORT_ID: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 3, 7, 1, 3];
const LLDP_REM: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 1, 1];
const LLDP_REM_ADDR: &[u32] = &[1, 0, 8802, 1, 1, 2, 1, 4, 2, 1, 3];
const BASE_PORT_IF: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 1, 4, 1, 2];
const FDB_PORT: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 4, 3, 1, 2];
const QFDB_PORT: &[u32] = &[1, 3, 6, 1, 2, 1, 17, 7, 1, 2, 2, 1, 2];
const ARP_PHYS: &[u32] = &[1, 3, 6, 1, 2, 1, 4, 22, 1, 2];
/// ipRouteNextHop of the default route (0.0.0.0).
const DEFAULT_NEXT_HOP: &[u32] = &[1, 3, 6, 1, 2, 1, 4, 21, 1, 7, 0, 0, 0, 0];

/// One polled device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnmpDevice {
    pub name: Option<String>,
    pub description: Option<String>,
    pub discovery: Discovery,
    /// ifIndex → interface name (flow records name ports by ifIndex).
    pub interfaces: Vec<(u32, String)>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A MAC as the map shows chassis ids: `aabb.ccdd.eeff`.
fn dotted(bytes: &[u8]) -> String {
    let h = hex(bytes);
    format!("{}.{}.{}", &h[0..4], &h[4..8], &h[8..12])
}

/// A port / chassis id: text when printable, a MAC when six raw bytes.
fn id_text(v: &Value) -> Option<String> {
    v.text().or_else(|| {
        let b = v.bytes()?;
        (b.len() == 6).then(|| dotted(b))
    })
}

/// LLDP capability bits (first octet, most significant bit first).
fn capabilities(v: &Value) -> Option<String> {
    let bits = *v.bytes()?.first()?;
    let names = [
        (0x40, "Repeater"),
        (0x20, "Bridge"),
        (0x10, "WLAN Access Point"),
        (0x08, "Router"),
        (0x04, "Telephone"),
        (0x01, "Station"),
    ];
    let on: Vec<&str> = names
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, n)| *n)
        .collect();
    (!on.is_empty()).then(|| on.join(", "))
}

/// The index part of an OID under `root`.
fn suffix<'a>(oid: &'a Oid, root: &[u32]) -> &'a [u32] {
    oid.get(root.len()..).unwrap_or(&[])
}

/// Reads the device. Tables the agent does not have come back empty; only "no answer at
/// all" (unreachable, wrong community, SNMP off) is an error.
pub fn discover(client: &mut Client) -> Result<SnmpDevice> {
    let sys = client.get(&[
        SYS_NAME.to_vec(),
        SYS_DESCR.to_vec(),
        LLDP_LOC_CHASSIS.to_vec(),
        DEFAULT_NEXT_HOP.to_vec(),
    ])?;
    let gateway = match sys.get(3).map(|(_, v)| v) {
        Some(Value::Ip([a, b, c, d])) if [*a, *b, *c, *d] != [0, 0, 0, 0] => {
            Some(format!("{a}.{b}.{c}.{d}"))
        }
        _ => None,
    };
    let mut sources = vec!["SNMP system".to_owned()];
    if gateway.is_some() {
        sources.push("SNMP ipRouteTable".to_owned());
    }
    let name = sys.first().and_then(|(_, v)| v.text());
    let description = sys.get(1).and_then(|(_, v)| v.text());
    let own_mac = sys
        .get(2)
        .and_then(|(_, v)| v.bytes())
        .filter(|b| b.len() == 6)
        .map(hex);
    // After the first answer the device is known to talk: a table it lacks is just empty.
    let mut answered: Vec<&'static str> = Vec::new();
    let mut table = |root: &[u32], name: &'static str| {
        let rows = client.walk(root).unwrap_or_default();
        if !rows.is_empty() && !answered.contains(&name) {
            answered.push(name);
        }
        rows
    };

    let mut if_names: HashMap<u32, String> = HashMap::new();
    for root in [IF_DESCR, IF_NAME] {
        for (oid, v) in table(root, "SNMP IF-MIB") {
            if let (Some(&index), Some(text)) = (suffix(&oid, root).first(), v.text()) {
                if_names.insert(index, text);
            }
        }
    }
    let loc_ports: HashMap<u32, String> = table(LLDP_LOC_PORT_ID, "SNMP LLDP-MIB")
        .into_iter()
        .filter_map(|(oid, v)| Some((*suffix(&oid, LLDP_LOC_PORT_ID).first()?, v.text()?)))
        .collect();

    // lldpRemTable: column, then index timeMark.localPortNum.remIndex.
    #[derive(Default)]
    struct Rem {
        chassis: Option<Value>,
        port_id: Option<Value>,
        port_desc: Option<String>,
        name: Option<String>,
        descr: Option<String>,
        caps: Option<String>,
        ip: Option<String>,
    }
    let mut rems: std::collections::BTreeMap<(u32, u32), Rem> = std::collections::BTreeMap::new();
    for (oid, v) in table(LLDP_REM, "SNMP LLDP-MIB") {
        let &[column, _time, port, index] = suffix(&oid, LLDP_REM) else {
            continue;
        };
        let r = rems.entry((port, index)).or_default();
        match column {
            5 => r.chassis = Some(v),
            7 => r.port_id = Some(v),
            8 => r.port_desc = v.text(),
            9 => r.name = v.text(),
            10 => r.descr = v.text(),
            12 => r.caps = capabilities(&v),
            _ => {}
        }
    }
    for (oid, _) in table(LLDP_REM_ADDR, "SNMP LLDP-MIB") {
        // timeMark.localPortNum.remIndex.addrSubtype.addrLen.address…
        let s = suffix(&oid, LLDP_REM_ADDR);
        if let [_, port, index, 1, 4, a, b, c, d] = *s
            && let Some(r) = rems.get_mut(&(port, index))
        {
            r.ip.get_or_insert(format!("{a}.{b}.{c}.{d}"));
        }
    }
    let neighbors = rems
        .into_iter()
        .map(|((port, _), r)| {
            let chassis = r
                .chassis
                .as_ref()
                .and_then(Value::bytes)
                .filter(|b| b.len() == 6)
                .map(dotted);
            Neighbor {
                name: r
                    .name
                    .or_else(|| r.chassis.as_ref().and_then(id_text))
                    .unwrap_or_default(),
                ip: r.ip,
                platform: r.descr.map(|d| d.chars().take(80).collect()),
                capability: r.caps,
                local_interface: loc_ports
                    .get(&port)
                    .or_else(|| if_names.get(&port))
                    .cloned()
                    .unwrap_or_else(|| format!("port {port}")),
                remote_interface: r.port_id.as_ref().and_then(id_text).or(r.port_desc),
                cdp: false,
                lldp: true,
                vendor: chassis.as_deref().and_then(oui::vendor).map(str::to_owned),
                chassis,
            }
        })
        .collect();

    // Forwarding table: MAC → bridge port → ifIndex → name. The VLAN-aware table first.
    let bridge_if: HashMap<u32, u32> = table(BASE_PORT_IF, "SNMP BRIDGE-MIB")
        .into_iter()
        .filter_map(|(oid, v)| {
            Some((
                *suffix(&oid, BASE_PORT_IF).first()?,
                u32::try_from(v.int()?).ok()?,
            ))
        })
        .collect();
    let mut macs: Vec<MacEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (root, vlan_first) in [(QFDB_PORT, true), (FDB_PORT, false)] {
        for (oid, v) in table(root, "SNMP BRIDGE-MIB") {
            let s = suffix(&oid, root);
            let (vlan, mac) = match (vlan_first, s.len()) {
                (true, 7) => (Some(s[0].to_string()), &s[1..]),
                (false, 6) => (None, s),
                _ => continue,
            };
            let Some(port) = v
                .int()
                .and_then(|p| u32::try_from(p).ok())
                .filter(|p| *p != 0)
            else {
                continue;
            };
            let Ok(bytes) = mac
                .iter()
                .map(|b| u8::try_from(*b))
                .collect::<std::result::Result<Vec<u8>, _>>()
            else {
                continue;
            };
            let key = hex(&bytes);
            let if_index = bridge_if.get(&port).copied().unwrap_or(port);
            let interface = if_names
                .get(&if_index)
                .cloned()
                .unwrap_or_else(|| format!("port {port}"));
            if !seen.insert((key.clone(), interface.clone())) || macs.len() >= MAX_MACS {
                continue;
            }
            macs.push(MacEntry {
                vendor: oui::vendor(&key).map(str::to_owned),
                mac: key,
                vlan,
                interface,
            });
        }
        if !macs.is_empty() {
            break;
        }
    }

    let arp: Vec<ArpEntry> = table(ARP_PHYS, "SNMP IP-MIB")
        .into_iter()
        .filter_map(|(oid, v)| {
            let &[_if, a, b, c, d] = suffix(&oid, ARP_PHYS) else {
                return None;
            };
            let mac = v.bytes().filter(|m| m.len() == 6)?;
            Some(ArpEntry {
                ip: format!("{a}.{b}.{c}.{d}"),
                mac: hex(mac),
            })
        })
        .take(MAX_ARP)
        .collect();

    let mut interfaces: Vec<(u32, String)> = if_names.into_iter().collect();
    interfaces.sort();
    Ok(SnmpDevice {
        name,
        description,
        discovery: Discovery {
            neighbors,
            macs,
            arp,
            own_vendor: own_mac.as_deref().and_then(oui::vendor).map(str::to_owned),
            own_mac,
            gateway,
            sources: {
                sources.extend(answered.iter().map(|s| (*s).to_owned()));
                sources
            },
        },
        interfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::super::agent::{SampleNeighbor, sample_mib};
    use super::super::testing::Agent;
    use super::*;

    #[test]
    fn a_switch_is_read_like_a_session_would_read_it() {
        let mib = sample_mib(
            "dist-sw-01",
            "L3 switch software, version 17.9",
            "0011.2233.9500",
            &[(1, "Te1/0/1"), (2, "Te1/0/2"), (10, "Gi1/0/10")],
            &[
                SampleNeighbor {
                    local_port: 1,
                    chassis: "0011.2233.4400",
                    port_id: "Te1/1/1",
                    name: "core-sw-01",
                    description: "Core switch",
                    caps: 0x28,
                    ip: [10, 0, 0, 1],
                },
                SampleNeighbor {
                    local_port: 2,
                    chassis: "5869.6c00.3333",
                    port_id: "eth0",
                    name: "ap-roof",
                    description: "Outdoor AP",
                    caps: 0x10,
                    ip: [10, 20, 0, 40],
                },
            ],
            &[("3c52.8200.0d05", 10), ("0011.2233.4400", 1)],
            &[
                ([10, 0, 0, 2], "0011.2233.9500"),
                ([10, 10, 0, 77], "3c52.8200.0d05"),
            ],
        );
        let agent = Agent::start(mib, "public");
        let mut c = Client::connect(agent.addr, "public").unwrap();
        let d = discover(&mut c).unwrap();
        assert_eq!(d.name.as_deref(), Some("dist-sw-01"));
        assert_eq!(d.discovery.gateway.as_deref(), Some("10.0.0.2"));
        assert!(d.discovery.sources.iter().any(|s| s.contains("LLDP")));
        assert_eq!(d.discovery.own_mac.as_deref(), Some("001122339500"));
        assert_eq!(d.interfaces.len(), 3);
        let n = &d.discovery.neighbors;
        assert_eq!(n.len(), 2, "{n:?}");
        assert_eq!(n[0].name, "core-sw-01");
        assert_eq!(n[0].local_interface, "Te1/0/1");
        assert_eq!(n[0].remote_interface.as_deref(), Some("Te1/1/1"));
        assert_eq!(n[0].ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(n[0].chassis.as_deref(), Some("0011.2233.4400"));
        assert_eq!(n[0].capability.as_deref(), Some("Bridge, Router"));
        assert_eq!(n[1].capability.as_deref(), Some("WLAN Access Point"));
        assert_eq!(d.discovery.macs.len(), 2);
        let pc = d
            .discovery
            .macs
            .iter()
            .find(|m| m.mac == "3c5282000d05")
            .unwrap();
        assert_eq!(pc.interface, "Gi1/0/10");
        assert!(
            d.discovery
                .arp
                .iter()
                .any(|a| a.ip == "10.10.0.77" && a.mac == "3c5282000d05")
        );
    }

    #[test]
    fn an_agent_with_only_the_system_group_gives_an_empty_device() {
        let mut mib = super::super::agent::Mib::new();
        mib.insert(SYS_NAME.to_vec(), Value::Bytes(b"edge-rtr".to_vec()));
        let agent = Agent::start(mib, "public");
        let mut c = Client::connect(agent.addr, "public").unwrap();
        let d = discover(&mut c).unwrap();
        assert_eq!(d.name.as_deref(), Some("edge-rtr"));
        assert!(d.discovery.neighbors.is_empty() && d.discovery.macs.is_empty());
        assert!(d.interfaces.is_empty());
    }
}
