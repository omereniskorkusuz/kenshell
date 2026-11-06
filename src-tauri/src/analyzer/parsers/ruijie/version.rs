//! `show version` — Ruijie RGOS: `System … : value` lines, then per-slot module lines
//! (shown as `Slot 0 · Serial number`).

use super::super::field;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_version",
    families: &["ruijie"],
    command: "show version",
    aliases: &[],
    columns: &[("field", ColumnKind::Text), ("value", ColumnKind::Text)],
    parse,
};

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut slot = String::new();
    for line in text.lines() {
        let Some((k, _)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() || key.len() > 60 {
            continue;
        }
        let value = field(line, key).unwrap_or("").to_owned();
        let indent = line.len() - line.trim_start().len();
        if key.starts_with("Slot") {
            slot = key.to_owned();
            rows.push(vec![key.to_owned(), value]);
        } else if indent >= 4 && !slot.is_empty() {
            rows.push(vec![format!("{slot} · {key}"), value]);
        } else if !value.is_empty() {
            slot.clear();
            rows.push(vec![key.trim_start_matches("System ").to_owned(), value]);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../../fixtures/ruijie_show_version.txt"));
        let get = |k: &str| rows.iter().find(|r| r[0] == k).map(|r| r[1].as_str());
        assert_eq!(get("serial number"), Some("G1MQ3K8000123"));
        assert_eq!(get("software version"), Some("S5750_RGOS 11.4(1)B12P10S2"));
        assert_eq!(get("uptime"), Some("35:04:12:33"));
        assert_eq!(get("Slot 0"), Some("S5750-48GT4XS-E"));
        assert_eq!(get("Slot 0 · Boot version"), Some("1.3.8"));
    }

    #[test]
    fn garbage() {
        let _ = parse(":\n : \n::::\nSlot");
    }
}
