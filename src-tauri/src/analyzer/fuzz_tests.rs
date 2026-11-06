//! Malformed and hostile input for every parser (P4-T05). Device output is untrusted:
//! no parser may panic, loop or blow up on it. A small deterministic mutation fuzzer (no
//! extra dependency) runs every parser over mutated copies of every fixture, plus a set
//! of pathological inputs, also through `analyze` (input/row caps).

use super::{MAX_ROWS, analyze, parsers::ALL};

const FIXTURES: &[&str] = &[
    include_str!("fixtures/cisco_show_cdp_neighbors.txt"),
    include_str!("fixtures/cisco_show_cdp_neighbors_detail.txt"),
    include_str!("fixtures/cisco_show_etherchannel_summary.txt"),
    include_str!("fixtures/cisco_show_interfaces_status.txt"),
    include_str!("fixtures/cisco_show_inventory.txt"),
    include_str!("fixtures/cisco_show_ip_interface_brief.txt"),
    include_str!("fixtures/cisco_show_ip_route.txt"),
    include_str!("fixtures/cisco_show_lldp_neighbors.txt"),
    include_str!("fixtures/cisco_show_lldp_neighbors_detail.txt"),
    include_str!("fixtures/cisco_show_mac_address_table.txt"),
    include_str!("fixtures/cisco_show_spanning_tree.txt"),
    include_str!("fixtures/cisco_show_version.txt"),
    include_str!("fixtures/cisco_show_vlan_brief.txt"),
    include_str!("fixtures/junos_show_chassis_hardware.txt"),
    include_str!("fixtures/junos_show_interfaces_terse.txt"),
    include_str!("fixtures/junos_show_lldp_neighbors.txt"),
    include_str!("fixtures/junos_show_version.txt"),
    include_str!("fixtures/nxos_show_mac_address_table.txt"),
    include_str!("fixtures/ruijie_show_aggregateport_summary.txt"),
    include_str!("fixtures/ruijie_show_cpu.txt"),
    include_str!("fixtures/ruijie_show_interfaces_status.txt"),
    include_str!("fixtures/ruijie_show_ip_interface_brief.txt"),
    include_str!("fixtures/ruijie_show_ip_route.txt"),
    include_str!("fixtures/ruijie_show_lldp_neighbors.txt"),
    include_str!("fixtures/ruijie_show_lldp_neighbors_detail.txt"),
    include_str!("fixtures/ruijie_show_mac_address_table.txt"),
    include_str!("fixtures/ruijie_show_memory.txt"),
    include_str!("fixtures/ruijie_show_poe_interfaces_status.txt"),
    include_str!("fixtures/ruijie_show_spanning_tree_summary.txt"),
    include_str!("fixtures/ruijie_show_version.txt"),
    include_str!("fixtures/ruijie_show_vlan.txt"),
];

/// xorshift64*: deterministic, so a failure reproduces.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

const POOL: &[&str] = &[
    " ",
    "  ",
    "\n",
    "\r\n",
    "\r",
    "\t",
    ":",
    " : ",
    ",",
    "-",
    "--",
    "---- ----",
    "[",
    "]",
    "(",
    ")",
    "\"",
    "#",
    ">",
    "-->",
    "/",
    ".",
    "0",
    "9",
    "255.255.255.255/32",
    "Gi1/0/1",
    "GigabitEthernet 0/1",
    "no address",
    "up",
    "down",
    "Interface",
    "Port",
    "VLAN",
    "Device ID",
    "NAME:",
    "PID:",
    "\u{0}",
    "\u{1b}[31m",
    "\u{7f}",
    "ı",
    "İ",
    "ş",
    "日本",
    "🙂",
    "\u{200b}",
    "\u{feff}",
];

fn mutate(rng: &mut Rng, base: &str) -> String {
    let mut s: Vec<char> = base.chars().collect();
    for _ in 0..=rng.below(6) {
        match rng.below(6) {
            0 => {
                // Truncate.
                let at = rng.below(s.len() + 1);
                s.truncate(at);
            }
            1 => {
                // Insert a token.
                let at = rng.below(s.len() + 1);
                let tok: Vec<char> = POOL[rng.below(POOL.len())].chars().collect();
                s.splice(at..at, tok);
            }
            2 => {
                // Delete a range.
                if !s.is_empty() {
                    let a = rng.below(s.len());
                    let b = (a + rng.below(40)).min(s.len());
                    s.drain(a..b);
                }
            }
            3 => {
                // Duplicate a range.
                if !s.is_empty() {
                    let a = rng.below(s.len());
                    let b = (a + rng.below(200)).min(s.len());
                    let copy: Vec<char> = s[a..b].to_vec();
                    s.splice(b..b, copy);
                }
            }
            4 => {
                // Replace a character with a random code point.
                if !s.is_empty() {
                    let at = rng.below(s.len());
                    s[at] = char::from_u32((rng.next() % 0x3000) as u32).unwrap_or('?');
                }
            }
            _ => {
                // Swap two lines.
                let text: String = s.iter().collect();
                let mut lines: Vec<&str> = text.lines().collect();
                if lines.len() > 1 {
                    let (a, b) = (rng.below(lines.len()), rng.below(lines.len()));
                    lines.swap(a, b);
                }
                s = lines.join("\n").chars().collect();
            }
        }
    }
    s.into_iter().collect()
}

#[test]
fn every_parser_survives_mutated_fixtures() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for p in ALL {
        for fx in FIXTURES {
            for _ in 0..40 {
                let input = mutate(&mut rng, fx);
                let rows = (p.parse)(&input);
                // Field/value parsers may give several rows per line; never more than
                // one per input character.
                assert!(rows.len() <= input.len() + 1, "{}: runaway rows", p.id);
                for r in &rows {
                    assert!(
                        r.len() <= p.columns.len(),
                        "{}: row wider than its columns",
                        p.id
                    );
                }
            }
        }
    }
}

#[test]
fn pathological_inputs() {
    let long_line = "x".repeat(1 << 20);
    let many_lines = "\n".repeat(200_000);
    let colons = ":".repeat(100_000);
    let dashes = format!("{}\n{}\n", "-".repeat(5000), " -".repeat(5000));
    let spaces = format!(
        "Interface{}Status\n{}\n",
        " ".repeat(100_000),
        "a ".repeat(50_000)
    );
    let nul = "\u{0}".repeat(10_000);
    let deep = "[".repeat(50_000) + &"]".repeat(50_000);
    for input in [
        &long_line,
        &many_lines,
        &colons,
        &dashes,
        &spaces,
        &nul,
        &deep,
    ] {
        for p in ALL {
            let rows = (p.parse)(input);
            assert!(rows.len() <= MAX_ROWS * 4, "{}", p.id);
        }
    }
}

#[test]
fn analyze_never_panics_on_mutated_echo_lines() {
    let mut rng = Rng(42);
    let families = [
        None,
        Some("cisco"),
        Some("ruijie"),
        Some("juniper"),
        Some("generic"),
        Some("x"),
    ];
    for p in ALL {
        let echo = format!("sw1#{}\n{}", p.command, FIXTURES[rng.below(FIXTURES.len())]);
        for _ in 0..30 {
            let input = mutate(&mut rng, &echo);
            let fam = families[rng.below(families.len())];
            if let Some(t) = analyze(&input, fam, None) {
                assert!(t.rows.len() <= MAX_ROWS);
                assert!(t.rows.iter().all(|r| r.len() == t.columns.len()));
            }
            let cmd = mutate(&mut rng, p.command);
            let _ = analyze(&input, fam, Some(&cmd));
        }
    }
}
