//! `show mac-address-table` — Ruijie RGOS (interface names with spaces, `Live Time`).

use super::super::is_noise;
use super::super::mac_address_table::is_mac;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_mac_address_table",
    families: &["ruijie"],
    command: "show mac-address-table",
    aliases: &["show mac address-table"],
    columns: &[
        ("vlan", ColumnKind::Number),
        ("mac", ColumnKind::Text),
        ("type", ColumnKind::Text),
        ("port", ColumnKind::Iface),
        ("age", ColumnKind::Text),
    ],
    parse,
};

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
        if m == 0 || w.len() < m + 3 {
            continue;
        }
        // Interface: one word, or a spaced name (`GigabitEthernet 0/1`).
        let rest = &w[m + 2..];
        let (port, used) = match rest {
            [a, b, ..]
                if !a.chars().any(|c| c.is_ascii_digit())
                    && b.starts_with(|c: char| c.is_ascii_digit())
                    && !b.contains(':') =>
            {
                (format!("{a} {b}"), 2)
            }
            [a, ..] => ((*a).to_owned(), 1),
            [] => continue,
        };
        rows.push(vec![
            w[m - 1].to_owned(),
            w[m].to_owned(),
            w[m + 1].to_owned(),
            port,
            rest[used..].join(" "),
        ]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!(
            "../../fixtures/ruijie_show_mac_address_table.txt"
        ));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            [
                "1",
                "0011.2233.4455",
                "DYNAMIC",
                "GigabitEthernet 0/1",
                "0d 00:10:12"
            ]
        );
        assert_eq!(
            rows[1],
            ["10", "aabb.cc00.0100", "STATIC", "AggregatePort 1", "-"]
        );
        assert_eq!(rows[2][3], "TenGigabitEthernet 0/49");
    }

    #[test]
    fn garbage() {
        assert!(parse("0011.2233.4455 x").is_empty());
    }
}
