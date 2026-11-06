//! `show interfaces status` — Ruijie RGOS (spaced names; optional Description column).

use super::super::{cut, header_offsets, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_interfaces_status",
    families: &["ruijie"],
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
    // Offsets and whether a Description column exists.
    let mut layout: Option<(Vec<usize>, bool)> = None;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("interface") && lower.contains("status") && lower.contains("vlan") {
            let desc = lower.contains("description");
            let titles: &[&str] = if desc {
                &[
                    "interface",
                    "description",
                    "status",
                    "vlan",
                    "duplex",
                    "speed",
                    "type",
                ]
            } else {
                &["interface", "status", "vlan", "duplex", "speed", "type"]
            };
            layout = header_offsets(line, titles).map(|o| (o, desc));
            continue;
        }
        if is_noise(line) || line.trim_start().starts_with('-') {
            continue;
        }
        let Some((o, desc)) = layout.as_ref() else {
            continue;
        };
        let mut c = cut(line, o);
        if !desc {
            c.insert(1, String::new());
        }
        if c[0].is_empty() || c[2].is_empty() {
            continue;
        }
        rows.push(c);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!(
            "../../fixtures/ruijie_show_interfaces_status.txt"
        ));
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0],
            [
                "GigabitEthernet 0/1",
                "",
                "up",
                "1",
                "Full",
                "1000M",
                "copper"
            ]
        );
        assert_eq!(rows[2][3], "Trunk");
        assert_eq!(
            rows[3],
            ["AggregatePort 1", "", "up", "Trunk", "Full", "2000M", ""]
        );
    }

    #[test]
    fn with_description_column() {
        let t = "Interface              Description   Status    Vlan   Duplex   Speed     Type\n\
                 ---------------------- ------------- --------  ----   -------  --------- ------\n\
                 GigabitEthernet 0/1    Uplink core   up        1      Full     1000M     copper\n";
        assert_eq!(
            parse(t)[0][..3],
            ["GigabitEthernet 0/1", "Uplink core", "up"]
        );
        assert!(parse("garbage").is_empty());
    }
}
