//! `show inventory` — Cisco IOS/IOS-XE/NX-OS: `NAME: "…", DESCR: "…"` then
//! `PID: …, VID: …, SN: …`.

use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_inventory",
    families: &["cisco"],
    command: "show inventory",
    aliases: &[],
    columns: &[
        ("name", ColumnKind::Text),
        ("description", ColumnKind::Text),
        ("pid", ColumnKind::Text),
        ("vid", ColumnKind::Text),
        ("serial", ColumnKind::Text),
    ],
    parse,
};

/// `KEY: value` pairs of a comma separated line (values may be quoted and contain commas).
fn pairs(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    while let Some((k, v)) = rest.split_once(':') {
        let key = k.trim().trim_start_matches(',').trim().to_ascii_uppercase();
        let v = v.trim_start();
        let (value, tail) = if let Some(q) = v.strip_prefix('"') {
            match q.split_once('"') {
                Some((val, t)) => (val.to_owned(), t),
                None => (q.to_owned(), ""),
            }
        } else {
            match v.find(',') {
                Some(i) => (v[..i].trim().to_owned(), &v[i..]),
                None => (v.trim().to_owned(), ""),
            }
        };
        out.push((key, value));
        rest = tail.trim_start_matches([',', ' ']);
    }
    out
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        let p = pairs(line);
        let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        if let Some(name) = get("NAME") {
            rows.push(vec![
                name,
                get("DESCR").unwrap_or_default(),
                String::new(),
                String::new(),
                String::new(),
            ]);
        } else if let (Some(pid), Some(row)) = (get("PID"), rows.last_mut()) {
            row[2] = pid;
            row[3] = get("VID").unwrap_or_default();
            row[4] = get("SN").unwrap_or_default();
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_inventory.txt"));
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[1],
            [
                "Switch 1",
                "WS-C3850-48P",
                "WS-C3850-48P",
                "V07",
                "FCW1234L0CD"
            ]
        );
        assert_eq!(rows[3][2..], ["SFP-10G-SR", "V03", "AVD1234567"]);
    }

    #[test]
    fn garbage() {
        assert!(parse("PID: x, VID: y").is_empty());
        let _ = parse("NAME: \"unterminated\nNAME:\n:::");
    }
}
