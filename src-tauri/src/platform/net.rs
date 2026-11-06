//! Network helpers for the scanner (P4-T13): reverse DNS, the OS ARP table, the default
//! gateway and an unprivileged ICMP echo sweep. OS calls live here (ADR-018); the output
//! parsers are pure and tested.

// Platform API exception (project rules file): reverse DNS has no safe std API, so `getnameinfo`
// (Unix) / `GetNameInfoW` (Windows) are called through FFI. Each unsafe block states why
// it is sound; nothing else in this file is unsafe.
#![allow(unsafe_code)]

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

/// Runs a system tool and returns its stdout (no console window on Windows).
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up from a GUI app.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ------------------------------------------------------------------------- ARP table

/// `aa:bb:cc:dd:ee:ff` from `aa-bb-…`, `0:1a:2b:3:4:5` (macOS drops leading zeros).
pub fn normalize_mac(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (b, p) in bytes.iter_mut().zip(&parts) {
        if p.is_empty() || p.len() > 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    // Broadcast, all-zero (incomplete on Linux) and multicast entries are not hosts.
    if bytes == [0xff; 6] || bytes == [0; 6] || bytes[0] & 1 == 1 {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// IP → MAC pairs from `arp -an` (macOS), `/proc/net/arp` (Linux) or `arp -a` (Windows):
/// every line with an IPv4 address and a MAC; incomplete entries are skipped.
pub fn parse_arp(text: &str) -> Vec<(Ipv4Addr, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut ip = None;
        let mut mac = None;
        for w in line.split_whitespace() {
            let w = w.trim_matches(|c| c == '(' || c == ')');
            if ip.is_none()
                && let Ok(a) = w.parse::<Ipv4Addr>()
            {
                ip = Some(a);
            } else if mac.is_none() {
                mac = normalize_mac(w);
            }
        }
        if let (Some(ip), Some(mac)) = (ip, mac) {
            out.push((ip, mac));
        }
    }
    out
}

/// The OS ARP / neighbour table (blocking).
pub fn arp_table() -> Vec<(Ipv4Addr, String)> {
    #[cfg(target_os = "linux")]
    if let Ok(t) = std::fs::read_to_string("/proc/net/arp") {
        return parse_arp(&t);
    }
    #[cfg(windows)]
    let text = run("arp", &["-a"]);
    #[cfg(not(windows))]
    let text = run("arp", &["-an"]);
    text.map(|t| parse_arp(&t)).unwrap_or_default()
}

// ------------------------------------------------------------------- default gateway

/// The IPv4 default gateway from `route -n get default` (macOS), `ip route show default`
/// (Linux) or `route print -4 0.0.0.0` (Windows).
pub fn parse_gateway(text: &str) -> Option<Ipv4Addr> {
    for line in text.lines() {
        let t = line.trim();
        // macOS: `gateway: 192.168.1.1`
        if let Some(g) = t.strip_prefix("gateway:") {
            return g.trim().parse().ok();
        }
        let w: Vec<&str> = t.split_whitespace().collect();
        // Linux: `default via 192.168.1.1 dev wlan0 …`
        if w.first() == Some(&"default") && w.get(1) == Some(&"via") {
            return w.get(2)?.parse().ok();
        }
        // Windows: `0.0.0.0          0.0.0.0      192.168.1.1    192.168.1.34     35`
        if w.len() >= 3
            && w[0] == "0.0.0.0"
            && w[1] == "0.0.0.0"
            && let Ok(g) = w[2].parse()
        {
            return Some(g);
        }
    }
    None
}

pub fn default_gateway() -> Option<Ipv4Addr> {
    #[cfg(target_os = "macos")]
    let text = run("route", &["-n", "get", "default"]);
    #[cfg(windows)]
    let text = run("route", &["print", "-4", "0.0.0.0"]);
    #[cfg(not(any(target_os = "macos", windows)))]
    let text = run("ip", &["route", "show", "default"]);
    text.and_then(|t| parse_gateway(&t))
}

// ----------------------------------------------------------------------- reverse DNS

/// Host name of `ip` from the OS resolver (blocking; may take seconds when no server
/// answers — call with a time limit).
#[cfg(unix)]
pub fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    use std::ffi::CStr;
    use std::mem::{size_of, zeroed};
    // SAFETY: sockaddr_in is plain old data; all-zero is a valid value we then fill in.
    let mut sa: libc::sockaddr_in = unsafe { zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_addr = libc::in_addr {
        s_addr: u32::from_ne_bytes(ip.octets()),
    };
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        sa.sin_len = size_of::<libc::sockaddr_in>() as u8;
    }
    let mut host = [0 as libc::c_char; 1025];
    // SAFETY: getnameinfo is the only portable reverse lookup. `sa` is a valid
    // sockaddr_in of the given length, `host` is a writable buffer of the given length,
    // no service buffer is requested; the result is NUL-terminated on success.
    let rc = unsafe {
        libc::getnameinfo(
            (&raw const sa).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
            host.as_mut_ptr(),
            host.len() as libc::socklen_t,
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        )
    };
    if rc != 0 {
        return None;
    }
    // SAFETY: on success `host` holds a NUL-terminated string inside the buffer.
    let name = unsafe { CStr::from_ptr(host.as_ptr()) }
        .to_string_lossy()
        .trim_end_matches('.')
        .to_owned();
    (!name.is_empty() && name != ip.to_string()).then_some(name)
}

#[cfg(windows)]
pub fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, GetNameInfoW, NI_NAMEREQD, SOCKADDR, SOCKADDR_IN,
    };
    // Winsock is initialised by the standard library on first socket use; the scanner's
    // TCP probes run before any reverse lookup.
    let _ = std::net::UdpSocket::bind("0.0.0.0:0");
    // SAFETY: SOCKADDR_IN is plain old data; all-zero is valid and then filled in.
    let mut sa: SOCKADDR_IN = unsafe { zeroed() };
    sa.sin_family = AF_INET;
    sa.sin_addr.S_un.S_addr = u32::from_ne_bytes(ip.octets());
    let mut host = [0u16; 1025];
    // SAFETY: GetNameInfoW is the system reverse lookup. `sa` is a valid SOCKADDR_IN of
    // the given size, `host` a writable UTF-16 buffer of the given length; no service
    // buffer. On success the name is NUL-terminated inside `host`.
    let rc = unsafe {
        GetNameInfoW(
            (&raw const sa).cast::<SOCKADDR>(),
            size_of::<SOCKADDR_IN>() as i32,
            host.as_mut_ptr(),
            host.len() as u32,
            std::ptr::null_mut(),
            0,
            NI_NAMEREQD as i32,
        )
    };
    if rc != 0 {
        return None;
    }
    let len = host.iter().position(|&c| c == 0).unwrap_or(host.len());
    let name = String::from_utf16_lossy(&host[..len])
        .trim_end_matches('.')
        .to_owned();
    (!name.is_empty() && name != ip.to_string()).then_some(name)
}

