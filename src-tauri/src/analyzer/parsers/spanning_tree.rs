//! `show spanning-tree` — Cisco IOS/IOS-XE (PVST/RSTP/MST): one row per port per instance.

use super::is_noise;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_spanning_tree",
    families: &["cisco"],
    command: "show spanning-tree",
    aliases: &[],
    columns: &[
        ("instance", ColumnKind::Text),
        ("interface", ColumnKind::Iface),
        ("role", ColumnKind::Text),
        ("state", ColumnKind::Status),
        ("cost", ColumnKind::Number),
        ("priority", ColumnKind::Text),
        ("type", ColumnKind::Text),
        ("root", ColumnKind::Text),
    ],
    parse,
};

const ROLES: &[&str] = &[
    "Root", "Desg", "Altn", "Back", "Mstr", "Shr", "Disa", "Bkup",
];

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut instance = String::new();
    let mut is_root = false;
    for line in text.lines() {
        if is_noise(line) {
            continue;
        }
        let t = line.trim();
        if !line.starts_with(char::is_whitespace)
            && (t.starts_with("VLAN") || t.starts_with("MST"))
            && t.split_whitespace().count() == 1
        {
            instance = t.to_owned();
            is_root = false;
            continue;
        }
        if t.contains("This bridge is the root") {
            is_root = true;
            continue;
        }
        let w: Vec<&str> = t.split_whitespace().collect();
        if w.len() >= 5 && ROLES.contains(&w[1]) && !line.starts_with(char::is_whitespace) {
            rows.push(vec![
                instance.clone(),
                w[0].to_owned(),
                w[1].to_owned(),
                w[2].to_owned(),
                w[3].to_owned(),
                w[4].to_owned(),
                w[5..].join(" "),
                if is_root {
                    "root".to_owned()
                } else {
                    String::new()
                },
            ]);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_spanning_tree.txt"));
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0],
            [
                "VLAN0001", "Gi1/0/1", "Desg", "FWD", "4", "128.1", "P2p Edge", ""
            ]
        );
        assert_eq!(rows[2][2..4], ["Altn", "BLK"]);
        assert_eq!(
            rows[3],
            [
                "VLAN0010", "Gi1/0/3", "Desg", "FWD", "4", "128.3", "P2p Edge", "root"
            ]
        );
    }

    #[test]
    fn garbage() {
        assert!(parse("Interface Role Sts\nfoo bar").is_empty());
    }
}
