//! LLDP neighbour details of the other vendor families the topology map reads (R-T17):
//! the "CX"-style `show lldp neighbor-info detail` and the OmniSwitch-style
//! `show lldp remote-system`. Rows have the columns of the IOS-style detail parser:
//! device, ip, local port, remote port, port description, chassis, capability, description.
//!
//! The formats are written from the vendors' published command references; they still
//! need a capture from real hardware (see Known Issues).

use super::field;

const COLS: usize = 8;

fn nonempty(v: &str) -> Option<&str> {
    let t = v.trim();
    (!t.is_empty() && t != "(null)").then_some(t)
}

/// `show lldp neighbor-info detail`: one block per local port, `Key : value` lines.
pub fn cx_lldp_detail(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        if let Some(port) = field(line, "port") {
            rows.push(vec![String::new(); COLS]);
            if let Some(r) = rows.last_mut() {
                r[2] = port.to_owned();
            }
            continue;
        }
        let Some(r) = rows.last_mut() else { continue };
        for (key, col) in [
            ("neighbor chassis-name", 0),
            ("neighbor management-address", 1),
            ("neighbor port-id", 3),
            ("neighbor port-desc", 4),
            ("neighbor chassis-id", 5),
            ("chassis capabilities enabled", 6),
            ("neighbor chassis-description", 7),
        ] {
            if let Some(v) = field(line, key).and_then(nonempty)
                && r[col].is_empty()
            {
                r[col] = v.to_owned();
            }
        }
    }
    // A port block without any neighbour identity is not a neighbour.
    rows.retain(|r| !r[2].is_empty() && (!r[0].is_empty() || !r[5].is_empty()));
    rows
}

/// `show lldp remote-system`: `… on Local Port <p>:` then `Chassis <id>, Port <id>:` and
/// `Key = value,` lines.
pub fn aos_lldp_remote(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut local = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some((_, port)) = t.rsplit_once("Local Port ") {
            local = port.trim_end_matches(':').trim().to_owned();
            continue;
        }
        if let Some(rest) = t.strip_prefix("Chassis ")
            && let Some((chassis, port)) = rest.split_once(", Port ")
        {
            let mut r = vec![String::new(); COLS];
            r[2].clone_from(&local);
            r[3] = port.trim_end_matches(':').trim().to_owned();
            r[5] = chassis.trim().to_owned();
            rows.push(r);
            continue;
        }
        let (Some(r), Some((key, value))) = (rows.last_mut(), t.split_once('=')) else {
            continue;
        };
        let Some(value) = nonempty(value.trim().trim_end_matches(',')) else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "system name" => r[0] = value.to_owned(),
            "management ip address" => r[1] = value.to_owned(),
            "port description" => {
                r[4] = value.to_owned();
                // The port id is often a bare index; the description ends with the name.
                if let Some(last) = value.split_whitespace().last()
                    && last.contains('/')
                    && r[3].chars().all(|c| c.is_ascii_digit())
                {
                    r[3] = last.to_owned();
                }
            }
            "capabilities enabled" => r[6] = value.to_owned(),
            "system description" => r[7] = value.to_owned(),
            _ => {}
        }
    }
    rows.retain(|r| !r[2].is_empty());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cx_blocks() {
        let rows = cx_lldp_detail(include_str!(
            "../fixtures/aoscx_show_lldp_neighbor_info_detail.txt"
        ));
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(
            rows[0],
            [
                "core-sw1",
                "10.0.0.1",
                "1/1/24",
                "Gi1/0/12",
                "GigabitEthernet1/0/12",
                "00:11:22:33:44:00",
                "Bridge, Router",
                "L3 switch software, release 17.9"
            ]
        );
        assert_eq!(rows[1][6], "WLAN");
        // An end station that only sends its MAC.
        assert_eq!(
            (rows[2][0].as_str(), rows[2][5].as_str()),
            ("", "3c:52:82:00:0b:02")
        );
    }

    #[test]
    fn omniswitch_blocks() {
        let rows = aos_lldp_remote(include_str!("../fixtures/aos8_show_lldp_remote_system.txt"));
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0][0], "dist-sw1");
        assert_eq!(rows[0][1], "10.1.1.2");
        assert_eq!(rows[0][2], "1/1/23");
        assert_eq!(rows[0][3], "1/1/23", "port name from the description");
        assert_eq!(rows[0][5], "94:24:e1:aa:bb:cc");
        assert_eq!(rows[0][6], "Bridge Router");
        assert_eq!(rows[1][2], "1/1/5");
        assert_eq!(rows[1][3], "00:80:9f:11:22:33");
        assert_eq!(rows[1][4], "", "(null) is no description");
    }

    #[test]
    fn garbage_and_truncation_never_panic() {
        for text in [
            include_str!("../fixtures/aoscx_show_lldp_neighbor_info_detail.txt"),
            include_str!("../fixtures/aos8_show_lldp_remote_system.txt"),
        ] {
            for cut in (0..text.len()).filter(|i| text.is_char_boundary(*i)) {
                let _ = cx_lldp_detail(&text[..cut]);
                let _ = aos_lldp_remote(&text[..cut]);
            }
        }
        for bad in [
            "",
            "Port :",
            "Chassis , Port :",
            "= = =",
            "% Invalid input\0\u{1b}[0m",
            ":::",
            "Local Port ",
        ] {
            assert!(cx_lldp_detail(bad).iter().all(|r| r.len() == COLS));
            assert!(aos_lldp_remote(bad).iter().all(|r| r.len() == COLS));
        }
    }
}
