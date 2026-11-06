//! `show ip route` — Cisco IOS/IOS-XE. One row per next hop; equal-cost paths on the
//! following lines repeat the prefix.

use super::is_noise;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_ip_route",
    families: &["cisco"],
    command: "show ip route",
    aliases: &[],
    columns: &[
        ("code", ColumnKind::Text),
        ("prefix", ColumnKind::Ip),
        ("distanceMetric", ColumnKind::Text),
        ("nextHop", ColumnKind::Ip),
        ("interface", ColumnKind::Iface),
        ("age", ColumnKind::Text),
    ],
    parse,
};

fn looks_prefix(w: &str) -> bool {
    let ip = w.split('/').next().unwrap_or("");
    ip.split('.').count() == 4
        && ip
            .split('.')
            .all(|o| !o.is_empty() && o.len() <= 3 && o.bytes().all(|b| b.is_ascii_digit()))
}

/// `[110/2] via 10.0.0.3, 00:01:02, Vlan10` / `is directly connected, Vlan10`.
fn route_tail(rest: &str) -> (String, String, String, String) {
    let rest = rest.trim();
    let (dm, rest) = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((dm, r)) => (dm.to_owned(), r.trim()),
        None => (String::new(), rest),
    };
    if rest.starts_with("is local host") {
        return (dm, String::new(), String::new(), String::new());
    }
    if let Some(r) = rest.strip_prefix("is directly connected,") {
        return (dm, String::new(), r.trim().to_owned(), String::new());
    }
    let r = rest.strip_prefix("via").unwrap_or(rest).trim();
    let parts: Vec<&str> = r.split(',').map(str::trim).collect();
    let nh = parts.first().copied().unwrap_or("").to_owned();
    // The rest: an age (digits/colons, `1d02h`, `2w0d`) and/or an interface.
    let mut age = String::new();
    let mut iface = String::new();
    for p in parts.iter().skip(1) {
        if p.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            age = (*p).to_owned();
        } else {
            iface = (*p).to_owned();
        }
    }
    (dm, nh, iface, age)
}

pub(crate) fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut last: Option<(String, String)> = None;
    for line in text.lines() {
        if is_noise(line) {
            continue;
        }
        let t = line.trim();
        if t.starts_with('[') {
            // Another path for the previous prefix.
            if let Some((code, prefix)) = last.clone() {
                let (dm, nh, iface, age) = route_tail(t);
                rows.push(vec![code, prefix, dm, nh, iface, age]);
            }
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            continue; // `10.0.0.0/8 is variably subnetted …`
        }
        let w: Vec<&str> = t.split_whitespace().collect();
        let Some(pi) = w.iter().position(|x| looks_prefix(x)) else {
            continue;
        };
        if pi == 0 || pi > 3 {
            continue;
        }
        let code = w[..pi].join(" ");
        let prefix = w[pi].to_owned();
        let rest = t[t.find(&prefix).map_or(t.len(), |i| i + prefix.len())..].to_owned();
        let (dm, nh, iface, age) = route_tail(&rest);
        last = Some((code.clone(), prefix.clone()));
        rows.push(vec![code, prefix, dm, nh, iface, age]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_ip_route.txt"));
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0], ["S*", "0.0.0.0/0", "1/0", "10.0.0.1", "", ""]);
        assert_eq!(rows[1], ["C", "10.0.0.0/24", "", "", "Vlan10", ""]);
        assert_eq!(
            rows[3],
            [
                "O IA",
                "10.20.0.0/24",
                "110/2",
                "10.0.0.3",
                "Vlan10",
                "00:01:02"
            ]
        );
        assert_eq!(rows[4][3], "10.0.0.4");
        assert_eq!(
            rows[5],
            [
                "D EX",
                "192.168.5.0/24",
                "170/3072",
                "10.0.0.9",
                "GigabitEthernet1/0/1",
                "1d02h"
            ]
        );
        assert_eq!(rows[6][3..], ["203.0.113.1", "", "2w0d"]);
    }

    #[test]
    fn garbage() {
        assert!(parse("Gateway of last resort is 10.0.0.1 to network 0.0.0.0").is_empty());
        let _ = parse("[1/0]\nS 1.2.3.4 [\nX 999.1");
    }
}
