//! Flow export datagrams → "this address was seen entering this port of this exporter".
//! sFlow v5 (flow samples: raw Ethernet header, sampled Ethernet / IPv4 records),
//! NetFlow v5, NetFlow v9 and IPFIX (via their templates). Standard library only.
//!
//! Input comes straight from the network: every read is bounds-checked, nothing panics,
//! and work per datagram is bounded.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

/// One sighting: a source address on an input port of an exporter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Observation {
    /// The exporting device (sFlow agent address, else the UDP source).
    pub agent: IpAddr,
    /// SNMP ifIndex of the input port.
    pub if_index: u32,
    pub mac: Option<[u8; 6]>,
    pub ip: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Sflow,
    Netflow,
}

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn usable_mac(m: &[u8]) -> Option<[u8; 6]> {
    let mac: [u8; 6] = m.try_into().ok()?;
    // Not multicast / broadcast, not all zero.
    (mac[0] & 1 == 0 && mac != [0; 6]).then_some(mac)
}

fn usable_ip(b: &[u8]) -> Option<Ipv4Addr> {
    let ip = Ipv4Addr::new(*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?);
    (!ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast()).then_some(ip)
}

/// Source MAC and (IPv4) source address of an Ethernet frame header.
fn ethernet(frame: &[u8]) -> (Option<[u8; 6]>, Option<Ipv4Addr>) {
    let mac = frame.get(6..12).and_then(usable_mac);
    let mut at = 12;
    let mut ethertype = frame
        .get(at..at + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]));
    // Up to two VLAN tags.
    for _ in 0..2 {
        if matches!(ethertype, Some(0x8100 | 0x88A8)) {
            at += 4;
            ethertype = frame
                .get(at..at + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]));
        }
    }
    let ip = (ethertype == Some(0x0800))
        .then(|| frame.get(at + 2 + 12..at + 2 + 16).and_then(usable_ip))
        .flatten();
    (mac, ip)
}

// ---- sFlow v5 -------------------------------------------------------------------------

const MAX_SAMPLES: u32 = 512;
const MAX_RECORDS: u32 = 64;

pub fn sflow(datagram: &[u8]) -> Vec<Observation> {
    sflow_inner(datagram).unwrap_or_default()
}

fn sflow_inner(datagram: &[u8]) -> Option<Vec<Observation>> {
    let mut r = R(datagram);
    if r.u32()? != 5 {
        return None;
    }
    let agent: IpAddr = match r.u32()? {
        1 => IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(r.take(4)?).ok()?)),
        2 => IpAddr::V6(<[u8; 16]>::try_from(r.take(16)?).ok()?.into()),
        _ => return None,
    };
    r.take(12)?; // sub-agent id, sequence, uptime
    let samples = r.u32()?.min(MAX_SAMPLES);
    let mut out = Vec::new();
    for _ in 0..samples {
        let Some(kind) = r.u32() else { break };
        let Some(len) = r.u32() else { break };
        let Some(body) = r.take(len as usize) else {
            break;
        };
        // Enterprise 0: 1 = flow sample, 3 = expanded flow sample. Counters are skipped.
        let mut s = R(body);
        let input = match kind {
            1 => {
                s.take(20); // sequence, source id, rate, pool, drops
                let Some(input) = s.u32() else { continue };
                s.take(4);
                // Top two bits: 0 = a single interface.
                if input >> 30 != 0 {
                    continue;
                }
                input & 0x3FFF_FFFF
            }
            3 => {
                s.take(24); // sequence, source type + index, rate, pool, drops
                let (Some(format), Some(value)) = (s.u32(), s.u32()) else {
                    continue;
                };
                s.take(8);
                if format != 0 {
                    continue;
                }
                value
            }
            _ => continue,
        };
        if input == 0 || input == 0x3FFF_FFFF {
            continue;
        }
        let Some(records) = s.u32() else { continue };
        let (mut mac, mut ip) = (None, None);
        for _ in 0..records.min(MAX_RECORDS) {
            let (Some(format), Some(len)) = (s.u32(), s.u32()) else {
                break;
            };
            // Record bodies are padded to four bytes by their length field already.
            let Some(body) = s.take(len as usize) else {
                break;
            };
            let mut b = R(body);
            match format {
                // Raw packet header: protocol (1 = Ethernet), frame length, stripped, size.
                1 => {
                    if b.u32() == Some(1) {
                        b.take(8);
                        if let Some(size) = b.u32() {
                            let frame = b.take((size as usize).min(b.0.len())).unwrap_or(&[]);
                            let (m, i) = ethernet(frame);
                            mac = mac.or(m);
                            ip = ip.or(i);
                        }
                    }
                }
                // Sampled Ethernet: length, source MAC (+2 pad), destination MAC, type.
                2 => {
                    b.take(4);
                    mac = mac.or(b.take(6).and_then(usable_mac));
                }
                // Sampled IPv4: length, protocol, source address, …
                3 => {
                    b.take(8);
                    ip = ip.or(b.take(4).and_then(usable_ip));
                }
                _ => {}
            }
        }
        if mac.is_some() || ip.is_some() {
            out.push(Observation {
                agent,
                if_index: input,
                mac,
                ip,
            });
        }
    }
    Some(out)
}

