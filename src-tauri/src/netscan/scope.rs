//! Scanner scope (P4-T12): what may be scanned. Every scan request goes through
//! [`check`]; the UI uses the same function to enable or disable "Scan".

use std::net::Ipv4Addr;

use serde::Serialize;

/// Largest scan: a /22 (1024 addresses).
pub const MAX_PREFIX_WIDTH: u8 = 22;
pub const MAX_ADDRESSES: u32 = 1 << (32 - MAX_PREFIX_WIDTH as u32);
/// Parallel probes (spec: default 64).
pub const CONCURRENCY: usize = 64;
/// Management ports tried with a plain TCP connect (discovery only).
pub const PORTS: [u16; 5] = [22, 23, 80, 443, 3389];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalNet {
    /// Interface name (`en0`, `Wi-Fi`, `Ethernet 2`).
    pub iface: String,
    /// This computer's address on it.
    pub ip: Ipv4Addr,
    pub prefix: u8,
    pub network: Ipv4Addr,
}

impl LocalNet {
    pub fn contains(&self, a: Ipv4Addr) -> bool {
        network_of(a, self.prefix) == self.network
    }
    /// `192.168.1.0/24`
    pub fn cidr(&self) -> String {
        format!("{}/{}", self.network, self.prefix)
    }
}

/// A requested range, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
}

impl Range {
    pub fn len(&self) -> u64 {
        u64::from(u32::from(self.last)) - u64::from(u32::from(self.first)) + 1
    }
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> {
        (u32::from(self.first)..=u32::from(self.last)).map(Ipv4Addr::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "reason")]
pub enum ScopeError {
    /// Not an address / CIDR / range.
    Invalid,
    /// Outside 10/8, 172.16/12, 192.168/16, 169.254/16.
    NotPrivate,
    /// Not inside a subnet this computer is directly connected to.
    NotConnected,
    /// Wider than a /22.
    TooLarge,
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix.min(32)))
    }
}

pub fn network_of(a: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(a) & mask(prefix))
}

pub fn prefix_of(netmask: Ipv4Addr) -> Option<u8> {
    let m = u32::from(netmask);
    let ones = m.leading_ones();
    // Contiguous masks only.
    (m.count_ones() == ones).then_some(ones as u8)
}

/// RFC 1918 or link-local (169.254/16).
pub fn allowed(a: Ipv4Addr) -> bool {
    a.is_private() || a.is_link_local()
}

/// `192.168.1.0/24`, `192.168.1.7`, `192.168.1.10-50`, `192.168.1.10-192.168.1.50`.
pub fn parse_range(text: &str) -> Result<Range, ScopeError> {
    let t = text.trim();
    if let Some((ip, p)) = t.split_once('/') {
        let ip: Ipv4Addr = ip.trim().parse().map_err(|_| ScopeError::Invalid)?;
        let p: u8 = p.trim().parse().map_err(|_| ScopeError::Invalid)?;
        if p > 32 {
            return Err(ScopeError::Invalid);
        }
        let first = network_of(ip, p);
        let last = Ipv4Addr::from(u32::from(first) | !mask(p));
        return Ok(Range { first, last });
    }
    if let Some((a, b)) = t.split_once('-') {
        let first: Ipv4Addr = a.trim().parse().map_err(|_| ScopeError::Invalid)?;
        let b = b.trim();
        let last: Ipv4Addr = if let Ok(full) = b.parse() {
            full
        } else {
            // Last octet only: `192.168.1.10-50`.
            let n: u8 = b.parse().map_err(|_| ScopeError::Invalid)?;
            let o = first.octets();
            Ipv4Addr::new(o[0], o[1], o[2], n)
        };
        if u32::from(last) < u32::from(first) {
            return Err(ScopeError::Invalid);
        }
        return Ok(Range { first, last });
    }
    let ip: Ipv4Addr = t.parse().map_err(|_| ScopeError::Invalid)?;
    Ok(Range {
        first: ip,
        last: ip,
    })
}

/// The addresses to scan for `text`, or why it may not be scanned. A whole subnet leaves
/// out its network and broadcast addresses.
pub fn check(text: &str, locals: &[LocalNet]) -> Result<(Range, LocalNet), ScopeError> {
    let mut r = parse_range(text)?;
    if !allowed(r.first) || !allowed(r.last) {
        return Err(ScopeError::NotPrivate);
    }
    let net = locals
        .iter()
        .find(|n| n.contains(r.first) && n.contains(r.last) && allowed(n.ip))
        .ok_or(ScopeError::NotConnected)?;
    // Network and broadcast addresses of the connected subnet are never probed.
    if net.prefix <= 30 {
        let bcast = u32::from(net.network) | !mask(net.prefix);
        let lo = u32::from(r.first).max(u32::from(net.network) + 1);
        let hi = u32::from(r.last).min(bcast - 1);
        if lo > hi {
            return Err(ScopeError::Invalid);
        }
        r = Range {
            first: Ipv4Addr::from(lo),
            last: Ipv4Addr::from(hi),
        };
    }
    if r.len() > u64::from(MAX_ADDRESSES) {
        return Err(ScopeError::TooLarge);
    }
    Ok((r, net.clone()))
}

/// The default target for a connected subnet: the subnet itself up to a /24, otherwise the
/// /24 that holds this computer's address.
pub fn default_target(net: &LocalNet) -> String {
    if net.prefix >= 24 {
        net.cidr()
    } else {
        format!("{}/24", network_of(net.ip, 24))
    }
}

