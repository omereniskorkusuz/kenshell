//! `show mac address-table` — Cisco IOS/IOS-XE and NX-OS (also the old
//! `show mac-address-table`). Rows are found by the MAC token.

use super::is_noise;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_mac_address_table",
    families: &["cisco"],
    command: "show mac address-table",
    aliases: &["show mac-address-table"],
    columns: &[
        ("vlan", ColumnKind::Number),
        ("mac", ColumnKind::Text),
        ("type", ColumnKind::Text),
        ("port", ColumnKind::Iface),
    ],
    parse,
};

pub(crate) fn is_mac(s: &str) -> bool {
    let b = s.as_bytes();
    let hex = |r: &[u8]| r.iter().all(u8::is_ascii_hexdigit);
    match b.len() {
        14 => b[4] == b'.' && b[9] == b'.' && hex(&b[..4]) && hex(&b[5..9]) && hex(&b[10..]),
        17 => {
            (0..6).all(|i| hex(&b[i * 3..i * 3 + 2]))
                && (1..6).all(|i| matches!(b[i * 3 - 1], b':' | b'-'))
        }
        _ => false,
    }
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if is_noise(line) {
            continue;
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some(m) = w.iter().position(|t| is_mac(t)) else {
            continue;
        };
        if m == 0 || m + 2 > w.len() {
            continue;
        }
        let vlan = w[m - 1].trim_start_matches(['*', '+', 'G', '~']).to_owned();
        rows.push(vec![
            vlan,
            w[m].to_owned(),
            w[m + 1].to_owned(),
            w[w.len() - 1].to_owned(),
        ]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_mac_address_table.txt"));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ["All", "0100.0ccc.cccc", "STATIC", "CPU"]);
        assert_eq!(rows[2], ["10", "aabb.cc00.0100", "DYNAMIC", "Gi1/0/3"]);
    }

    #[test]
    fn nxos_fixture() {
        let rows = parse(include_str!("../fixtures/nxos_show_mac_address_table.txt"));
        assert_eq!(
            rows,
            [
                ["10", "0050.56aa.0001", "dynamic", "Eth1/1"],
                ["-", "5254.0012.3456", "static", "sup-eth1(R)"],
            ]
        );
    }

    #[test]
    fn mac_forms() {
        assert!(
            is_mac("00:1a:2b:3c:4d:5e") && is_mac("00-1A-2B-3C-4D-5E") && is_mac("aabb.ccdd.eeff")
        );
        assert!(!is_mac("aabb.ccdd.eefg") && !is_mac("1.2.3.4"));
        assert!(parse("0011.2233.4455").is_empty());
    }
}
