//! `show lldp neighbors` and `show lldp neighbors detail` — Ruijie RGOS.

use super::super::{cut, field, header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static BRIEF: ParserDef = ParserDef {
    id: "ruijie.show_lldp_neighbors",
    families: &["ruijie"],
    command: "show lldp neighbors",
    aliases: &[],
    columns: &[
        ("device", ColumnKind::Text),
        ("localInterface", ColumnKind::Iface),
        ("remoteInterface", ColumnKind::Iface),
        ("capability", ColumnKind::Text),
        ("age", ColumnKind::Text),
    ],
    parse: parse_brief,
};

pub static DETAIL: ParserDef = ParserDef {
    id: "ruijie.show_lldp_neighbors_detail",
    families: &["ruijie"],
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
        if lower.starts_with("system name") {
            offs = header_offsets(
                line,
                &[
                    "system name",
                    "local intf",
                    "port id",
                    "capability",
                    "aging-time",
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
    for line in text.lines() {
        // `LLDP neighbor-information of port [GigabitEthernet 0/49]`
        if let Some(i) = line.find("of port [") {
            let local = line[i + 9..].trim_end().trim_end_matches(']').to_owned();
            let mut r = vec![String::new(); 8];
            r[2] = local;
            rows.push(r);
            continue;
        }
        let Some(r) = rows.last_mut() else { continue };
        let set = |slot: &mut String, v: &str| {
            if slot.is_empty() {
                *slot = v.to_owned();
            }
        };
        if let Some(v) = field(line, "system name") {
            set(&mut r[0], v);
        } else if let Some(v) = field(line, "management address") {
            set(&mut r[1], v);
        } else if let Some(v) = field(line, "port id") {
            set(&mut r[3], v);
        } else if let Some(v) = field(line, "port description") {
            set(&mut r[4], v);
        } else if let Some(v) = field(line, "chassis id") {
            set(&mut r[5], v);
        } else if let Some(v) = field(line, "system capabilities supported") {
            set(&mut r[6], v);
        } else if let Some(v) = field(line, "system description") {
            set(&mut r[7], v);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_fixture() {
        let rows = parse_brief(include_str!(
            "../../fixtures/ruijie_show_lldp_neighbors.txt"
        ));
        assert_eq!(
            rows,
            [
                ["core-sw1", "Gi0/49", "Gi1/0/47", "B, R", "1min 45s"],
                ["ap-lobby", "Gi0/5", "eth0", "W", "1min 52s"],
            ]
        );
    }

    #[test]
    fn detail_fixture() {
        let rows = parse_detail(include_str!(
            "../../fixtures/ruijie_show_lldp_neighbors_detail.txt"
        ));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0][..7],
            [
                "core-sw1",
                "10.0.0.1",
                "GigabitEthernet 0/49",
                "Gi1/0/47",
                "GigabitEthernet1/0/47",
                "0011.2233.4400",
                "Bridge, Router"
            ]
        );
        assert_eq!(
            rows[1][..4],
            ["ap-lobby", "10.20.0.31", "GigabitEthernet 0/5", "eth0"]
        );
    }

    #[test]
    fn garbage() {
        assert!(parse_detail("System name : x").is_empty());
        assert!(parse_brief("core-sw1 Gi0/49").is_empty());
    }
}
