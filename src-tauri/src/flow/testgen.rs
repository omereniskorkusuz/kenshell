//! Builders for flow export datagrams: used by the tests and, included by path, by the
//! device simulator (`--sflow-to`, `--netflow-to`). Standard library only.

fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

/// An Ethernet + IPv4 header as a switch would sample it.
pub fn frame(src_mac: [u8; 6], src_ip: [u8; 4], vlan: Option<u16>) -> Vec<u8> {
    let mut f = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x00];
    f.extend_from_slice(&src_mac);
    if let Some(v) = vlan {
        f.extend_from_slice(&[0x81, 0x00]);
        f.extend_from_slice(&v.to_be_bytes());
    }
    f.extend_from_slice(&[0x08, 0x00]);
    let mut ip = vec![0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6, 0, 0];
    ip.extend_from_slice(&src_ip);
    ip.extend_from_slice(&[10, 0, 0, 1]);
    f.extend_from_slice(&ip);
    f.extend_from_slice(&[0; 20]);
    f
}

/// sFlow v5 datagram with one flow sample per `(input ifIndex, frame)`; `expanded` uses
/// the expanded sample layout.
pub fn sflow_v5(agent: [u8; 4], samples: &[(u32, Vec<u8>)], expanded: bool) -> Vec<u8> {
    let mut d = Vec::new();
    let u = |d: &mut Vec<u8>, v: u32| d.extend_from_slice(&v.to_be_bytes());
    u(&mut d, 5);
    u(&mut d, 1);
    d.extend_from_slice(&agent);
    u(&mut d, 0);
    u(&mut d, 1);
    u(&mut d, 1000);
    u(&mut d, samples.len() as u32);
    for (input, frame) in samples {
        let mut record = Vec::new();
        u(&mut record, 1); // header protocol: Ethernet
        u(&mut record, frame.len() as u32 + 4);
        u(&mut record, 4);
        u(&mut record, frame.len() as u32);
        record.extend_from_slice(frame);
        pad4(&mut record);

        let mut s = Vec::new();
        u(&mut s, 1); // sequence
        if expanded {
            u(&mut s, 0);
            u(&mut s, *input);
        } else {
            u(&mut s, *input);
        }
        u(&mut s, 512);
        u(&mut s, 1000);
        u(&mut s, 0);
        if expanded {
            u(&mut s, 0);
            u(&mut s, *input);
            u(&mut s, 0);
            u(&mut s, 1);
        } else {
            u(&mut s, *input);
            u(&mut s, 1);
        }
        u(&mut s, 1); // one record
        u(&mut s, 1); // raw packet header
        u(&mut s, record.len() as u32);
        s.extend_from_slice(&record);

        u(&mut d, if expanded { 3 } else { 1 });
        u(&mut d, s.len() as u32);
        d.extend_from_slice(&s);
    }
    d
}

/// NetFlow v5 packet with one record per `(source address, input ifIndex)`.
pub fn netflow_v5(flows: &[([u8; 4], u16)]) -> Vec<u8> {
    let mut d = vec![0, 5];
    d.extend_from_slice(&(flows.len() as u16).to_be_bytes());
    d.extend_from_slice(&[0; 20]);
    for (src, input) in flows {
        let mut rec = [0u8; 48];
        rec[0..4].copy_from_slice(src);
        rec[4..8].copy_from_slice(&[10, 0, 0, 1]);
        rec[12..14].copy_from_slice(&input.to_be_bytes());
        d.extend_from_slice(&rec);
    }
    d
}

/// Template 256 = source address (4), input interface (4), source MAC (6), packets (4),
/// plus data records, as NetFlow v9 (`ipfix` false) or IPFIX.
pub fn template_flows(
    ipfix: bool,
    flows: &[([u8; 4], u32, [u8; 6])],
    with_template: bool,
) -> Vec<u8> {
    let mut sets = Vec::new();
    if with_template {
        let mut t = Vec::new();
        t.extend_from_slice(&256u16.to_be_bytes());
        t.extend_from_slice(&4u16.to_be_bytes());
        for (kind, len) in [(8u16, 4u16), (10, 4), (56, 6), (2, 4)] {
            t.extend_from_slice(&kind.to_be_bytes());
            t.extend_from_slice(&len.to_be_bytes());
        }
        sets.extend_from_slice(&(if ipfix { 2u16 } else { 0 }).to_be_bytes());
        sets.extend_from_slice(&(t.len() as u16 + 4).to_be_bytes());
        sets.extend_from_slice(&t);
    }
    let mut data = Vec::new();
    for (src, input, mac) in flows {
        data.extend_from_slice(src);
        data.extend_from_slice(&input.to_be_bytes());
        data.extend_from_slice(mac);
        data.extend_from_slice(&7u32.to_be_bytes());
    }
    sets.extend_from_slice(&256u16.to_be_bytes());
    sets.extend_from_slice(&(data.len() as u16 + 4).to_be_bytes());
    sets.extend_from_slice(&data);

    let mut d = Vec::new();
    if ipfix {
        d.extend_from_slice(&10u16.to_be_bytes());
        d.extend_from_slice(&(sets.len() as u16 + 16).to_be_bytes());
        d.extend_from_slice(&[0; 8]); // export time, sequence
        d.extend_from_slice(&1u32.to_be_bytes()); // observation domain
    } else {
        d.extend_from_slice(&9u16.to_be_bytes());
        d.extend_from_slice(&2u16.to_be_bytes());
        d.extend_from_slice(&[0; 12]); // uptime, seconds, sequence
        d.extend_from_slice(&1u32.to_be_bytes()); // source id
    }
    d.extend_from_slice(&sets);
    d
}