// ------------------------------------------------------------------------- ICMP sweep

fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for pair in data.chunks(2) {
        let word = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// An ICMP echo request (type 8) with a checksum.
pub fn echo_request(ident: u16, seq: u16) -> Vec<u8> {
    let mut p = vec![8, 0, 0, 0];
    p.extend_from_slice(&ident.to_be_bytes());
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(b"kenshell-scan");
    let c = checksum(&p);
    p[2..4].copy_from_slice(&c.to_be_bytes());
    p
}

/// True for an echo reply; datagram ICMP sockets on macOS hand back the IP header too.
pub fn is_echo_reply(packet: &[u8]) -> bool {
    let icmp = match packet.first() {
        Some(b) if b >> 4 == 4 => packet.get(usize::from(b & 0x0f) * 4..).unwrap_or(&[]),
        _ => packet,
    };
    icmp.first() == Some(&0)
}

/// Sends one echo request to every address and collects who answers within `wait`
/// after the last send. None when this OS / user cannot open an unprivileged ICMP
/// socket (Windows, or Linux with a restrictive `ping_group_range`): the scanner then
/// relies on TCP and the ARP table. Blocking.
#[cfg(unix)]
pub fn icmp_sweep(
    hosts: &[Ipv4Addr],
    wait: Duration,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<std::collections::HashSet<Ipv4Addr>> {
    use socket2::{Domain, Protocol, Socket, Type};
    use std::sync::atomic::Ordering;

    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::ICMPV4)).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(20)))
        .ok()?;
    // A datagram socket: std's UdpSocket reads it safely (recv_from on the same fd).
    let sock = std::net::UdpSocket::from(sock);
    let ident = (std::process::id() & 0xffff) as u16;
    let mut alive = std::collections::HashSet::new();
    let mut buf = [0u8; 1500];
    let mut drain = |sock: &std::net::UdpSocket,
                     alive: &mut std::collections::HashSet<Ipv4Addr>| {
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            if is_echo_reply(&buf[..n])
                && let std::net::SocketAddr::V4(a) = from
                && hosts.contains(a.ip())
            {
                alive.insert(*a.ip());
            }
        }
    };
    for (i, ip) in hosts.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Some(alive);
        }
        let _ = sock.send_to(
            &echo_request(ident, i as u16),
            std::net::SocketAddrV4::new(*ip, 0),
        );
        // A short pause every few packets keeps the burst polite; answers are read
        // while sending.
        if i % 16 == 15 {
            drain(&sock, &mut alive);
        }
    }
    let end = Instant::now() + wait;
    while Instant::now() < end && !cancel.load(Ordering::Relaxed) {
        drain(&sock, &mut alive);
    }
    Some(alive)
}

