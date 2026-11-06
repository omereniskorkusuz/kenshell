//! `show memory` — Ruijie RGOS, as field / value pairs (`Total memory: 975.7MB, …`).

use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_memory",
    families: &["ruijie"],
    command: "show memory",
    aliases: &[],
    columns: &[("field", ColumnKind::Text), ("value", ColumnKind::Text)],
    parse,
};

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        // `watermarks : min 1184, lower 1480 …` is one value; `a: 1, b: 2` are several.
        let parts: Vec<&str> = if t.matches(':').count() > 1 {
            t.split(',').collect()
        } else {
            vec![t]
        };
        for p in parts {
            if let Some((k, v)) = p.split_once(':') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() && k.len() <= 60 {
                    rows.push(vec![k.to_owned(), v.to_owned()]);
                }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../../fixtures/ruijie_show_memory.txt"));
        let get = |k: &str| rows.iter().find(|r| r[0] == k).map(|r| r[1].as_str());
        assert_eq!(get("Total memory"), Some("975.7MB"));
        assert_eq!(get("Used rate"), Some("85%"));
        assert_eq!(
            get("watermarks"),
            Some("min 1184, lower 1480, low 1776, high 2368")
        );
        assert_eq!(get("Free pages"), Some("37452"));
    }
}
