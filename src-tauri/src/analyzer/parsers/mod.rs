//! Parser registry. Add a parser: one module here + its fixtures in `../fixtures/`, then
//! list it in [`ALL`] (more specific commands before shorter ones with the same prefix).

use super::ParserDef;

mod cdp;
mod etherchannel;
mod interfaces_status;
mod inventory;
mod ip_interface_brief;
mod ip_route;
mod juniper;
mod lldp;
mod mac_address_table;
mod ruijie;
mod spanning_tree;
mod version;
mod vlan_brief;

pub static ALL: &[&ParserDef] = &[
    &ip_interface_brief::PARSER,
    &interfaces_status::PARSER,
    &vlan_brief::PARSER,
    &cdp::BRIEF,
    &cdp::DETAIL,
    &lldp::BRIEF,
    &lldp::DETAIL,
    &version::PARSER,
    &inventory::PARSER,
    &mac_address_table::PARSER,
    &ip_route::PARSER,
    &etherchannel::PARSER,
    &spanning_tree::PARSER,
    // Ruijie (P4-T03).
    &ruijie::version::PARSER,
    &ruijie::interfaces_status::PARSER,
    &ruijie::reuse::VLAN,
    &ruijie::lldp::BRIEF,
    &ruijie::lldp::DETAIL,
    &ruijie::mac_address_table::PARSER,
    &ruijie::reuse::IP_ROUTE,
    &ruijie::aggregate_port::PARSER,
    &ruijie::reuse::STP_SUMMARY,
    &ruijie::poe::PARSER,
    &ruijie::cpu::PARSER,
    &ruijie::memory::PARSER,
    // Juniper (P4-T04).
    &juniper::INTERFACES_TERSE,
    &juniper::VERSION,
    &juniper::CHASSIS_HARDWARE,
    &juniper::LLDP_NEIGHBORS,
];

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

/// Start offsets (in characters) of the header's column titles, searched left to right
/// (case-insensitive). None when a title is missing.
pub(crate) fn header_offsets(header: &str, titles: &[&str]) -> Option<Vec<usize>> {
    let hay: Vec<char> = header.to_lowercase().chars().collect();
    let mut out = Vec::with_capacity(titles.len());
    let mut from = 0usize;
    for t in titles {
        let needle: Vec<char> = t.to_lowercase().chars().collect();
        let at = (from..=hay.len().checked_sub(needle.len())?)
            .find(|&i| hay[i..i + needle.len()] == needle[..])?;
        out.push(at);
        from = at + needle.len();
    }
    Some(out)
}

/// Cuts `line` at the character offsets (column `i` = `offsets[i]..offsets[i+1]`), trimmed.
pub(crate) fn cut(line: &str, offsets: &[usize]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    (0..offsets.len())
        .map(|i| {
            let a = offsets[i].min(chars.len());
            let b = offsets
                .get(i + 1)
                .copied()
                .unwrap_or(chars.len())
                .min(chars.len());
            if a >= b {
                String::new()
            } else {
                chars[a..b].iter().collect::<String>().trim().to_owned()
            }
        })
        .collect()
}

/// The value of a `Key: value` / `Key : value` line (first colon), when the key matches.
pub(crate) fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.trim_start().split_once(':')?;
    k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_cut_and_fields() {
        let h = "Port      Name               Status       Vlan";
        let o = header_offsets(h, &["port", "name", "status", "vlan"]).unwrap();
        assert_eq!(o, [0, 10, 29, 42]);
        assert_eq!(
            cut("Gi1/0/1   Uplink to core     connected    trunk", &o),
            ["Gi1/0/1", "Uplink to core", "connected", "trunk"]
        );
        assert_eq!(cut("Gi1", &o), ["Gi1", "", "", ""]);
        assert!(header_offsets(h, &["port", "missing"]).is_none());
        assert_eq!(field("  Device ID: sw1", "device id"), Some("sw1"));
        assert_eq!(field("Version :", "version"), Some(""));
    }
}
