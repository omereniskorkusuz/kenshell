//! `show lldp neighbors` and `show lldp neighbors detail` — Cisco (the brief form is also
//! close to Ruijie's; P4-T03 decides whether Ruijie uses it).

use super::{cut, field, header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static BRIEF: ParserDef = ParserDef {
    id: "cisco.show_lldp_neighbors",
    families: &["cisco"],
    command: "show lldp neighbors",
    aliases: &[],
    columns: &[
        ("device", ColumnKind::Text),
        ("localInterface", ColumnKind::Iface),
        ("holdtime", ColumnKind::Number),
        ("capability", ColumnKind::Text),
        ("remoteInterface", ColumnKind::Iface),
    ],
    parse: parse_brief,
};

pub static DETAIL: ParserDef = ParserDef {
    id: "cisco.show_lldp_neighbors_detail",
    families: &["cisco"],
    command: "show lldp neighbors detail",
    aliases: &[],
    columns: &[
        ("device", ColumnKind::Text),
        ("ip", ColumnKind::Ip),
        ("localInterface", ColumnKind::Iface),
        ("remoteInterface", ColumnKind::Iface),
        ("portDescription", ColumnKind::Text),
        ("chassis", ColumnKind::Text),
        ("capability", ColumnKind::Text),
        ("description", ColumnKind::Text),
    ],
    parse: parse_detail,
};

fn parse_brief(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("device id") {
            offs = header_offsets(
                line,
                &[
                    "device id",
                    "local intf",
                    "hold-time",
                    "capability",
                    "port id",
                ],
            );
            continue;
        }
        if is_noise(line) || lower.starts_with("total ") {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let c = cut(line, o);
        if c[0].is_empty() || c[1].is_empty() {
            continue;
        }
        rows.push(c);
    }
    rows
}

fn parse_detail(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_desc = false;
    for line in text.lines() {
        if let Some(v) = field(line, "local intf") {
            rows.push(vec![String::new(); 8]);
            if let Some(r) = rows.last_mut() {
                r[2] = v.to_owned();
            }
            in_desc = false;
            continue;
        }
        let Some(r) = rows.last_mut() else { continue };
        let t = line.trim();
        if in_desc {
            if !t.is_empty() && r[7].is_empty() {
                r[7] = t.to_owned();
            }
            in_desc = false;
            continue;
        }
        if let Some(v) = field(line, "system name") {
            r[0] = v.to_owned();
        } else if let Some(v) = field(line, "ip") {
            if r[1].is_empty() {
                r[1] = v.to_owned();
            }
        } else if let Some(v) = field(line, "port id") {
            r[3] = v.to_owned();
        } else if let Some(v) = field(line, "port description") {
            r[4] = v.to_owned();
        } else if let Some(v) = field(line, "chassis id") {
            r[5] = v.to_owned();
        } else if let Some(v) = field(line, "system capabilities") {
            r[6] = v.to_owned();
        } else if let Some(v) = field(line, "system description") {
            if v.is_empty() {
                in_desc = true;
            } else {
                r[7] = v.to_owned();
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_fixture() {
        let rows = parse_brief(include_str!("../fixtures/cisco_show_lldp_neighbors.txt"));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], ["access-sw2", "Gi1/0/48", "120", "B,R", "Gi0/1"]);
        assert_eq!(rows[1][4], "GigabitEthernet 0/49");
    }

    #[test]
    fn detail_fixture() {
        let rows = parse_detail(include_str!(
            "../fixtures/cisco_show_lldp_neighbors_detail.txt"
        ));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0][..7],
            [
                "access-sw2",
                "10.0.0.12",
                "Gi1/0/48",
                "Gi0/1",
                "GigabitEthernet0/1",
                "0011.2233.4400",
                "B,R"
            ]
        );
        assert!(rows[0][7].starts_with("Cisco IOS Software, C2960X"));
        assert_eq!(rows[1][3], "GigabitEthernet 0/49");
        assert!(rows[1][7].contains("Ruijie"));
    }

    #[test]
    fn garbage() {
        assert!(parse_brief("access-sw2 Gi1/0/48 120 B Gi0/1").is_empty());
        assert!(parse_detail("System Name: x").is_empty());
    }
}