// ---- NetFlow v5 / v9 / IPFIX ----------------------------------------------------------

/// Field types (same numbers in NetFlow v9 and IPFIX).
const F_SRC_ADDR: u16 = 8;
const F_INPUT: u16 = 10;
const F_SRC_MAC: u16 = 56;

const MAX_TEMPLATES: usize = 1024;
const MAX_FIELDS: usize = 256;
const MAX_FLOWS: usize = 2000;

/// Templates announced by exporters: (exporter, source / domain id, template id).
#[derive(Default)]
pub struct Templates(HashMap<(IpAddr, u32, u16), Vec<(u16, u16)>>);

impl Templates {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

pub fn netflow(datagram: &[u8], from: IpAddr, templates: &mut Templates) -> Vec<Observation> {
    let mut out = Vec::new();
    let mut r = R(datagram);
    match r.u16() {
        Some(5) => v5(&mut r, from, &mut out),
        Some(9) => {
            // count, uptime, unix seconds, sequence, source id
            let _ = r.take(14);
            if let Some(source) = r.u32() {
                sets(&mut r, from, source, templates, 0, 1, &mut out);
            }
        }
        Some(10) => {
            // length, export time, sequence, observation domain
            let _ = r.take(10);
            if let Some(domain) = r.u32() {
                sets(&mut r, from, domain, templates, 2, 3, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn v5(r: &mut R, from: IpAddr, out: &mut Vec<Observation>) {
    let Some(count) = r.u16() else { return };
    if r.take(20).is_none() {
        return;
    }
    for _ in 0..count.min(30) {
        let Some(rec) = r.take(48) else { return };
        let input = u32::from(u16::from_be_bytes([rec[12], rec[13]]));
        if let (Some(ip), true) = (usable_ip(&rec[0..4]), input != 0) {
            out.push(Observation {
                agent: from,
                if_index: input,
                mac: None,
                ip: Some(ip),
            });
        }
    }
}

/// Walks the flow sets of a v9 packet / the sets of an IPFIX message.
fn sets(
    r: &mut R,
    from: IpAddr,
    source: u32,
    templates: &mut Templates,
    template_set: u16,
    options_set: u16,
    out: &mut Vec<Observation>,
) {
    while let (Some(id), Some(len)) = (r.u16(), r.u16()) {
        let Some(body) = r.take(usize::from(len).saturating_sub(4)) else {
            return;
        };
        if len < 4 {
            return;
        }
        if id == template_set {
            let mut t = R(body);
            while let (Some(tid), Some(count)) = (t.u16(), t.u16()) {
                if tid < 256 || usize::from(count) > MAX_FIELDS {
                    break;
                }
                let mut fields = Vec::with_capacity(usize::from(count));
                let mut ok = true;
                for _ in 0..count {
                    let (Some(kind), Some(size)) = (t.u16(), t.u16()) else {
                        ok = false;
                        break;
                    };
                    // IPFIX enterprise field: four more bytes, and not one of ours.
                    let kind = if kind & 0x8000 != 0 && template_set == 2 {
                        if t.take(4).is_none() {
                            ok = false;
                            break;
                        }
                        0
                    } else {
                        kind
                    };
                    fields.push((kind, size));
                }
                // Variable-length fields (65535) make record sizes unknowable here.
                if ok && fields.iter().all(|(_, s)| *s != 0xFFFF) {
                    let key = (from, source, tid);
                    if templates.0.len() < MAX_TEMPLATES || templates.0.contains_key(&key) {
                        templates.0.insert(key, fields);
                    }
                }
                if !ok {
                    break;
                }
            }
        } else if id == options_set {
            // Options describe the exporter, not flows.
        } else if id >= 256 {
            let Some(fields) = templates.0.get(&(from, source, id)) else {
                continue;
            };
            let size: usize = fields.iter().map(|(_, s)| usize::from(*s)).sum();
            if size == 0 {
                continue;
            }
            let mut d = R(body);
            while let Some(rec) = d.take(size) {
                let (mut at, mut mac, mut ip, mut input) = (0usize, None, None, 0u32);
                for (kind, len) in fields {
                    let len = usize::from(*len);
                    let v = &rec[at..at + len];
                    at += len;
                    match *kind {
                        F_SRC_ADDR if len == 4 => ip = usable_ip(v),
                        F_SRC_MAC if len == 6 => mac = usable_mac(v),
                        F_INPUT if (1..=4).contains(&len) => {
                            input = v.iter().fold(0u32, |a, b| (a << 8) | u32::from(*b));
                        }
                        _ => {}
                    }
                }
                if input != 0 && (mac.is_some() || ip.is_some()) {
                    out.push(Observation {
                        agent: from,
                        if_index: input,
                        mac,
                        ip,
                    });
                }
                if out.len() >= MAX_FLOWS {
                    return;
                }
            }
        }
    }
}
