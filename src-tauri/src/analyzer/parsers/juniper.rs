//! Juniper Junos parser set (P4-T04): `show interfaces terse`, `show version`,
//! `show chassis hardware`, `show lldp neighbors`.

use super::{cut, header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static INTERFACES_TERSE: ParserDef = ParserDef {
    id: "juniper.show_interfaces_terse",
    families: &["juniper"],
    command: "show interfaces terse",
    aliases: &[],
    columns: &[
        ("interface", ColumnKind::Text),
        ("admin", ColumnKind::Status),
        ("link", ColumnKind::Status),
        ("proto", ColumnKind::Text),
        ("local", ColumnKind::Ip),
        ("remote", ColumnKind::Text),
    ],
    parse: parse_terse,
};

pub static VERSION: ParserDef = ParserDef {
    id: "juniper.show_version",
    families: &["juniper"],
    command: "show version",
    aliases: &[],
    columns: &[("field", ColumnKind::Text), ("value", ColumnKind::Text)],
    parse: parse_version,
};

pub static CHASSIS_HARDWARE: ParserDef = ParserDef {
    id: "juniper.show_chassis_hardware",
    families: &["juniper"],
    command: "show chassis hardware",
    aliases: &[],
    columns: &[
        ("item", ColumnKind::Text),
        ("version", ColumnKind::Text),
        ("partNumber", ColumnKind::Text),
        ("serial", ColumnKind::Text),
        ("description", ColumnKind::Text),
    ],
    parse: parse_hardware,
};

pub static LLDP_NEIGHBORS: ParserDef = ParserDef {
    id: "juniper.show_lldp_neighbors",
    families: &["juniper"],
    command: "show lldp neighbors",
    aliases: &[],
    columns: &[
        ("localInterface", ColumnKind::Text),
        ("parent", ColumnKind::Text),
        ("chassis", ColumnKind::Text),
        ("remoteInterface", ColumnKind::Iface),
        ("device", ColumnKind::Text),
    ],
    parse: parse_lldp,
};

/// Junos prompts end in `>` / `#` like others; also `{master:0}` banners.
fn junk(line: &str) -> bool {
    is_noise(line) || line.trim_start().starts_with('{')
}

fn parse_terse(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    let mut last = String::new();
    for line in text.lines() {
        if line.to_ascii_lowercase().starts_with("interface") {
            offs = header_offsets(
                line,
                &["interface", "admin", "link", "proto", "local", "remote"],
            );
            continue;
        }
        if junk(line) {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let mut c = cut(line, o);
        if c[0].is_empty() {
            // More addresses of the previous logical interface.
            if last.is_empty() || (c[3].is_empty() && c[4].is_empty()) {
                continue;
            }
            c[0] = last.clone();
        } else {
            last = c[0].clone();
        }
        // Local and Remote: the rest of the line (a long IPv6 address runs past its
        // column); Remote only ever follows `-->`.
        let rest: String = line.chars().skip(o[4]).collect();
        let (local, remote) = rest.split_once("-->").unwrap_or((rest.as_str(), ""));
        c[4] = local.trim().to_owned();
        c[5] = remote.trim().to_owned();
        rows.push(c);
    }
    rows
}

fn parse_version(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some((k, v)) = t.split_once(':')
            && !k.contains(' ')
            && !v.trim().is_empty()
        {
            let key = match k.to_ascii_lowercase().as_str() {
                "hostname" => "hostname".to_owned(),
                "model" => "model".to_owned(),
                "junos" => "version".to_owned(),
                other => other.to_owned(),
            };
            rows.push(vec![key, v.trim().to_owned()]);
        } else if let Some((name, ver)) = t.strip_suffix(']').and_then(|x| x.rsplit_once('[')) {
            rows.push(vec![name.trim().to_owned(), ver.to_owned()]);
        }
    }
    rows
}

fn parse_hardware(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        if line.to_ascii_lowercase().starts_with("item") {
            offs = header_offsets(
                line,
                &[
                    "item",
                    "version",
                    "part number",
                    "serial number",
                    "description",
                ],
            );
            continue;
        }
        if junk(line) || line.ends_with(':') {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let c = cut(line, o);
        if !c[0].is_empty() {
            rows.push(c);
        }
    }
    rows
}

fn parse_lldp(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        if line.to_ascii_lowercase().starts_with("local interface") {
            offs = header_offsets(
                line,
                &[
                    "local interface",
                    "parent interface",
                    "chassis id",
                    "port info",
                ],
            );
            continue;
        }
        if junk(line) {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let c = cut(line, o);
        // Port info may run into System Name: the system name is the last word.
        let mut tail: Vec<&str> = c[3].split_whitespace().collect();
        if c[0].is_empty() || tail.len() < 2 {
            continue;
        }
        let device = tail.pop().unwrap_or_default().to_owned();
        rows.push(vec![
            c[0].clone(),
            c[1].clone(),
            c[2].clone(),
            tail.join(" "),
            device,
        ]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_fixture() {
        let rows = parse_terse(include_str!("../fixtures/junos_show_interfaces_terse.txt"));
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0], ["ge-0/0/0", "up", "up", "", "", ""]);
        assert_eq!(
            rows[1],
            ["ge-0/0/0.0", "up", "up", "inet", "10.0.0.1/24", ""]
        );
        assert_eq!(
            rows[2][0..5],
            ["ge-0/0/0.0", "", "", "inet6", "fe80::5254:ff:fe12:3456/64"]
        );
        assert_eq!(rows[3][..3], ["ge-0/0/1", "up", "down"]);
        assert_eq!(rows[7], ["lo0.0", "up", "up", "inet", "10.255.0.1", "0/0"]);
    }

    #[test]
    fn version_fixture() {
        let rows = parse_version(include_str!("../fixtures/junos_show_version.txt"));
        assert_eq!(
            rows[..3],
            [
                ["hostname", "mx1"],
                ["model", "mx204"],
                ["version", "21.4R3-S2.3"]
            ]
        );
        assert!(rows.iter().any(|r| r[0] == "JUNOS OS Kernel 64-bit"));
    }

    #[test]
    fn hardware_fixture() {
        let rows = parse_hardware(include_str!("../fixtures/junos_show_chassis_hardware.txt"));
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0], ["Chassis", "", "", "JN12345AFJA", "MX204"]);
        assert_eq!(
            rows[5],
            [
                "Xcvr 0",
                "REV 01",
                "740-058734",
                "1ACQ2345678",
                "QSFP-100GBASE-SR4"
            ]
        );
        assert_eq!(rows[6][4], "Fan Tray, Front to Back Airflow - AFO");
    }

    #[test]
    fn lldp_fixture() {
        let rows = parse_lldp(include_str!("../fixtures/junos_show_lldp_neighbors.txt"));
        assert_eq!(
            rows,
            [
                ["ge-0/0/0", "-", "00:11:22:33:44:00", "Gi1/0/1", "core-sw1"],
                [
                    "xe-0/1/0",
                    "ae0",
                    "58:69:6c:00:11:11",
                    "GigabitEthernet 0/49",
                    "ruijie-s5750"
                ],
            ]
        );
    }

    #[test]
    fn garbage() {
        assert!(parse_terse("ge-0/0/0 up up").is_empty());
        assert!(parse_hardware("Chassis x").is_empty());
        assert!(parse_lldp("ge-0/0/0").is_empty());
        let _ = parse_version("::\n[\n]");
    }
}
