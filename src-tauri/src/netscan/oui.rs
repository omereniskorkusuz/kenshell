//! MAC vendor lookup (P4-T15) from the embedded IEEE MA-L table `data/oui.bin`
//! (built by `tools/scripts/gen-oui.mjs`): decompressed once on first use, then binary
//! searched. A damaged table gives no vendors instead of failing.

use std::io::Read;
use std::sync::LazyLock;

static RAW: &[u8] = include_bytes!("../../data/oui.bin");
/// Decompression limit (the real table is ~1–2 MB plain).
const MAX_PLAIN: u64 = 8 * 1024 * 1024;

struct Table {
    prefixes: Vec<u32>,
    name_of: Vec<u16>,
    names: Vec<String>,
}

fn parse(plain: &[u8]) -> Option<Table> {
    let mut o = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = plain.get(o..o + n)?;
        o += n;
        Some(s)
    };
    if take(4)? != b"OUI1" {
        return None;
    }
    let count = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let p = take(count.checked_mul(3)?)?;
    let prefixes: Vec<u32> = p
        .chunks(3)
        .map(|c| u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]))
        .collect();
    let idx = take(count.checked_mul(2)?)?;
    let name_of: Vec<u16> = idx
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let n = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
    let mut names = Vec::with_capacity(n.min(70_000));
    for _ in 0..n {
        let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
        names.push(String::from_utf8_lossy(take(len)?).into_owned());
    }
    let sorted = prefixes.windows(2).all(|w| w[0] < w[1]);
    let valid = name_of.iter().all(|&i| usize::from(i) < names.len());
    (sorted && valid).then_some(Table {
        prefixes,
        name_of,
        names,
    })
}

static TABLE: LazyLock<Option<Table>> = LazyLock::new(|| {
    let mut plain = Vec::new();
    flate2::read::DeflateDecoder::new(RAW)
        .take(MAX_PLAIN)
        .read_to_end(&mut plain)
        .ok()?;
    parse(&plain)
});

/// Vendor of a MAC (`aa:bb:cc:…`, `aa-bb-…`, `aabb.cc…`), or None.
pub fn vendor(mac: &str) -> Option<&'static str> {
    let hex: String = mac
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(6)
        .collect();
    if hex.len() < 6 {
        return None;
    }
    let prefix = u32::from_str_radix(&hex, 16).ok()?;
    let t = TABLE.as_ref()?;
    let i = t.prefixes.binary_search(&prefix).ok()?;
    t.names.get(usize::from(t.name_of[i])).map(String::as_str)
}

/// Number of prefixes in the embedded table (0 when missing or damaged).
pub fn size() -> usize {
    TABLE.as_ref().map_or(0, |t| t.prefixes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_loads_and_looks_up() {
        assert!(size() > 0, "oui.bin present and valid");
        // In the stub and in the full IEEE registry alike.
        assert_eq!(vendor("00:00:0c:12:34:56"), Some("Cisco Systems"));
        assert_eq!(vendor("0000.0c12.3456"), Some("Cisco Systems"));
        assert_eq!(vendor("58-69-6C-00-11-11"), Some("Ruijie Networks"));
        assert_eq!(vendor("02:00:00:00:00:01"), None);
        assert_eq!(vendor("zz"), None);
    }

    #[test]
    fn damaged_tables_are_rejected() {
        assert!(parse(b"").is_none());
        assert!(parse(b"OUI1\xff\xff\xff\xff").is_none());
        // Unsorted prefixes.
        let mut t = b"OUI1".to_vec();
        t.extend_from_slice(&2u32.to_le_bytes());
        t.extend_from_slice(&[0, 0, 2, 0, 0, 1]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&1u16.to_le_bytes());
        t.push(b'x');
        assert!(parse(&t).is_none());
    }
}
