//! `show etherchannel summary` — Cisco IOS/IOS-XE. Member lists continue on later lines.

use super::is_noise;
use crate::analyzer::{ColumnKind, ParserDef};

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_etherchannel_summary",
    families: &["cisco"],
    command: "show etherchannel summary",
    aliases: &[],
    columns: &[
        ("group", ColumnKind::Number),
        ("portChannel", ColumnKind::Iface),
        ("flags", ColumnKind::Text),
        ("protocol", ColumnKind::Text),
        ("members", ColumnKind::Text),
    ],
    parse,
};

/// `Po1(SU)` → (`Po1`, `SU`).
fn split_flags(s: &str) -> (String, String) {
    match s.split_once('(') {
        Some((n, f)) => (n.to_owned(), f.trim_end_matches(')').to_owned()),
        None => (s.to_owned(), String::new()),
    }
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut started = false;
    for line in text.lines() {
        if line.starts_with("------") {
            started = true;
            continue;
        }
        if !started || is_noise(line) {
            continue;
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        if line.starts_with(char::is_whitespace) {
            if let Some(r) = rows.last_mut() {
                for m in w {
                    if !r[4].is_empty() {
                        r[4].push(' ');
                    }
                    r[4].push_str(m);
                }
            }
            continue;
        }
        if w.len() < 3 || !w[0].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let (pc, flags) = split_flags(w[1]);
        rows.push(vec![
            w[0].to_owned(),
            pc,
            flags,
            w[2].to_owned(),
            w[3..].join(" "),
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
            "../fixtures/cisco_show_etherchannel_summary.txt"
        ));
        assert_eq!(
            rows,
            [
                ["1", "Po1", "SU", "LACP", "Gi1/0/1(P) Gi1/0/2(P)"],
                ["2", "Po2", "SD", "-", "Gi1/0/3(D) Gi1/0/4(s) Gi1/0/5(s)"],
            ]
        );
    }

    #[test]
    fn garbage() {
        assert!(
            parse("1 Po1(SU) LACP").is_empty(),
            "nothing before the dashes"
        );
        let _ = parse("------\n x\n1\n1 (\n");
    }
}
