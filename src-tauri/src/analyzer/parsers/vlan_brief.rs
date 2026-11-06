//! `show vlan brief` — Cisco IOS/IOS-XE. Columns from the dashes line; port lists continue
//! on the next lines.

use super::{cut, is_noise};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_vlan_brief",
    families: &["cisco"],
    command: "show vlan brief",
    aliases: &[],
    columns: &[
        ("vlan", ColumnKind::Number),
        ("name", ColumnKind::Text),
        ("status", ColumnKind::Status),
        ("ports", ColumnKind::Text),
    ],
    parse,
};

/// Column starts from a `---- ------ ---` line.
pub(crate) fn dash_offsets(line: &str) -> Option<Vec<usize>> {
    let t = line.trim_end();
    if t.is_empty() || !t.chars().all(|c| c == '-' || c == ' ' || c == '+') {
        return None;
    }
    let mut out = Vec::new();
    let mut prev = ' ';
    for (i, c) in t.chars().enumerate() {
        if c == '-' && (prev == ' ' || prev == '+') {
            out.push(i);
        }
        prev = c;
    }
    Some(out)
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut offs: Option<Vec<usize>> = None;
    for line in text.lines() {
        if let Some(o) = dash_offsets(line) {
            if o.len() >= 4 {
                offs = Some(o);
            }
            continue;
        }
        if is_noise(line) {
            continue;
        }
        let Some(o) = offs.as_ref() else { continue };
        let c = cut(line, &o[..4]);
        if c[0].is_empty() {
            // Port list continued.
            if let Some(last) = rows.last_mut()
                && !c[3].is_empty()
            {
                last[3] = if last[3].is_empty() {
                    c[3].clone()
                } else {
                    format!("{}, {}", last[3], c[3])
                };
            }
            continue;
        }
        if !c[0].chars().all(|ch| ch.is_ascii_digit()) {
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
        let rows = parse(include_str!("../fixtures/cisco_show_vlan_brief.txt"));
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[0],
            [
                "1",
                "default",
                "active",
                "Gi1/0/2, Gi1/0/5, Gi1/0/6, Gi1/0/7, Gi1/0/8"
            ]
        );
        assert_eq!(rows[3], ["99", "MGMT", "active", ""]);
        assert_eq!(rows[4][2], "act/unsup");
    }

    #[test]
    fn garbage() {
        assert!(parse("VLAN Name\n1 default active").is_empty());
        assert_eq!(dash_offsets("---- --- --"), Some(vec![0, 5, 9]));
        assert_eq!(dash_offsets("abc"), None);
    }
}
