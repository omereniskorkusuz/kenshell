//! `show interfaces status` — Cisco IOS/IOS-XE. Fixed-width: the description (Name) is cut
//! by the header's column offsets; the rest is split on spaces (Type may contain spaces:
//! `Not Present`).

use super::{header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_interfaces_status",
    families: &["cisco"],
    command: "show interfaces status",
    aliases: &[],
    columns: &[
        ("interface", ColumnKind::Iface),
        ("description", ColumnKind::Text),
        ("status", ColumnKind::Status),
        ("vlan", ColumnKind::Text),
        ("duplex", ColumnKind::Text),
        ("speed", ColumnKind::Text),
        ("type", ColumnKind::Text),
    ],
    parse,
};

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        if is_noise(line) {
            continue;
        }
        if line.trim_start().to_ascii_lowercase().starts_with("port ") {
            offs = header_offsets(line, &["port", "name", "status"]);
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let chars: Vec<char> = line.chars().collect();
        let port: String = line.split_whitespace().next().unwrap_or("").to_owned();
        if port.is_empty() || chars.len() <= o[1] {
            continue;
        }
        // Name ends where the status column starts (Cisco cuts names to fit).
        let status_at = o[2].min(chars.len());
        let name: String = chars[o[1].min(status_at)..status_at].iter().collect();
        let rest: String = chars[status_at..].iter().collect();
        let w: Vec<&str> = rest.split_whitespace().collect();
        if w.is_empty() {
            continue;
        }
        rows.push(vec![
            port,
            name.trim().to_owned(),
            w[0].to_owned(),
            w.get(1).copied().unwrap_or("").to_owned(),
            w.get(2).copied().unwrap_or("").to_owned(),
            w.get(3).copied().unwrap_or("").to_owned(),
            w.get(4..).map(|t| t.join(" ")).unwrap_or_default(),
        ]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_interfaces_status.txt"));
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows[0],
            [
                "Gi1/0/1",
                "Uplink to core",
                "connected",
                "trunk",
                "full",
                "1000",
                "10/100/1000BaseTX"
            ]
        );
        assert_eq!(rows[1][1], "");
        assert_eq!(rows[2][1..3], ["Printer 2nd floor", "err-disabled"]);
        assert_eq!(rows[4][6], "Not Present");
        assert_eq!(
            rows[5],
            [
                "Po1",
                "to-core",
                "connected",
                "trunk",
                "a-full",
                "a-1000",
                ""
            ]
        );
    }

    #[test]
    fn no_header_no_rows() {
        assert!(parse("Gi1/0/1 x connected 1 full 1000 T").is_empty());
        let _ = parse("Port Name Status\n\u{1b}\nä");
    }
}
