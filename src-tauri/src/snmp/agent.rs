//! A minimal SNMPv2c agent over a fixed table of values, for tests and the device
//! simulator (`--snmp`). Answers Get, GetNext and GetBulk for one community; anything
//! else is ignored, like a real agent ignores a wrong community. Standard library only.

use std::collections::BTreeMap;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::ber::{Message, Oid, PduKind, Value, decode, encode};

pub type Mib = BTreeMap<Oid, Value>;

fn next(mib: &Mib, oid: &Oid) -> (Oid, Value) {
    mib.range::<Oid, _>((std::ops::Bound::Excluded(oid), std::ops::Bound::Unbounded))
        .next()
        .map_or_else(
            || (oid.clone(), Value::Missing),
            |(o, v)| (o.clone(), v.clone()),
        )
}

/// The response to one request datagram, or None when it is to be ignored.
pub fn answer(mib: &Mib, community: &[u8], datagram: &[u8]) -> Option<Vec<u8>> {
    let req = decode(datagram)?;
    if req.community != community {
        return None;
    }
    let bindings = match req.kind {
        PduKind::Get => req
            .bindings
            .iter()
            .map(|(o, _)| (o.clone(), mib.get(o).cloned().unwrap_or(Value::Missing)))
            .collect(),
        PduKind::GetNext => req.bindings.iter().map(|(o, _)| next(mib, o)).collect(),
        PduKind::GetBulk => {
            let mut out = Vec::new();
            for (o, _) in &req.bindings {
                let mut cur = o.clone();
                for _ in 0..req.b.clamp(1, 50) {
                    let (n, v) = next(mib, &cur);
                    let end = v == Value::Missing;
                    cur.clone_from(&n);
                    out.push((n, v));
                    if end {
                        break;
                    }
                }
            }
            out
        }
        PduKind::Response => return None,
    };
    Some(encode(&Message {
        community: req.community,
        kind: PduKind::Response,
        request_id: req.request_id,
        a: 0,
        b: 0,
        bindings,
    }))
}

/// Serves `mib` on `socket` until `stop` is set.
pub fn serve(socket: &UdpSocket, mib: &Mib, community: &[u8], stop: &Arc<AtomicBool>) {
    let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        let Ok((n, from)) = socket.recv_from(&mut buf) else {
            continue;
        };
        if let Some(reply) = answer(mib, community, &buf[..n]) {
            let _ = socket.send_to(&reply, from);
        }
    }
}

fn oid(text: &str) -> Oid {
    text.split('.').filter_map(|p| p.parse().ok()).collect()
}

fn mac(text: &str) -> Vec<u8> {
    let hex: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    hex.chunks(2)
        .filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

/// One LLDP neighbour of a [`sample_mib`] device.
pub struct SampleNeighbor {
    pub local_port: u32,
    pub chassis: &'static str,
    pub port_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// LLDP capability bits (bridge 0x20, wlan 0x10, router 0x08, phone 0x04).
    pub caps: u8,
    pub ip: [u8; 4],
}

/// A switch as an agent presents it: system group, interface names, LLDP local and remote
/// tables, the forwarding table and the ARP table.
pub fn sample_mib(
    name: &str,
    description: &str,
    chassis: &str,
    ports: &[(u32, &str)],
    neighbors: &[SampleNeighbor],
    fdb: &[(&str, u32)],
    arp: &[([u8; 4], &str)],
) -> Mib {
    let mut m = Mib::new();
    let text = |s: &str| Value::Bytes(s.as_bytes().to_vec());
    m.insert(oid("1.3.6.1.2.1.1.1.0"), text(description));
    m.insert(oid("1.3.6.1.2.1.1.5.0"), text(name));
    m.insert(oid("1.0.8802.1.1.2.1.3.2.0"), Value::Bytes(mac(chassis)));
    for (index, port) in ports {
        m.insert(oid(&format!("1.3.6.1.2.1.31.1.1.1.1.{index}")), text(port));
        m.insert(
            oid(&format!("1.0.8802.1.1.2.1.3.7.1.3.{index}")),
            text(port),
        );
        // Bridge port number = ifIndex in this sample.
        m.insert(
            oid(&format!("1.3.6.1.2.1.17.1.4.1.2.{index}")),
            Value::Int(i64::from(*index)),
        );
    }
    for (i, n) in neighbors.iter().enumerate() {
        let row = format!("0.{}.{}", n.local_port, i + 1);
        let col = |c: u32| oid(&format!("1.0.8802.1.1.2.1.4.1.1.{c}.{row}"));
        m.insert(col(4), Value::Int(4));
        m.insert(col(5), Value::Bytes(mac(n.chassis)));
        m.insert(col(6), Value::Int(5));
        m.insert(col(7), text(n.port_id));
        m.insert(col(9), text(n.name));
        m.insert(col(10), text(n.description));
        m.insert(col(12), Value::Bytes(vec![n.caps, 0]));
        let [a, b, c, d] = n.ip;
        m.insert(
            oid(&format!(
                "1.0.8802.1.1.2.1.4.2.1.3.{row}.1.4.{a}.{b}.{c}.{d}"
            )),
            Value::Int(2),
        );
    }
    for (address, port) in fdb {
        let index: Vec<String> = mac(address).iter().map(ToString::to_string).collect();
        m.insert(
            oid(&format!("1.3.6.1.2.1.17.4.3.1.2.{}", index.join("."))),
            Value::Int(i64::from(*port)),
        );
    }
    for ([a, b, c, d], address) in arp {
        m.insert(
            oid(&format!("1.3.6.1.2.1.4.22.1.2.1.{a}.{b}.{c}.{d}")),
            Value::Bytes(mac(address)),
        );
    }
    m
}
