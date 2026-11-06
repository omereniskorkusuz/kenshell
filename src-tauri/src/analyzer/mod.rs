//! Analyzer (Phase 4): turns known `show` command output into tables.
//!
//! P4-T01: parser registry, command matching (abbreviations: `sh ip int br`), automatic
//! detection (the command given, else the echoed `prompt#command` line at the top of the
//! text) and CSV export. Each parser is one module under `parsers/` with its fixtures in
//! `fixtures/`. Device output is untrusted: input and table sizes are capped and parsers
//! never panic on malformed text (P4-T05 adds fuzz tests).

pub mod config_rules;
pub mod csv;
pub mod diagnostics;
#[cfg(test)]
mod fuzz_tests;
pub mod parsers;

use serde::{Deserialize, Serialize};

/// Largest text analysed at once (a `show tech` goes through the file analyser, P4-T09).
pub const MAX_INPUT: usize = 5 * 1024 * 1024;
/// Rows kept per table.
pub const MAX_ROWS: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKind {
    Text,
    /// Sorted numerically.
    Number,
    /// Interface name (normalised key, natural sort).
    Iface,
    /// IPv4/IPv6 address (sorted numerically).
    Ip,
    /// up / down style state (coloured in the table).
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    /// Stable id; the UI label is `analyzer.col.<key>` (falls back to the key).
    pub key: String,
    pub kind: ColumnKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// Parser id, e.g. `cisco.show_ip_interface_brief`.
    pub parser: String,
    /// Canonical command, e.g. `show ip interface brief`.
    pub command: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
    /// Rows were cut at [`MAX_ROWS`] or the input at [`MAX_INPUT`].
    pub truncated: bool,
}

/// A parser for one command (possibly several vendor families with the same format).
pub struct ParserDef {
    pub id: &'static str,
    /// Vendor families (`cisco`, `ruijie` …); the generic / unknown device uses all.
    pub families: &'static [&'static str],
    /// Canonical command words.
    pub command: &'static str,
    /// Other spellings of the same command (`show mac-address-table`).
    pub aliases: &'static [&'static str],
    pub columns: &'static [(&'static str, ColumnKind)],
    /// Rows from the output (without the command line and prompt). Never panics.
    pub parse: fn(&str) -> Vec<Vec<String>>,
}

