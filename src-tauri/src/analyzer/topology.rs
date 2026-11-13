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
    /// Next hop of the device's default route (its gateway), if it has one.
    pub gateway: Option<String>,
    /// Where the data came from: the commands (or SNMP tables) that answered.
    pub sources: Vec<String>,
}

/// Upper bounds: a core switch can know tens of thousands of MACs; the map never needs them.
pub const MAX_MACS: usize = 4000;
pub const MAX_ARP: usize = 4000;

/// Read-only commands that show the default route, tried in order until one names a
/// next hop (a layer-2 switch has only a default gateway).
pub fn gateway_commands(family: Option<&str>) -> &'static [&'static str] {
    match family {
        Some("ruijie") => &["show ip route 0.0.0.0", "show ip route"],
        Some("aruba") => &["show ip route 0.0.0.0/0", "show ip route"],
        Some("alcatel") => &["show ip routes"],
        Some("juniper") => &["show route 0.0.0.0/0 exact"],
        Some("cisco") | None | Some("generic") => &[
            "show ip route 0.0.0.0",
            "show ip route",
            "show ip default-gateway",
        ],
        _ => &[],
    }
}

/// The default route's next hop in any vendor's route listing: `Gateway of last resort is
/// X`, `ip default-gateway X`-style lines, or the line of `0.0.0.0/0` (`0.0.0.0 0.0.0.0`,
/// `default`) with the first other address on it or the next lines (`via X`).
pub fn default_gateway(text: &str) -> Option<String> {
    let ip_of = |t: &str| {
        let t = t.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        (is_ipv4(t) && t != "0.0.0.0" && !t.starts_with("255.")).then(|| t.to_owned())
    };
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines {
        let l = line.to_ascii_lowercase();
        if let Some(rest) = l.split("gateway of last resort is").nth(1)
            && let Some(ip) = rest.split_whitespace().find_map(ip_of)
        {
            return Some(ip);
        }
    }
    for (i, line) in lines.iter().enumerate() {
        let l = line.to_ascii_lowercase();
        let words: Vec<&str> = l.split_whitespace().collect();
        if l.contains("default-gateway") || l.contains("default gateway") {
            if let Some(ip) = words.iter().find_map(|w| ip_of(w)) {
                return Some(ip);
            }
            continue;
        }
        let default = words.iter().enumerate().any(|(k, w)| {
            let w = w.trim_end_matches(',');
            w == "0.0.0.0/0"
                || (w == "0.0.0.0"
                    && words.get(k + 1).is_some_and(|n| {
                        n.trim_end_matches(',') == "0.0.0.0" || n.starts_with('[')
                    }))
                || (w == "default" && k == 0)
                || (w == "s*" || w == "s*e" || w == "s*>")
        });
        if !default {
            continue;
        }
        // The next hop is on this line or on the following ones (Junos: `> to X via ge-…`).
        for next in lines.iter().skip(i).take(4) {
            let n = next.to_ascii_lowercase();
            let ws: Vec<&str> = n.split_whitespace().collect();
            let after_via = ws
                .iter()
                .position(|w| *w == "via" || *w == "to")
                .and_then(|p| ws.get(p + 1))
                .and_then(|w| ip_of(w));
            if let Some(ip) = after_via {
                return Some(ip);
            }
            if let Some(ip) = ws
                .iter()
                .filter(|w| !w.contains('/'))
                .find_map(|w| ip_of(w))
            {
                return Some(ip);
            }
        }
    }
    None
}

