//! `show poe interfaces status` — Ruijie RGOS PoE models.

use super::super::{is_noise, take_iface};
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_poe_interfaces_status",
    families: &["ruijie"],
    command: "show poe interfaces status",
    aliases: &[],
    columns: &[
        ("interface", ColumnKind::Iface),
        ("poeEnable", ColumnKind::Text),
        ("status", ColumnKind::Status),
        ("maxPower", ColumnKind::Number),
        ("currentPower", ColumnKind::Number),
        ("averagePower", ColumnKind::Number),
        ("peakPower", ColumnKind::Number),
        ("priority", ColumnKind::Text),
    ],
    parse,
};

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        if is_noise(line) || line.starts_with(char::is_whitespace) || line.starts_with('-') {
            continue;
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        if w.first()
            .is_some_and(|f| f.eq_ignore_ascii_case("interface"))
        {
            continue;
        }
        let Some((name, rest)) = take_iface(&w) else {
            continue;
        };
        if rest.len() < 7 || !matches!(rest[0], "enable" | "disable") {
            continue;
        }
        let mut r = vec![name];
        r.extend(rest[..7].iter().map(|s| (*s).to_owned()));
        rows.push(r);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!(
            "../../fixtures/ruijie_show_poe_interfaces_status.txt"
        ));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            ["Gi0/1", "enable", "on", "30.0", "4.2", "4.1", "5.0", "low"]
        );
        assert_eq!(rows[2][1..3], ["disable", "off"]);
    }

    #[test]
    fn garbage() {
        assert!(parse("Gi0/1 maybe on").is_empty());
    }
}
