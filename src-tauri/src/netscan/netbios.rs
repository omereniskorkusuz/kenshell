//! NetBIOS node status (RFC 1002, UDP 137): the computer name of a host that has no
//! reverse DNS record (typically a Windows PC on an office LAN). One query, one answer,
//! read-only. Standard library only.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

const PORT: u16 = 137;
/// NBSTAT question type and IN class.
const NBSTAT: [u8; 2] = [0x00, 0x21];
const CLASS_IN: [u8; 2] = [0x00, 0x01];

/// A node status query for the wildcard name `*`.
pub fn query(id: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(50);
    p.extend_from_slice(&id.to_be_bytes());
    // Flags 0, one question, no other records.
    p.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    p.push(32);
    let mut name = [0u8; 16];
    name[0] = b'*';
    for b in name {
        p.push(b'A' + (b >> 4));
        p.push(b'A' + (b & 0x0f));
    }
    p.push(0);
    p.extend_from_slice(&NBSTAT);
    p.extend_from_slice(&CLASS_IN);
    p
}

/// The workstation name in a node status answer: the first unique (not group) name with
/// suffix 0x00, otherwise the first name at all.
pub fn parse(id: u16, answer: &[u8]) -> Option<String> {
    if answer.get(..2)? != id.to_be_bytes() || answer.get(2)? & 0x80 == 0 {
        return None;
    }
    // Answer name: a full 32-byte encoded name, or a 2-byte pointer.
    let mut at = 12;
    match *answer.get(at)? {
        0xC0..=0xFF => at += 2,
        len => at += 1 + usize::from(len) + 1,
    }
    if answer.get(at..at + 2)? != NBSTAT {
        return None;
    }
    // type, class, ttl, rdlength
    at += 10;
    let count = usize::from(*answer.get(at)?);
    at += 1;
    let mut first = None;
    for i in 0..count {
        let entry = answer.get(at + i * 18..at + i * 18 + 18)?;
        let name = String::from_utf8_lossy(&entry[..15]).trim_end().to_owned();
        let name: String = name.chars().filter(|c| !c.is_control()).collect();
        if name.is_empty() {
            continue;
        }
        let group = entry[16] & 0x80 != 0;
        if entry[15] == 0x00 && !group {
            return Some(name);
        }
        first.get_or_insert(name);
    }
    first
}

/// Asks `ip` for its NetBIOS name. Blocking, at most `timeout`.
pub fn node_name(ip: Ipv4Addr, timeout: Duration) -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(timeout)).ok()?;
    sock.connect(SocketAddrV4::new(ip, PORT)).ok()?;
    let id = (u32::from(ip) as u16) ^ 0x4b53;
    sock.send(&query(id)).ok()?;
    let mut buf = [0u8; 1024];
    let n = sock.recv(&mut buf).ok()?;
    parse(id, &buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(id: u16, names: &[(&str, u8, bool)]) -> Vec<u8> {
        let mut a = query(id);
        a[2] = 0x84; // response, authoritative
        a[5] = 0; // no question
        a[7] = 1; // one answer
        a.truncate(a.len() - 4);
        a.extend_from_slice(&NBSTAT);
        a.extend_from_slice(&CLASS_IN);
        a.extend_from_slice(&[0, 0, 0, 0]);
        let len = (1 + names.len() * 18 + 46) as u16;
        a.extend_from_slice(&len.to_be_bytes());
        a.push(names.len() as u8);
        for (name, suffix, group) in names {
            let mut e = [b' '; 15].to_vec();
            e[..name.len()].copy_from_slice(name.as_bytes());
            e.push(*suffix);
            e.extend_from_slice(&[if *group { 0x84 } else { 0x04 }, 0]);
            a.extend_from_slice(&e);
        }
        a.extend_from_slice(&[0u8; 46]);
        a
    }

    #[test]
    fn query_is_the_wildcard_node_status() {
        let q = query(0x1234);
        assert_eq!(q.len(), 50);
        assert_eq!(&q[..2], &[0x12, 0x34]);
        assert_eq!(&q[13..15], b"CK", "'*' first-level encoded");
        assert_eq!(&q[15..45], [b'A'; 30].as_slice());
        assert_eq!(&q[46..], &[0, 0x21, 0, 1]);
    }

    #[test]
    fn workstation_name_wins() {
        let a = answer(
            7,
            &[
                ("WORKGROUP", 0x00, true),
                ("PC-MUHASEBE", 0x20, false),
                ("PC-MUHASEBE", 0x00, false),
            ],
        );
        assert_eq!(parse(7, &a).as_deref(), Some("PC-MUHASEBE"));
        let only_group = answer(7, &[("WORKGROUP", 0x00, true)]);
        assert_eq!(parse(7, &only_group).as_deref(), Some("WORKGROUP"));
    }

    #[test]
    fn wrong_or_broken_answers_are_ignored() {
        let a = answer(7, &[("PC", 0x00, false)]);
        assert_eq!(parse(8, &a), None, "other transaction");
        assert_eq!(parse(7, &query(7)), None, "a question, not an answer");
        for cut in [0, 5, 13, 50, 60] {
            assert_eq!(parse(7, &a[..cut.min(a.len())]), None);
        }
        assert_eq!(parse(7, &[0xff; 300]), None);
    }
}
