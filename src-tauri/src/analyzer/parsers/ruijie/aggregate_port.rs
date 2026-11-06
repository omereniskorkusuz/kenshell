//! `show aggregatePort summary` — Ruijie RGOS (link aggregation, Cisco's EtherChannel).

use super::super::vlan_brief::dash_offsets;
use super::super::{cut, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_aggregateport_summary",
    families: &["ruijie"],
    command: "show aggregatePort summary",
    aliases: &[],
    columns: &[
        ("portChannel", ColumnKind::Iface),
        ("maxPorts", ColumnKind::Number),
        ("switchport", ColumnKind::Text),
        ("mode", ColumnKind::Text),
        ("loadBalance", ColumnKind::Text),
        ("members", ColumnKind::Text),
    ],
    parse,
};

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        if let Some(o) = dash_offsets(line) {
            if o.len() >= 6 {
                offs = Some(o);
            }
            continue;
        }
        if is_noise(line) {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let mut c = cut(line, &o[..6]);
        if c[0].is_empty() || c[0].eq_ignore_ascii_case("aggregateport") {
            continue;
        }
        // `Gi0/23  ,Gi0/24` → `Gi0/23, Gi0/24`
        c[5] = c[5]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
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
            "../../fixtures/ruijie_show_aggregateport_summary.txt"
        ));
        assert_eq!(
            rows,
            [
                [
                    "Ag1",
                    "16",
                    "Enabled",
                    "TRUNK",
                    "src-dst-mac",
                    "Gi0/23, Gi0/24"
                ],
                ["Ag2", "16", "Enabled", "ACCESS", "src-dst-ip", "Te0/49"],
            ]
        );
    }

    #[test]
    fn garbage() {
        assert!(parse("Ag1 16 Enabled").is_empty());
    }
}
