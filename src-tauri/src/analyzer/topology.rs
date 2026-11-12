//! Data for the topology map: what one device knows about what hangs off its ports.
//! Besides CDP / LLDP neighbours ([`super::neighbors`]) that is the MAC address table
//! (which MAC was learned on which port) and the ARP table (MAC → IP, and the device's own
//! MAC). The map itself is merged and laid out in the frontend from every open session.

use serde::Serialize;

use super::neighbors::Neighbor;
use super::parsers::{ALL, mac_address_table::is_mac};
use crate::netscan::oui;

/// A MAC learned on a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacEntry {
    /// 12 lowercase hex digits.
    pub mac: String,
    pub vlan: Option<String>,
    pub interface: String,
    /// Manufacturer from the IEEE OUI table.
    pub vendor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArpEntry {
    pub ip: String,
    /// 12 lowercase hex digits.
    pub mac: String,
}

/// Everything one session contributes to the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub neighbors: Vec<Neighbor>,
    pub macs: Vec<MacEntry>,
    pub arp: Vec<ArpEntry>,
    /// The device's own MAC (from its own ARP entries), if it could be told.
    pub own_mac: Option<String>,
    pub own_vendor: Option<String>,
}

/// Upper bounds: a core switch can know tens of thousands of MACs; the map never needs them.
pub const MAX_MACS: usize = 4000;
pub const MAX_ARP: usize = 4000;

/// Read-only commands for the MAC and ARP tables of a vendor family.
pub fn host_commands(family: Option<&str>) -> &'static [&'static str] {
    match family {
        Some("ruijie") => &["show mac-address-table", "show arp"],
        Some("cisco") | None | Some("generic") => &["show mac address-table", "show ip arp"],
        // Other families: their tables are not parsed yet.
        _ => &[],
    }
}

/// `aabb.ccdd.eeff` / `aa:bb:…` / `aa-bb-…` → 12 lowercase hex digits.
pub fn mac_key(mac: &str) -> String {
    mac.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn is_ipv4(s: &str) -> bool {
    let mut n = 0;
    for part in s.split('.') {
        if part.is_empty() || part.len() > 3 || part.parse::<u8>().is_err() {
            return false;
        }
        n += 1;
    }
    n == 4
}

/// Ports that are the switch itself, not a cable.
fn internal_port(port: &str) -> bool {
    let p = port.to_ascii_lowercase();
    [
        "cpu", "sup", "router", "switch", "drop", "vlan", "self", "igmp",
    ]
    .iter()
    .any(|k| p.starts_with(k))
}

/// MAC table rows: learned (dynamic) addresses on real ports, deduplicated per port.
pub fn mac_entries(family: Option<&str>, text: &str) -> Vec<MacEntry> {
    let id = if family == Some("ruijie") {
        "ruijie.show_mac_address_table"
    } else {
        "cisco.show_mac_address_table"
    };
    let rows = ALL
        .iter()
        .find(|p| p.id == id)
        .map(|p| (p.parse)(text))
        .unwrap_or_default();
    let mut out: Vec<MacEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in rows {
        // vlan, mac, type, port
        let (Some(vlan), Some(mac), Some(kind), Some(port)) =
            (r.first(), r.get(1), r.get(2), r.get(3))
        else {
            continue;
        };
        if !kind.to_ascii_lowercase().starts_with("dyn") || internal_port(port) {
            continue;
        }
        let key = mac_key(mac);
        if key.len() != 12 || !seen.insert((key.clone(), port.to_lowercase())) {
            continue;
        }
        out.push(MacEntry {
            vendor: oui::vendor(&key).map(str::to_owned),
            mac: key,
            vlan: Some(vlan.clone()).filter(|v| !v.is_empty()),
            interface: port.clone(),
        });
        if out.len() >= MAX_MACS {
            break;
        }
    }
    out
}

/// ARP rows (`IP … MAC` on one line, any vendor layout) and the device's own MAC: the
/// entry whose age is `-` / `--` (the device's own address on Cisco-like CLIs).
pub fn arp_entries(text: &str) -> (Vec<ArpEntry>, Option<String>) {
    let mut out = Vec::new();
    let mut own = None;
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let (Some(ip), Some(m)) = (
            w.iter().find(|t| is_ipv4(t)),
            w.iter().position(|t| is_mac(t)),
        ) else {
            continue;
        };
        let mac = mac_key(w[m]);
        if mac.len() != 12 || mac == "ffffffffffff" {
            continue;
        }
        if own.is_none() && m > 0 && matches!(w[m - 1], "-" | "--") {
            own = Some(mac.clone());
        }
        if seen.insert(((*ip).to_owned(), mac.clone())) && out.len() < MAX_ARP {
            out.push(ArpEntry {
                ip: (*ip).to_owned(),
                mac,
            });
        }
    }
    (out, own)
}