/// Read-only commands for the MAC and ARP tables of a vendor family.
pub fn host_commands(family: Option<&str>) -> &'static [&'static str] {
    match family {
        Some("ruijie") => &["show mac-address-table", "show arp"],
        Some("cisco") | None | Some("generic") => &["show mac address-table", "show ip arp"],
        Some("aruba") => &["show mac-address-table", "show arp"],
        Some("alcatel") => &["show mac-learning", "show arp"],
        Some("juniper") => &["show ethernet-switching table", "show arp no-resolve"],
        // Other families: their tables are not read yet.
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

/// MAC table rows of any layout that has the address, a type word and the port last on
/// one line (`<mac> <vlan> dynamic <port>`, `VLAN <vlan> <mac> dynamic bridging <port>`,
/// `<vlan-name> <mac> D - <port>`), as `[vlan, mac, type, port]`.
fn generic_mac_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some(m) = w.iter().position(|t| is_mac(t)) else {
            continue;
        };
        // The port is the last word and not the address itself.
        if w.len() < 3 || m == w.len() - 1 {
            continue;
        }
        let learned = w.iter().enumerate().any(|(i, t)| {
            let t = t.to_ascii_lowercase();
            i != m && (t.starts_with("dyn") || t.starts_with("learn") || t == "d")
        });
        let vlan = w
            .iter()
            .enumerate()
            .find(|(i, t)| *i != m && *i != w.len() - 1 && t.chars().all(|c| c.is_ascii_digit()))
            .map_or("", |(_, t)| *t);
        rows.push(vec![
            vlan.to_owned(),
            w[m].to_owned(),
            if learned { "dynamic" } else { "static" }.to_owned(),
            w[w.len() - 1].to_owned(),
        ]);
    }
    rows
}

/// MAC table rows: learned (dynamic) addresses on real ports, deduplicated per port.
pub fn mac_entries(family: Option<&str>, text: &str) -> Vec<MacEntry> {
    let parser = match family {
        Some("ruijie") => Some("ruijie.show_mac_address_table"),
        Some("cisco" | "generic") | None => Some("cisco.show_mac_address_table"),
        _ => None,
    };
    let rows = match parser {
        Some(id) => ALL
            .iter()
            .find(|p| p.id == id)
            .map(|p| (p.parse)(text))
            .unwrap_or_default(),
        None => generic_mac_rows(text),
    };
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
        if command.contains("route") || command.contains("default-gateway") {
            if d.gateway.is_none() {
                d.gateway = default_gateway(text);
            }
        } else if command.contains("mac") {
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
    let mut sources = Vec::new();
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
            Ok(text) => {
                if !looks_rejected(&text) {
                    sources.push((*command).to_owned());
                }
                if neighbour {
                    near.push(((*command).to_owned(), text));
                } else {
                    tables.push(((*command).to_owned(), text));
                }
            }
            Err(e) if e.code == crate::error::ErrorCode::ConnectionClosed => return Err(e),
            Err(_) => {}
        }
    }
    // The gateway: the first command that names one.
    for command in gateway_commands(family) {
        match sessions
            .capture(id, command, std::time::Duration::from_secs(30))
            .await
        {
            Ok(text) if default_gateway(&text).is_some() => {
                sources.push((*command).to_owned());
                tables.push(((*command).to_owned(), text));
                break;
            }
            Err(e) if e.code == crate::error::ErrorCode::ConnectionClosed => return Err(e),
            _ => {}
        }
    }
    let mut d = discovery(family, super::neighbors::neighbors(family, &near), &tables);
    d.sources = sources;
    Ok(d)
}

