//! `show ip interface brief` — Cisco IOS/IOS-XE (`OK? Method` columns) and Ruijie RGOS
//! (`IP-Address(Pri) IP-Address(Sec)`, names may be spaced: `GigabitEthernet 0/1`).
//! The layout is taken from the header line; columns a vendor does not have stay empty.

use super::{is_noise, take_iface};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_ip_interface_brief",
    families: &["cisco", "ruijie"],
    command: "show ip interface brief",
    aliases: &[],
    columns: &[
        ("interface", ColumnKind::Iface),
        ("ip", ColumnKind::Ip),
        ("secondary", ColumnKind::Ip),
        ("ok", ColumnKind::Text),
        ("method", ColumnKind::Text),
        ("status", ColumnKind::Status),
        ("protocol", ColumnKind::Status),
    ],
    parse,
};

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Cisco,
    Ruijie,
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut layout = Layout::Cisco;
    for line in text.lines() {
        if is_noise(line) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("interface") && lower.contains("status") {
            layout = if lower.contains("(sec)") {
                Layout::Ruijie
            } else {
                Layout::Cisco
            };
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((name, rest)) = take_iface(&words) else {
            continue;
        };
        let row = match layout {
            Layout::Cisco => {
                // ip ok method status… protocol (status may be `administratively down`).
                if rest.len() < 5 {
                    continue;
                }
                let tail = &rest[3..];
                vec![
                    name,
                    rest[0].into(),
                    String::new(),
                    rest[1].into(),
                    rest[2].into(),
                    tail[..tail.len() - 1].join(" "),
                    tail[tail.len() - 1].into(),
                ]
            }
            Layout::Ruijie => {
                // pri sec status protocol; `no address` is two words.
                if rest.len() < 4 {
                    continue;
                }
                let pri = if rest[0] == "no" && rest.get(1) == Some(&"address") {
                    2
                } else {
                    1
                };
                let after = &rest[pri..];
                let sec = if after.first() == Some(&"no") && after.get(1) == Some(&"address") {
                    2
                } else {
                    1
                };
                let tail = &after[sec.min(after.len())..];
                if tail.len() < 2 {
                    continue;
                }
                let addr = |w: &[&str]| {
                    if w.len() == 2 {
                        String::new()
                    } else {
                        w[0].to_owned()
                    }
                };
                vec![
                    name,
                    addr(&rest[..pri]),
                    addr(&after[..sec]),
                    String::new(),
                    String::new(),
                    tail[..tail.len() - 1].join(" "),
                    tail[tail.len() - 1].into(),
                ]
            }
        };
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cisco_fixture() {
        let rows = parse(include_str!(
            "../fixtures/cisco_show_ip_interface_brief.txt"
        ));
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows[0],
            [
                "Vlan1",
                "unassigned",
                "",
                "YES",
                "NVRAM",
                "administratively down",
                "down"
            ]
        );
        assert_eq!(
            rows[1],
            ["Vlan10", "10.0.10.2", "", "YES", "NVRAM", "up", "up"]
        );
        assert_eq!(rows[5][0], "TenGigabitEthernet1/1/1");
    }

    #[test]
    fn ruijie_fixture_with_spaced_names() {
        let rows = parse(include_str!(
            "../fixtures/ruijie_show_ip_interface_brief.txt"
        ));
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0],
            [
                "GigabitEthernet 0/1",
                "192.168.1.1/24",
                "",
                "",
                "",
                "up",
                "up"
            ]
        );
        assert_eq!(
            rows[1],
            ["Mgmt 0", "172.16.0.10/24", "", "", "", "up", "up"]
        );
        assert_eq!(
            rows[2],
            [
                "VLAN 10",
                "10.10.10.1/24",
                "10.10.11.1/24",
                "",
                "",
                "down",
                "down"
            ]
        );
        assert_eq!(
            rows[3],
            ["VLAN 20", "", "", "", "", "administratively down", "down"]
        );
    }

    #[test]
    fn garbage_gives_no_rows_and_no_panic() {
        for g in [
            "",
            "Interface",
            "x\n y\n\u{0}\u{1b}[31m",
            "Gi1/0/1",
            "Gi1/0/1 1.1.1.1 YES",
            "a b c d e f g h",
        ] {
            let _ = parse(g);
        }
        assert!(parse("random text\nmore text").is_empty());
    }
}
