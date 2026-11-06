//! `show cpu` — Ruijie RGOS: the 5 s / 1 min / 5 min totals as a first row, then processes.

use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "ruijie.show_cpu",
    families: &["ruijie"],
    command: "show cpu",
    aliases: &[],
    columns: &[
        ("process", ColumnKind::Text),
        ("sec5", ColumnKind::Number),
        ("min1", ColumnKind::Number),
        ("min5", ColumnKind::Number),
        ("no", ColumnKind::Number),
    ],
    parse,
};

fn pct(line: &str) -> String {
    line.rsplit(':')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('%')
        .to_owned()
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut total = vec![
        "(total)".to_owned(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    ];
    let mut procs = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("cpu utilization in five seconds") {
            total[1] = pct(t);
        } else if lower.starts_with("cpu utilization in one minute") {
            total[2] = pct(t);
        } else if lower.starts_with("cpu utilization in five minutes") {
            total[3] = pct(t);
        } else {
            let w: Vec<&str> = t.split_whitespace().collect();
            if w.len() >= 5 && w[0].bytes().all(|b| b.is_ascii_digit()) && w[1].ends_with('%') {
                procs.push(vec![
                    w[4..].join(" "),
                    w[1].trim_end_matches('%').to_owned(),
                    w[2].trim_end_matches('%').to_owned(),
                    w[3].trim_end_matches('%').to_owned(),
                    w[0].to_owned(),
                ]);
            }
        }
    }
    let mut rows = Vec::new();
    if total[1..4].iter().any(|v| !v.is_empty()) {
        rows.push(total);
    }
    rows.extend(procs);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../../fixtures/ruijie_show_cpu.txt"));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ["(total)", "6.20", "5.90", "5.85", ""]);
        assert_eq!(rows[1], ["LISR INT", "0.00", "0.00", "0.00", "0"]);
        assert_eq!(rows[3][0], "ksoftirqd/0");
    }

    #[test]
    fn garbage() {
        assert!(parse("1 2 3 4 5").is_empty());
    }
}
