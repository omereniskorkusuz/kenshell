//! Config backup text handling (P4-T08): turning a captured `show running-config` into the
//! config that is stored and compared.

/// Lines that change on every read without any config change (they would defeat
/// "unchanged" detection and clutter diffs).
fn volatile(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("! Last configuration change at")
        || t.starts_with("! NVRAM config last updated at")
        || t.starts_with("! No configuration change since last restart")
        || t.starts_with("!Time:")
        || t.starts_with("!Running configuration last done at")
        || t.starts_with("Building configuration")
        || t.starts_with("Current configuration :")
        || t.starts_with("Using ") && t.contains(" out of ") && t.ends_with("bytes")
}

/// The config from a captured command output: the echoed command line and volatile
/// lines removed, line ends normalised, trailing blank lines dropped.
pub fn clean_capture(captured: &str) -> String {
    let mut lines = captured.lines();
    // The first line is the echo of the command.
    let first = lines.next().unwrap_or("");
    let mut out: Vec<&str> = Vec::new();
    if !first.trim_start().to_ascii_lowercase().starts_with("sh") {
        out.push(first.trim_end());
    }
    out.extend(lines.map(str::trim_end).filter(|l| !volatile(l)));
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_and_volatile_lines_are_dropped() {
        let cap = "show running-config\nBuilding configuration...\n\nCurrent configuration : 4321 bytes\n!\n! Last configuration change at 10:00:01 UTC Thu Nov 6 2026 by admin\n! NVRAM config last updated at 09:00:00 UTC Thu Nov 6 2026\n!\nversion 17.9\nhostname sw1\nend\n\n";
        assert_eq!(
            clean_capture(cap),
            "!\n!\nversion 17.9\nhostname sw1\nend\n"
        );
        let nx = "sh run\n\n!Command: show running-config\n!Time: Thu Nov  6 10:00:01 2026\n\nversion 9.3(8)\n";
        assert_eq!(
            clean_capture(nx),
            "!Command: show running-config\n\nversion 9.3(8)\n"
        );
        assert_eq!(clean_capture(""), "\n");
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