impl ParserDef {
    pub fn matches(&self, typed: &str) -> bool {
        command_matches(typed, self.command)
            || self.aliases.iter().any(|a| command_matches(typed, a))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserInfo {
    pub id: String,
    pub command: String,
    pub aliases: Vec<String>,
    pub families: Vec<String>,
}

pub fn list() -> Vec<ParserInfo> {
    parsers::ALL
        .iter()
        .map(|p| ParserInfo {
            id: p.id.into(),
            command: p.command.into(),
            aliases: p.aliases.iter().map(|a| (*a).into()).collect(),
            families: p.families.iter().map(|f| (*f).into()).collect(),
        })
        .collect()
}

/// `typed` is an abbreviation of `canonical`: same number of words, each a prefix
/// (`sh ip int br` → `show ip interface brief`). Pipes (`| include …`) do not match.
pub fn command_matches(typed: &str, canonical: &str) -> bool {
    let t: Vec<&str> = typed.split_whitespace().collect();
    let c: Vec<&str> = canonical.split_whitespace().collect();
    !t.is_empty()
        && t.len() == c.len()
        && t.iter().zip(&c).all(|(a, b)| {
            !a.is_empty() && b.to_ascii_lowercase().starts_with(&a.to_ascii_lowercase())
        })
}

fn family_ok(p: &ParserDef, family: Option<&str>) -> bool {
    match family {
        None | Some("generic") => true,
        Some(f) => p.families.contains(&f),
    }
}

/// Splits an echoed `sw1#show ip int brief` / `<HW>display …` / `user@host> show …` line.
fn command_of_line(line: &str) -> Option<&str> {
    let line = line.trim_end();
    let at = line.find(['#', '>', ']', '$'])?;
    let rest = line[at + 1..].trim();
    (!rest.is_empty() && rest.len() <= 200).then_some(rest)
}

/// Finds the parser for `text` and parses it. `command` = what was typed, if known;
/// otherwise the first non-empty line is taken as the echoed `prompt#command`.
pub fn analyze(text: &str, family: Option<&str>, command: Option<&str>) -> Option<Table> {
    let mut truncated = false;
    let mut text = text;
    if text.len() > MAX_INPUT {
        let mut end = MAX_INPUT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = &text[..end];
        truncated = true;
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = text.lines();
    let mut body_start = 0usize;
    let typed = match command {
        Some(c) => Some(c.trim().to_owned()),
        None => {
            // First non-empty line: `prompt#command`.
            let mut found = None;
            for l in lines.by_ref() {
                body_start += l.len() + 1;
                if l.trim().is_empty() {
                    continue;
                }
                found = command_of_line(l).map(str::to_owned);
                break;
            }
            found
        }
    };
    let typed = typed?;
    let typed = typed.split('|').next().unwrap_or("").trim();
    let def = parsers::ALL
        .iter()
        .find(|p| family_ok(p, family) && p.matches(typed))?;
    let body = if command.is_some() {
        // The text may still start with the echo line: drop it when it is one.
        match text.lines().find(|l| !l.trim().is_empty()) {
            Some(first) if command_of_line(first).is_some_and(|c| def.matches(c)) => {
                let at = text.find(first).unwrap_or(0) + first.len();
                &text[at..]
            }
            _ => &text[..],
        }
    } else {
        &text[body_start.min(text.len())..]
    };
    let mut rows = (def.parse)(body);
    if rows.len() > MAX_ROWS {
        rows.truncate(MAX_ROWS);
        truncated = true;
    }
    let width = def.columns.len();
    for r in &mut rows {
        r.resize(width, String::new());
    }
    Some(Table {
        parser: def.id.into(),
        command: def.command.into(),
        columns: def
            .columns
            .iter()
            .map(|(k, kind)| Column {
                key: (*k).into(),
                kind: *kind,
            })
            .collect(),
        rows,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviations_match_whole_words_only() {
        assert!(command_matches("sh ip int br", "show ip interface brief"));
        assert!(command_matches(
            "SHOW IP INTERFACE BRIEF",
            "show ip interface brief"
        ));
        assert!(!command_matches("sh ip int", "show ip interface brief"));
        assert!(!command_matches(
            "sh ip int br x",
            "show ip interface brief"
        ));
        assert!(!command_matches(
            "sh ipv6 int br",
            "show ip interface brief"
        ));
        assert!(!command_matches("", "show version"));
    }

    #[test]
    fn detects_from_the_echo_line_or_the_given_command() {
        let out = "\nsw1#sh ip int br\nInterface  IP-Address  OK? Method Status  Protocol\n\
                   Vlan1  10.0.0.1  YES NVRAM  up  up\nsw1#";
        let t = analyze(out, Some("cisco"), None).unwrap();
        assert_eq!(t.parser, "cisco.show_ip_interface_brief");
        assert_eq!(t.rows.len(), 1);
        let body = "Interface  IP-Address  OK? Method Status  Protocol\nVlan1  10.0.0.1  YES NVRAM  up  up\n";
        assert_eq!(
            analyze(body, None, Some("show ip interface brief"))
                .unwrap()
                .rows
                .len(),
            1
        );
        // Pipes are ignored for the match, unknown commands give nothing.
        assert!(analyze("sw1#show ip int br | ex unass\n", Some("cisco"), None).is_some());
        assert!(analyze("sw1#show clock\n*10:00\n", Some("cisco"), None).is_none());
        assert!(analyze("no prompt here", None, None).is_none());
        // Wrong vendor family.
        assert!(analyze(out, Some("huawei"), None).is_none());
    }

    #[test]
    fn the_device_family_picks_the_parser() {
        let ver = "sw#show version\nSystem serial number    : G1\n";
        assert_eq!(
            analyze(ver, Some("ruijie"), None).unwrap().parser,
            "ruijie.show_version"
        );
        assert_eq!(
            analyze(ver, Some("cisco"), None).unwrap().parser,
            "cisco.show_version"
        );
        let mac = "sw#sh mac-address-table\n";
        assert_eq!(
            analyze(mac, Some("ruijie"), None).unwrap().parser,
            "ruijie.show_mac_address_table"
        );
        assert_eq!(
            analyze(mac, Some("cisco"), None).unwrap().parser,
            "cisco.show_mac_address_table"
        );
        assert!(analyze("sw#show cpu\n", Some("cisco"), None).is_none());
        assert_eq!(list().len(), parsers::ALL.len());
    }

    #[test]
    fn huge_input_is_capped_not_rejected() {
        let mut s = String::from(
            "sw1#show ip interface brief\nInterface IP-Address OK? Method Status Protocol\n",
        );
        let row = "Gi1/0/1 unassigned YES unset up up\n";
        while s.len() < MAX_INPUT + 1000 {
            s.push_str(row);
        }
        let t = analyze(&s, None, None).unwrap();
        assert!(t.truncated);
        assert_eq!(t.rows.len(), MAX_ROWS);
    }
}