#[cfg(windows)]
pub fn icmp_sweep(
    _hosts: &[Ipv4Addr],
    _wait: Duration,
    _cancel: &std::sync::atomic::AtomicBool,
) -> Option<std::collections::HashSet<Ipv4Addr>> {
    // Unprivileged ICMP needs IcmpSendEcho on Windows (Known Issues); TCP + ARP cover it.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arp_outputs_of_every_os() {
        let mac = "? (192.168.1.1) at 0:1a:2b:3:4:5 on en0 ifscope [ethernet]\n\
                   ? (192.168.1.20) at (incomplete) on en0 ifscope [ethernet]\n\
                   ? (192.168.1.255) at ff:ff:ff:ff:ff:ff on en0 ifscope [ethernet]\n\
                   ? (224.0.0.251) at 1:0:5e:0:0:fb on en0 ifscope permanent [ethernet]\n";
        assert_eq!(
            parse_arp(mac),
            [(
                Ipv4Addr::new(192, 168, 1, 1),
                "00:1a:2b:03:04:05".to_owned()
            )]
        );
        let linux = "IP address       HW type     Flags       HW address            Mask     Device\n\
                     192.168.1.1      0x1         0x2         aa:bb:cc:dd:ee:01     *        wlan0\n\
                     192.168.1.9      0x1         0x0         00:00:00:00:00:00     *        wlan0\n";
        assert_eq!(parse_arp(linux).len(), 1);
        let win = "Interface: 192.168.1.34 --- 0xb\n  Internet Address      Physical Address      Type\n  \
                   192.168.1.1           aa-bb-cc-dd-ee-01     dynamic\n  192.168.1.255         ff-ff-ff-ff-ff-ff     static\n";
        assert_eq!(
            parse_arp(win),
            [(
                Ipv4Addr::new(192, 168, 1, 1),
                "aa:bb:cc:dd:ee:01".to_owned()
            )]
        );
        assert!(parse_arp("garbage\n\n:::").is_empty());
    }

    #[test]
    fn gateway_outputs_of_every_os() {
        assert_eq!(
            parse_gateway(
                "   route to: default\ndestination: default\n    gateway: 192.168.1.1\n  interface: en0\n"
            ),
            Some(Ipv4Addr::new(192, 168, 1, 1))
        );
        assert_eq!(
            parse_gateway("default via 10.0.0.1 dev eth0 proto dhcp metric 100\n"),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(
            parse_gateway(
                "Active Routes:\nNetwork Destination        Netmask          Gateway       Interface  Metric\n          0.0.0.0          0.0.0.0      192.168.1.1   192.168.1.34     35\n"
            ),
            Some(Ipv4Addr::new(192, 168, 1, 1))
        );
        assert_eq!(parse_gateway("nothing here"), None);
    }

    #[test]
    fn icmp_packets() {
        let p = echo_request(0x1234, 7);
        assert_eq!(&p[..2], &[8, 0]);
        assert_eq!(checksum(&p), 0, "a valid checksum sums to zero");
        let mut reply = p.clone();
        reply[0] = 0;
        assert!(is_echo_reply(&reply));
        let mut with_ip = vec![0x45u8; 1];
        with_ip.extend_from_slice(&[0u8; 19]);
        with_ip.extend_from_slice(&reply);
        assert!(is_echo_reply(&with_ip));
        assert!(!is_echo_reply(&p));
        assert!(!is_echo_reply(&[]));
    }

    /// Real network, this machine only: pings the default gateway once and reads the ARP
    /// table. `cargo test -p kenshell local_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn local_smoke() {
        let gw = default_gateway();
        println!("gateway: {gw:?}");
        if let Some(gw) = gw {
            let cancel = std::sync::atomic::AtomicBool::new(false);
            println!(
                "icmp: {:?}",
                icmp_sweep(&[gw], Duration::from_millis(800), &cancel)
            );
            println!("rdns: {:?}", reverse_dns(gw));
        }
        println!("arp entries: {}", arp_table().len());
    }

    #[test]
    fn reverse_dns_of_unroutable_address_is_none_or_a_name() {
        // Must not panic or hang for long on an address nobody resolves.
        let _ = reverse_dns(Ipv4Addr::new(192, 0, 2, 1));
    }
}
