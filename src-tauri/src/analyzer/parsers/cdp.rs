//! `show cdp neighbors` and `show cdp neighbors detail` — Cisco. In the brief form a long
//! Device ID sits alone on its line and the data follows on the next one.

use super::{cut, field, header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static BRIEF: ParserDef = ParserDef {
    id: "cisco.show_cdp_neighbors",
    families: &["cisco"],
    command: "show cdp neighbors",
    aliases: &[],
    columns: &[
        ("device", ColumnKind::Text),
        ("localInterface", ColumnKind::Iface),
        ("holdtime", ColumnKind::Number),
        ("capability", ColumnKind::Text),
        ("platform", ColumnKind::Text),
        ("remoteInterface", ColumnKind::Iface),
    ],
    parse: parse_brief,
};

pub static DETAIL: ParserDef = ParserDef {
    id: "cisco.show_cdp_neighbors_detail",
    families: &["cisco"],
    command: "show cdp neighbors detail",
    aliases: &[],
    columns: &[
        ("device", ColumnKind::Text),
        ("ip", ColumnKind::Ip),
        ("platform", ColumnKind::Text),
        ("capability", ColumnKind::Text),
        ("localInterface", ColumnKind::Iface),
        ("remoteInterface", ColumnKind::Iface),
        ("version", ColumnKind::Text),
    ],
    parse: parse_detail,
};

fn parse_brief(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    let mut pending: Option<String> = None;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("device id") {
            offs = header_offsets(
                line,
                &[
                    "device id",
                    "local intrfce",
                    "holdtme",
                    "capability",
                    "platform",
                    "port id",
                ],
            );
            continue;
        }
        if is_noise(line) || lower.starts_with("total ") {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        // A long Device ID stands alone on its line (it can run past its column).
        if !line.starts_with(char::is_whitespace) && line.split_whitespace().count() == 1 {
            pending = Some(line.trim().to_owned());
            continue;
        }
        let c = cut(line, o);
        if c[1].is_empty() {
            continue;
        }
        let device = if c[0].is_empty() {
            pending.take().unwrap_or_default()
        } else {
            c[0].clone()
        };
        rows.push(vec![
            device,
            c[1].clone(),
            c[2].clone(),
            c[3].clone(),
            c[4].clone(),
            c[5].clone(),
        ]);
    }
    rows
}

fn parse_detail(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_version = false;
    for line in text.lines() {
        if let Some(v) = field(line, "device id") {
            rows.push(vec![
                v.to_owned(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ]);
            in_version = false;
            continue;
        }
        let Some(row) = rows.last_mut() else { continue };
        let t = line.trim();
        if in_version {
            if !t.is_empty() && row[6].is_empty() {
                row[6] = t.to_owned();
            }
            in_version = false;
            continue;
        }
        if let Some(ip) = field(line, "ip address").or_else(|| field(line, "ipv4 address")) {
            if row[1].is_empty() {
                row[1] = ip.to_owned();
            }
        } else if t.starts_with("Platform:") {
            // `Platform: cisco WS-C3850-48P,  Capabilities: Router Switch IGMP`
            let (p, caps) = t.split_once("Capabilities:").unwrap_or((t, ""));
            row[2] = p
                .trim_start_matches("Platform:")
                .trim()
                .trim_end_matches(',')
                .trim()
                .to_owned();
            row[3] = caps.trim().to_owned();
        } else if t.starts_with("Interface:") {
            let (l, r) = t.split_once("Port ID (outgoing port):").unwrap_or((t, ""));
            row[4] = l
                .trim_start_matches("Interface:")
                .trim()
                .trim_end_matches(',')
                .trim()
                .to_owned();
            row[5] = r.trim().to_owned();
        } else if t.starts_with("Version") && t.trim_start_matches("Version").trim() == ":" {
            in_version = true;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_fixture_with_wrapped_device_id() {
        let rows = parse_brief(include_str!("../fixtures/cisco_show_cdp_neighbors.txt"));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            [
                "core-sw1.lab.example.com",
                "Gig 1/0/24",
                "150",
                "R S I",
                "WS-C3850-",
                "Gig 1/0/1"
            ]
        );
        assert_eq!(rows[2][4..], ["IP Phone", "Port 1"]);
    }

    #[test]
    fn detail_fixture() {
        let rows = parse_detail(include_str!(
            "../fixtures/cisco_show_cdp_neighbors_detail.txt"
        ));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0][..6],
            [
                "core-sw1.lab.example.com",
                "10.0.0.1",
                "cisco WS-C3850-48P",
                "Router Switch IGMP",
                "GigabitEthernet1/0/24",
                "GigabitEthernet1/0/1"
            ]
        );
        assert!(rows[0][6].contains("Version 16.6.4"));
        assert_eq!(rows[1][1], "10.20.0.15");
    }

    #[test]
    fn garbage() {
        assert!(parse_brief("Gig 1/0/1 x").is_empty());
        assert!(parse_detail("Platform: x").is_empty());
        let _ = parse_brief("Device ID\n\n x");
    }
}