/// This computer's private / link-local IPv4 subnets (loopback and /31–/32 left out).
pub fn local_networks() -> Vec<LocalNet> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut out: Vec<LocalNet> = ifaces
        .into_iter()
        .filter_map(|i| {
            let if_addrs::IfAddr::V4(v4) = &i.addr else {
                return None;
            };
            let prefix = prefix_of(v4.netmask)?;
            (!v4.ip.is_loopback() && allowed(v4.ip) && prefix <= 30).then(|| LocalNet {
                iface: i.name.clone(),
                ip: v4.ip,
                prefix,
                network: network_of(v4.ip, prefix),
            })
        })
        .collect();
    out.sort_by(|a, b| (a.ip.is_link_local(), &a.iface).cmp(&(b.ip.is_link_local(), &b.iface)));
    out.dedup_by(|a, b| a.network == b.network && a.prefix == b.prefix);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(iface: &str, ip: [u8; 4], prefix: u8) -> LocalNet {
        let ip = Ipv4Addr::from(ip);
        LocalNet {
            iface: iface.into(),
            ip,
            prefix,
            network: network_of(ip, prefix),
        }
    }

    fn locals() -> Vec<LocalNet> {
        vec![
            net("Wi-Fi", [192, 168, 1, 34], 24),
            net("Ethernet", [10, 20, 0, 5], 16),
            net("Lab", [172, 16, 4, 9], 22),
            net("Link", [169, 254, 7, 7], 16),
        ]
    }

    #[test]
    fn only_private_and_link_local() {
        let l = locals();
        for public in [
            "8.8.8.0/24",
            "1.1.1.1",
            "100.64.0.0/24",
            "172.32.0.0/24",
            "192.169.1.0/24",
            "224.0.0.0/24",
            "127.0.0.0/24",
        ] {
            assert_eq!(check(public, &l), Err(ScopeError::NotPrivate), "{public}");
        }
        assert!(allowed(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(allowed(Ipv4Addr::new(172, 31, 255, 255)));
        assert!(allowed(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(!allowed(Ipv4Addr::new(172, 32, 0, 1)));
    }

    #[test]
    fn only_directly_connected_subnets() {
        let l = locals();
        // Private but not ours.
        for other in [
            "192.168.2.0/24",
            "10.21.0.0/24",
            "172.16.8.0/24",
            "10.0.0.0/8",
        ] {
            assert!(
                matches!(
                    check(other, &l),
                    Err(ScopeError::NotConnected | ScopeError::TooLarge)
                ),
                "{other}"
            );
        }
        assert_eq!(check("192.168.2.0/24", &l), Err(ScopeError::NotConnected));
        // Straddling two subnets.
        assert_eq!(
            check("192.168.1.250-192.168.2.5", &l),
            Err(ScopeError::NotConnected)
        );
        // Nothing connected: nothing can be scanned.
        assert_eq!(check("192.168.1.0/24", &[]), Err(ScopeError::NotConnected));
    }

    #[test]
    fn at_most_a_slash_22() {
        let l = locals();
        let (r, n) = check("172.16.4.0/22", &l).unwrap();
        assert_eq!(n.iface, "Lab");
        assert_eq!(r.len(), 1022, "network and broadcast left out");
        assert_eq!(check("10.20.0.0/21", &l), Err(ScopeError::TooLarge));
        assert_eq!(check("10.20.0.0/16", &l), Err(ScopeError::TooLarge));
        assert_eq!(
            check("10.20.0.1-10.20.4.1", &l),
            Err(ScopeError::TooLarge),
            "1025 addresses"
        );
        assert!(check("10.20.0.1-10.20.4.0", &l).is_ok(), "exactly 1024");
        assert!(check("10.20.0.0/22", &l).is_ok());
        assert_eq!(MAX_ADDRESSES, 1 << (32 - u32::from(MAX_PREFIX_WIDTH)));
        assert_eq!(CONCURRENCY, 64);
        assert_eq!(PORTS, [22, 23, 80, 443, 3389]);
    }

    #[test]
    fn ranges_and_defaults() {
        let l = locals();
        let (r, _) = check("192.168.1.0/24", &l).unwrap();
        assert_eq!(
            (r.first, r.last, r.len()),
            (
                Ipv4Addr::new(192, 168, 1, 1),
                Ipv4Addr::new(192, 168, 1, 254),
                254
            )
        );
        let (r, _) = check("192.168.1.10-50", &l).unwrap();
        assert_eq!(r.len(), 41);
        let (r, _) = check("192.168.1.7", &l).unwrap();
        assert_eq!(
            r.hosts().collect::<Vec<_>>(),
            [Ipv4Addr::new(192, 168, 1, 7)]
        );
        assert_eq!(
            check("192.168.1.0", &l),
            Err(ScopeError::Invalid),
            "only the network address"
        );
        for bad in [
            "",
            "abc",
            "192.168.1.0/33",
            "192.168.1.50-10",
            "192.168.1.1-999",
            "300.1.1.1",
        ] {
            assert_eq!(check(bad, &l), Err(ScopeError::Invalid), "{bad}");
        }
        assert_eq!(default_target(&l[0]), "192.168.1.0/24");
        assert_eq!(default_target(&l[1]), "10.20.0.0/24");
        assert_eq!(prefix_of(Ipv4Addr::new(255, 255, 252, 0)), Some(22));
        assert_eq!(prefix_of(Ipv4Addr::new(255, 0, 255, 0)), None);
    }

    #[test]
    fn this_machine_lists_only_allowed_networks() {
        for n in local_networks() {
            assert!(allowed(n.ip) && n.prefix <= 30, "{n:?}");
        }
    }
}