/// The device said it does not know the command.
fn looks_rejected(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    [
        "invalid input",
        "unknown command",
        "incomplete command",
        "invalid command",
        "error:",
        "syntax error",
    ]
    .iter()
    .any(|k| l.contains(k))
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
        assert!(host_commands(Some("fortinet")).is_empty());
        assert_eq!(host_commands(Some("alcatel"))[0], "show mac-learning");
    }

    #[test]
    fn other_families_mac_tables() {
        // Address first.
        let cx = mac_entries(
            Some("aruba"),
            "MAC age-time            : 300 seconds\nNumber of MAC addresses : 3\n\n\
             MAC Address          VLAN     Type                      Port\n\
             --------------------------------------------------------------\n\
             00:50:56:96:d8:6d    10       dynamic                   1/1/3\n\
             3c:52:82:00:0b:02    20       dynamic                   lag1\n\
             88:3a:30:aa:bb:00    1        static                    1/1/9\n",
        );
        assert_eq!(cx.len(), 2, "{cx:?}");
        assert_eq!(
            (
                cx[0].mac.as_str(),
                cx[0].vlan.as_deref(),
                cx[0].interface.as_str()
            ),
            ("00505696d86d", Some("10"), "1/1/3")
        );
        assert_eq!(cx[1].interface, "lag1");
        // Domain and VLAN first, an operation word before the port.
        let aos = mac_entries(
            Some("alcatel"),
            "   Domain    Vlan/SrvcId[ISId/vnId]            Mac Address         Type         Operation          Interface\n\
             ------------+----------------------+-------------------+------------------+-------------+-------------------------\n\
                   VLAN                    1   00:e0:b1:aa:bb:cc             dynamic      bridging                    1/1/5\n\
                   VLAN                   30   00:80:9f:11:22:33             dynamic      bridging                    1/1/7\n\n\
             Total number of Valid MAC addresses above = 2\n",
        );
        assert_eq!(aos.len(), 2, "{aos:?}");
        assert_eq!(
            (aos[1].vlan.as_deref(), aos[1].interface.as_str()),
            (Some("30"), "1/1/7")
        );
        // VLAN by name, a one-letter flag.
        let ex = mac_entries(
            Some("juniper"),
            "default             00:11:22:33:44:55   D        -   ge-0/0/1.0\n\
             default             00:11:22:33:44:66   S        -   ge-0/0/2.0\n",
        );
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].interface, "ge-0/0/1.0");
        assert!(mac_entries(Some("aruba"), "00:50:56:96:d8:6d\n% Invalid\n\0").is_empty());
    }

    #[test]
    fn default_gateway_in_every_layout() {
        let cases = [
            (
                "Gateway of last resort is 10.0.100.1 to network 0.0.0.0\n\nS*    0.0.0.0/0 [1/0] via 10.0.100.1\n",
                "10.0.100.1",
            ),
            (
                "Routing entry for 0.0.0.0/0, supernet\n  Known via \"static\", distance 1\n  Routing Descriptor Blocks:\n  * 10.0.0.254\n",
                "10.0.0.254",
            ),
            // Ruijie: `S* 0.0.0.0/0 [1/0] via 10.0.100.1`
            (
                "Codes:  C - Connected, S - Static\nGateway of last resort is no set\nS*    0.0.0.0/0 [1/0] via 10.0.100.1\nC     10.0.100.0/24 is directly connected, VLAN 1\n",
                "10.0.100.1",
            ),
            // AOS 8 `show ip routes`
            (
                " + = Equal cost multipath routes\n Total 3 routes\n\n  Dest Address       Gateway Addr        Age        Protocol\n------------------+-------------------+----------+-----------\n  0.0.0.0/0          10.0.100.1         5d 2h      STATIC\n  10.0.100.0/24     10.0.100.15        5d 2h      LOCAL\n",
                "10.0.100.1",
            ),
            // AOS-CX
            (
                "Displaying ipv4 routes selected for forwarding\n\n0.0.0.0/0, vrf default\n\tvia  10.0.100.1,  [1/0],  static\n10.0.100.0/24, vrf default\n\tvia  vlan1,  [0/0],  connected\n",
                "10.0.100.1",
            ),
            // Junos
            (
                "inet.0: 5 destinations, 5 routes\n\n0.0.0.0/0          *[Static/5] 3w2d 04:11:12\n                    >  to 10.0.100.1 via ge-0/0/0.0\n",
                "10.0.100.1",
            ),
            // Layer-2 IOS
            ("10.0.100.1\n", ""),
            ("ip default-gateway 10.0.100.1\n", "10.0.100.1"),
            ("Default Gateway : 10.0.100.1\n", "10.0.100.1"),
        ];
        for (text, want) in cases {
            assert_eq!(default_gateway(text).unwrap_or_default(), want, "{text}");
        }
        assert_eq!(
            default_gateway(
                "Gateway of last resort is not set\nC 10.0.0.0/24 is directly connected\n"
            ),
            None
        );
        assert_eq!(
            default_gateway("% Invalid input detected at '^' marker.\n"),
            None
        );
        assert!(looks_rejected("% Invalid input detected"));
        assert!(!looks_rejected("LLDP Neighbor Information"));
        assert!(gateway_commands(Some("alcatel")).contains(&"show ip routes"));
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
