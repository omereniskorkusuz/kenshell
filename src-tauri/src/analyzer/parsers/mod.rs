//! Parser registry. Add a parser: one module here + its fixtures in `../fixtures/`, then
//! list it in [`ALL`] (more specific commands before shorter ones with the same prefix).

use super::ParserDef;

mod ip_interface_brief;

pub static ALL: &[&ParserDef] = &[&ip_interface_brief::PARSER];

/// Lines that are not data: empty, the trailing prompt, `--More--` leftovers.
pub(crate) fn is_noise(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.contains("--More--") || t.ends_with('#') || t.ends_with('>')
}

/// Takes the interface name from the start of a row: one word (`Gi1/0/1`, `Vlan10`) or a
/// Ruijie spaced name (`GigabitEthernet 0/1`, `VLAN 10`, `Mgmt 0`). The name must look like
/// an interface: a known type or letters followed by a number.
pub(crate) fn take_iface<'a, 'b>(words: &'b [&'a str]) -> Option<(String, &'b [&'a str])> {
    let first = *words.first()?;
    let starts_alpha = first
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic());
    if !starts_alpha {
        return None;
    }
    if let Some(second) = words.get(1)
        && !first.chars().any(|c| c.is_ascii_digit())
        && second.starts_with(|c: char| c.is_ascii_digit())
        && !second.contains('.')
    {
        return Some((format!("{first} {second}"), &words[2..]));
    }
    if !first.chars().any(|c| c.is_ascii_digit()) && crate::device::iface::parse(first).is_none() {
        return None;
    }
    Some((first.to_owned(), &words[1..]))
}