/// Builds one session's contribution from its neighbours and `(command, output)` pairs.
pub fn discovery(
    family: Option<&str>,
    neighbors: Vec<Neighbor>,
    outputs: &[(String, String)],
) -> Discovery {
    let mut d = Discovery {
        neighbors,
        ..Discovery::default()
    };
    for (command, text) in outputs {
        if command.contains("mac") {
            d.macs = mac_entries(family, text);
        } else if command.contains("arp") {
            let (arp, own) = arp_entries(text);
            d.arp = arp;
            d.own_mac = own;
        }
    }
    d.own_vendor = d
        .own_mac
        .as_deref()
        .and_then(oui::vendor)
        .map(str::to_owned);
    d
}

/// Reads one live session (hidden, read-only): its neighbours and, with `hosts`, its MAC
/// and ARP tables. Commands the device does not know are skipped; a closed session is an
/// error.
pub async fn discover(
    sessions: &crate::session::SessionManager,
    id: crate::model::RuntimeId,
    family: Option<&str>,
    hosts: bool,
) -> crate::error::Result<Discovery> {
    let mut near = Vec::new();
    let mut tables = Vec::new();
    let wanted = super::neighbors::commands(family)
        .iter()
        .map(|c| (c, true))
        .chain(
            host_commands(family)
                .iter()
                .filter(|_| hosts)
                .map(|c| (c, false)),
        );
    for (command, neighbour) in wanted {
        match sessions
            .capture(id, command, std::time::Duration::from_secs(60))
            .await
        {
            Ok(text) if neighbour => near.push(((*command).to_owned(), text)),
            Ok(text) => tables.push(((*command).to_owned(), text)),
            Err(e) if e.code == crate::error::ErrorCode::ConnectionClosed => return Err(e),
            Err(_) => {}
        }
    }
    Ok(discovery(
        family,
        super::neighbors::neighbors(family, &near),
        &tables,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC_TABLE: &str = "\
          Mac Address Table
-------------------------------------------
Vlan    Mac Address       Type        Ports
----    -----------       --------    -----
 All    0100.0ccc.cccc    STATIC      CPU
  10    0011.2233.4455    DYNAMIC     Gi1/0/1
  10    0011.2233.4455    DYNAMIC     Gi1/0/1
  20    aabb.ccdd.eeff    DYNAMIC     Gi1/0/2
  20    aabb.ccdd.0001    STATIC      Gi1/0/3
Total Mac Addresses for this criterion: 4
";

    const ARP: &str = "\
Protocol  Address          Age (min)  Hardware Addr   Type   Interface
Internet  10.0.0.1                -   0011.2233.4400  ARPA   Vlan1
Internet  10.0.0.20              12   aabb.ccdd.eeff  ARPA   Vlan20
Internet  10.0.0.99               0   Incomplete      ARPA
";

    #[test]
    fn mac_table_keeps_learned_addresses_on_real_ports() {
        let m = mac_entries(Some("cisco"), MAC_TABLE);
        assert_eq!(m.len(), 2, "{m:?}");
        assert_eq!(m[0].mac, "001122334455");
        assert_eq!(m[0].interface, "Gi1/0/1");
        assert_eq!(m[0].vlan.as_deref(), Some("10"));
        assert_eq!(m[1].mac, "aabbccddeeff");
    }

    #[test]
    fn arp_gives_addresses_and_the_own_mac() {
        let (arp, own) = arp_entries(ARP);
        assert_eq!(arp.len(), 2);
        assert_eq!(arp[1].ip, "10.0.0.20");
        assert_eq!(arp[1].mac, "aabbccddeeff");
        assert_eq!(own.as_deref(), Some("001122334400"));
    }

    #[test]
    fn ruijie_layout_and_garbage() {
        let (arp, own) = arp_entries(
            "Protocol Address     Age(min) Hardware       Type  Interface\n\
             Internet 192.168.1.1 --       5869.6c00.1111 arpa  VLAN 1\n\
             Internet 192.168.1.7 3        58:69:6c:00:22:22 arpa VLAN 1\n",
        );
        assert_eq!(arp.len(), 2);
        assert_eq!(own.as_deref(), Some("58696c001111"));
        assert_eq!(arp[1].mac, "58696c002222");
        assert!(
            arp_entries("% Invalid input\n\0\u{1b}[0m 999.1.1.1 zz")
                .0
                .is_empty()
        );
        assert!(mac_entries(None, "garbage\n\n---").is_empty());
        assert!(host_commands(Some("juniper")).is_empty());
    }

    #[test]
    fn discovery_joins_the_parts() {
        let d = discovery(
            Some("cisco"),
            Vec::new(),
            &[
                ("show mac address-table".into(), MAC_TABLE.into()),
                ("show ip arp".into(), ARP.into()),
            ],
        );
        assert_eq!(d.macs.len(), 2);
        assert_eq!(d.arp.len(), 2);
        assert_eq!(d.own_mac.as_deref(), Some("001122334400"));
    }
}
