//! `show version` — Cisco IOS/IOS-XE/NX-OS, as a field / value table (identity facts come
//! from the vendor profile, P3-T04; this adds image, reload reason, memory …).

use crate::analyzer::{ColumnKind, ParserDef};
use crate::device::modes;

pub static PARSER: ParserDef = ParserDef {
    id: "cisco.show_version",
    families: &["cisco"],
    command: "show version",
    aliases: &[],
    columns: &[("field", ColumnKind::Text), ("value", ColumnKind::Text)],
    parse,
};

fn after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let t = l.trim();
        t.get(..prefix.len())
            .filter(|p| p.eq_ignore_ascii_case(prefix))
            .map(|_| t[prefix.len()..].trim().trim_matches('"'))
    })
}

fn parse(text: &str) -> Vec<Vec<String>> {
    let profile = modes::profile("cisco-ios").or_else(|| Some(modes::default_profile()));
    let facts = profile.map(|p| p.parse_facts(text)).unwrap_or_default();
    let mut rows = Vec::new();
    let mut push = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            rows.push(vec![k.to_owned(), v]);
        }
    };
    push("hostname", facts.hostname.clone());
    push("model", facts.model.clone());
    push("version", facts.version.clone());
    push("serial", facts.serial.clone());
    push("uptime", facts.uptime.clone());
    push(
        "image",
        after(text, "System image file is").map(str::to_owned),
    );
    push(
        "reloadReason",
        after(text, "Last reload reason:").map(str::to_owned),
    );
    push(
        "configRegister",
        after(text, "Configuration register is").map(str::to_owned),
    );
    push(
        "memory",
        text.lines()
            .find(|l| l.contains("bytes of memory"))
            .map(|l| l.trim().to_owned()),
    );
    push(
        "mac",
        after(text, "Base Ethernet MAC Address")
            .map(|v| v.trim_start_matches(':').trim().to_owned()),
    );
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let rows = parse(include_str!("../fixtures/cisco_show_version.txt"));
        let get = |k: &str| rows.iter().find(|r| r[0] == k).map(|r| r[1].as_str());
        assert_eq!(get("hostname"), Some("core-sw1"));
        assert_eq!(get("serial"), Some("FOC2345X1YZ"));
        assert_eq!(get("image"), Some("flash:packages.conf"));
        assert_eq!(get("reloadReason"), Some("Reload Command"));
        assert_eq!(get("configRegister"), Some("0x102"));
        assert_eq!(get("mac"), Some("00:11:22:33:44:00"));
        assert_eq!(get("version"), Some("17.09.04a"));
        assert_eq!(get("model"), Some("C9300-48P"));
    }

    #[test]
    fn garbage() {
        assert!(parse("").is_empty());
        let _ = parse("System image file is\nLast reload reason:");
    }
}
