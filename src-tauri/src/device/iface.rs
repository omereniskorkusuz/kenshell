//! Interface name normalization (P3-T01).
//!
//! Ruijie writes `GigabitEthernet 0/24` (with a space), Cisco `GigabitEthernet0/24`, people
//! type `Gi0/24` or `gi 0/24`: all are the same interface. [`parse`] turns any of them into
//! one [`Iface`] whose `key` is used for every match (notes, tooltips, colouring).
//!
//! Types and abbreviations come from `src/data/interfaces.json` (shared with
//! `src/lib/iface.ts`); `src/data/iface-cases.json` holds test vectors for both.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

const DATA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/data/interfaces.json"
));

#[derive(Debug, Deserialize)]
struct TypeDef {
    canonical: String,
    short: String,
    names: Vec<String>,
    aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Data {
    types: Vec<TypeDef>,
}

struct Table {
    types: Vec<TypeDef>,
    /// Whole-name parser: letters, at most one space, number.
    whole: Regex,
    /// Finds interface mentions in free text (known spellings only, ≥ 2 letters).
    scan: Regex,
}

/// `0/24`, `1/0/24`, `0/1.100` (sub-interface), `0/0/0:1` (channel).
const NUMBER: &str = r"\d+(?:/\d+)*(?:\.\d+)?(?::\d+)?";

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let data: Data = serde_json::from_str(DATA).expect("src/data/interfaces.json is valid");
        let mut spellings: Vec<String> = data
            .types
            .iter()
            .flat_map(|t| t.names.iter().chain(&t.aliases))
            .filter(|s| s.len() >= 2)
            .map(|s| regex::escape(s))
            .collect();
        // Longest first so `GigabitEthernet` wins over `Gi`.
        spellings.sort_by_key(|s| std::cmp::Reverse(s.len()));
        spellings.dedup();
        Table {
            whole: Regex::new(&format!(r"^([A-Za-z][A-Za-z-]*) ?({NUMBER})$")).expect("regex"),
            scan: Regex::new(&format!(
                r"(?i)(?:^|[^A-Za-z0-9])((?:{}) ?{NUMBER})(?:$|[^\w/])",
                spellings.join("|")
            ))
            .expect("regex"),
            types: data.types,
        }
    })
}

/// A normalized interface name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Iface {
    /// Canonical type, e.g. `GigabitEthernet`.
    pub kind: String,
    /// Numbering, e.g. `1/0/24`.
    pub number: String,
    /// Lowercase match key, e.g. `gigabitethernet1/0/24` (same for every spelling).
    pub key: String,
    /// Display form, e.g. `Gi1/0/24`.
    pub short: String,
}

impl Iface {
    /// Full canonical name, e.g. `GigabitEthernet1/0/24`.
    pub fn canonical(&self) -> String {
        format!("{}{}", self.kind, self.number)
    }
}

/// The type for a typed word: exact alias, full name, or a unique prefix (≥ 2 letters)
/// of a full name — like the Cisco CLI accepts `giga`.
fn resolve(word: &str) -> Option<&'static TypeDef> {
    let t = table();
    let w = word.to_ascii_lowercase();
    if let Some(d) = t
        .types
        .iter()
        .find(|d| d.aliases.iter().any(|a| a.eq_ignore_ascii_case(&w)))
    {
        return Some(d);
    }
    if let Some(d) = t
        .types
        .iter()
        .find(|d| d.names.iter().any(|n| n.eq_ignore_ascii_case(&w)))
    {
        return Some(d);
    }
    if w.len() < 2 {
        return None;
    }
    let mut hits = t.types.iter().filter(|d| {
        d.names
            .iter()
            .any(|n| n.to_ascii_lowercase().starts_with(&w))
    });
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

/// Normalizes one interface name (surrounding whitespace ignored); `None` if it is not one.
pub fn parse(text: &str) -> Option<Iface> {
    let caps = table().whole.captures(text.trim())?;
    let def = resolve(caps.get(1)?.as_str())?;
    let number = caps.get(2)?.as_str().to_owned();
    Some(Iface {
        key: format!("{}{number}", def.canonical.to_ascii_lowercase()),
        short: format!("{}{number}", def.short),
        kind: def.canonical.clone(),
        number,
    })
}

/// Interface mentions in `text`, in order (each with its byte range).
pub fn scan(text: &str) -> Vec<(std::ops::Range<usize>, Iface)> {
    let mut out = Vec::new();
    let mut at = 0;
    // The pattern consumes one boundary character on each side, so search again from the
    // end of each match (not after the trailing boundary) to find adjacent mentions.
    while let Some(caps) = table().scan.captures_at(text, at) {
        let Some(m) = caps.get(1) else { break };
        if let Some(i) = parse(m.as_str()) {
            out.push((m.range(), i));
        }
        at = m.end();
    }
    out
}

/// True when both names are the same interface (any spelling).
pub fn same(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(x), Some(y)) if x.key == y.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/data/iface-cases.json"
    ));

    #[derive(Deserialize)]
    struct Case {
        #[serde(rename = "in")]
        input: String,
        key: Option<String>,
        short: Option<String>,
        canonical: Option<String>,
    }

    #[derive(Deserialize)]
    struct Scan {
        text: String,
        found: Vec<String>,
    }

    #[derive(Deserialize)]
    struct Cases {
        cases: Vec<Case>,
        scan: Vec<Scan>,
    }

    fn cases() -> Cases {
        serde_json::from_str(CASES).unwrap()
    }

    #[test]
    fn shared_vectors_normalize() {
        for c in cases().cases {
            let got = parse(&c.input);
            assert_eq!(got.as_ref().map(|i| i.key.clone()), c.key, "{:?}", c.input);
            if let Some(i) = got {
                assert_eq!(Some(i.short.clone()), c.short, "{:?}", c.input);
                assert_eq!(Some(i.canonical()), c.canonical, "{:?}", c.input);
            }
        }
    }

    #[test]
    fn shared_vectors_scan() {
        for s in cases().scan {
            let found: Vec<String> = scan(&s.text).into_iter().map(|(_, i)| i.key).collect();
            assert_eq!(found, s.found, "{:?}", s.text);
        }
    }

    #[test]
    fn ruijie_spaced_and_cisco_short_are_the_same() {
        assert!(same("GigabitEthernet 0/24", "Gi0/24"));
        assert!(same("AggregatePort 1", "ag1"));
        assert!(!same("Gi0/24", "Gi0/25"));
        assert!(!same("Gi0/24", "Te0/24"));
        let (range, _) = scan("x GigabitEthernet 0/24, y").remove(0);
        assert_eq!(&"x GigabitEthernet 0/24, y"[range], "GigabitEthernet 0/24");
    }
}
