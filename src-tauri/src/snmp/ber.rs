//! SNMPv2c wire format: the small part of ASN.1 BER that SNMP uses. Standard library only
//! (the device simulator includes this file for its test agent).
//!
//! Decoding never panics and never trusts a length: everything is bounds-checked.

pub type Oid = Vec<u32>;

/// A variable binding value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    Oid(Oid),
    Ip([u8; 4]),
    /// Counter32, Gauge32, TimeTicks, Counter64.
    Unsigned(u64),
    Null,
    /// noSuchObject, noSuchInstance or endOfMibView.
    Missing,
}

impl Value {
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Unsigned(u) => i64::try_from(*u).ok(),
            _ => None,
        }
    }
    /// Text when the bytes are printable (trailing NULs dropped).
    pub fn text(&self) -> Option<String> {
        let b = self.bytes()?;
        let b = &b[..b.iter().rposition(|c| *c != 0).map_or(0, |i| i + 1)];
        let s = std::str::from_utf8(b).ok()?;
        (!s.is_empty()
            && s.chars()
                .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t'))
        .then(|| s.trim().to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PduKind {
    Get = 0xA0,
    GetNext = 0xA1,
    Response = 0xA2,
    GetBulk = 0xA5,
    /// SNMPv3 engine discovery and USM errors.
    Report = 0xA8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub community: Vec<u8>,
    pub kind: PduKind,
    pub request_id: i32,
    /// error-status (non-repeaters in a GetBulk request).
    pub a: i32,
    /// error-index (max-repetitions in a GetBulk request).
    pub b: i32,
    pub bindings: Vec<(Oid, Value)>,
}

// ---- encoding -------------------------------------------------------------------------

fn put_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}

pub(crate) fn tlv(tag: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    put_len(out, body.len());
    out.extend_from_slice(body);
}

pub(crate) fn int_body(v: i64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut i = 0;
    // Drop redundant sign bytes, keep at least one.
    while i < 7 && ((b[i] == 0 && b[i + 1] & 0x80 == 0) || (b[i] == 0xFF && b[i + 1] & 0x80 != 0)) {
        i += 1;
    }
    b[i..].to_vec()
}

fn oid_body(oid: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(oid.len() + 1);
    let first = oid.first().copied().unwrap_or(0) * 40 + oid.get(1).copied().unwrap_or(0);
    for sub in std::iter::once(first).chain(oid.iter().skip(2).copied()) {
        let mut stack = [0u8; 5];
        let mut n = 0;
        let mut v = sub;
        loop {
            stack[n] = (v & 0x7F) as u8;
            n += 1;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        for k in (0..n).rev() {
            out.push(stack[k] | if k > 0 { 0x80 } else { 0 });
        }
    }
    out
}

fn value(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Int(i) => tlv(0x02, &int_body(*i), out),
        Value::Bytes(b) => tlv(0x04, b, out),
        Value::Oid(o) => tlv(0x06, &oid_body(o), out),
        Value::Ip(a) => tlv(0x40, a, out),
        Value::Unsigned(u) => {
            let mut body = int_body(i64::try_from(*u).unwrap_or(i64::MAX));
            if body.len() > 1 && body[0] == 0 && body[1] & 0x80 == 0 {
                body.remove(0);
            }
            tlv(0x42, &body, out);
        }
        Value::Null => tlv(0x05, &[], out),
        Value::Missing => tlv(0x82, &[], out),
    }
}

/// The PDU alone (tag, request id, the two numbers, bindings): what SNMPv3 wraps.
pub fn encode_pdu(m: &Message) -> Vec<u8> {
    let mut binds = Vec::new();
    for (oid, v) in &m.bindings {
        let mut one = Vec::new();
        tlv(0x06, &oid_body(oid), &mut one);
        value(v, &mut one);
        tlv(0x30, &one, &mut binds);
    }
    let mut pdu = Vec::new();
    tlv(0x02, &int_body(i64::from(m.request_id)), &mut pdu);
    tlv(0x02, &int_body(i64::from(m.a)), &mut pdu);
    tlv(0x02, &int_body(i64::from(m.b)), &mut pdu);
    tlv(0x30, &binds, &mut pdu);
    let mut out = Vec::new();
    tlv(m.kind as u8, &pdu, &mut out);
    out
}

pub fn encode(m: &Message) -> Vec<u8> {
    let mut body = Vec::new();
    tlv(0x02, &[1], &mut body); // version: v2c
    tlv(0x04, &m.community, &mut body);
    body.extend_from_slice(&encode_pdu(m));
    let mut out = Vec::new();
    tlv(0x30, &body, &mut out);
    out
}

// ---- decoding -------------------------------------------------------------------------

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);

impl<'a> Reader<'a> {
    pub(crate) fn tlv(&mut self) -> Option<(u8, &'a [u8])> {
        let (&tag, rest) = self.0.split_first()?;
        let (&l0, rest) = rest.split_first()?;
        let (len, rest) = if l0 < 0x80 {
            (usize::from(l0), rest)
        } else {
            let n = usize::from(l0 & 0x7F);
            if n == 0 || n > 4 || rest.len() < n {
                return None;
            }
            let len = rest[..n]
                .iter()
                .fold(0usize, |a, b| (a << 8) | usize::from(*b));
            (len, &rest[n..])
        };
        if rest.len() < len {
            return None;
        }
        self.0 = &rest[len..];
        Some((tag, &rest[..len]))
    }
    pub(crate) fn expect(&mut self, tag: u8) -> Option<&'a [u8]> {
        let (t, body) = self.tlv()?;
        (t == tag).then_some(body)
    }
}

pub(crate) fn int_of(body: &[u8]) -> Option<i64> {
    if body.is_empty() || body.len() > 8 {
        return None;
    }
    let mut v: i64 = if body[0] & 0x80 != 0 { -1 } else { 0 };
    for b in body {
        v = (v << 8) | i64::from(*b);
    }
    Some(v)
}

fn unsigned_of(body: &[u8]) -> Option<u64> {
    let body = if body.len() > 1 && body[0] == 0 {
        &body[1..]
    } else {
        body
    };
    if body.is_empty() || body.len() > 8 {
        return None;
    }
    Some(body.iter().fold(0u64, |a, b| (a << 8) | u64::from(*b)))
}

fn oid_of(body: &[u8]) -> Option<Oid> {
    let mut subs = Vec::new();
    let mut cur: u32 = 0;
    let mut pending = false;
    for b in body {
        cur = cur.checked_mul(128)? | u32::from(b & 0x7F);
        pending = b & 0x80 != 0;
        if !pending {
            subs.push(cur);
            cur = 0;
        }
    }
    if pending || subs.is_empty() {
        return None;
    }
    let first = subs[0];
    let mut oid = Vec::with_capacity(subs.len() + 1);
    let top = (first / 40).min(2);
    oid.push(top);
    oid.push(first - top * 40);
    oid.extend_from_slice(&subs[1..]);
    Some(oid)
}

fn value_of(tag: u8, body: &[u8]) -> Option<Value> {
    Some(match tag {
        0x02 => Value::Int(int_of(body)?),
        0x04 => Value::Bytes(body.to_vec()),
        0x05 => Value::Null,
        0x06 => Value::Oid(oid_of(body)?),
        0x40 => Value::Ip(body.try_into().ok()?),
        0x41 | 0x42 | 0x43 | 0x46 => Value::Unsigned(unsigned_of(body)?),
        0x80..=0x82 => Value::Missing,
        // Opaque and anything unknown: kept as raw bytes.
        _ => Value::Bytes(body.to_vec()),
    })
}

pub fn decode(datagram: &[u8]) -> Option<Message> {
    let mut top = Reader(datagram);
    let mut msg = Reader(top.expect(0x30)?);
    if int_of(msg.expect(0x02)?)? != 1 {
        return None; // not v2c
    }
    let community = msg.expect(0x04)?.to_vec();
    let (tag, pdu) = msg.tlv()?;
    decode_pdu_body(tag, pdu, community)
}

/// A PDU (one TLV) as carried inside an SNMPv3 scoped PDU.
pub fn decode_pdu(bytes: &[u8]) -> Option<Message> {
    let (tag, pdu) = Reader(bytes).tlv()?;
    decode_pdu_body(tag, pdu, Vec::new())
}

fn decode_pdu_body(tag: u8, pdu: &[u8], community: Vec<u8>) -> Option<Message> {
    let kind = match tag {
        0xA0 => PduKind::Get,
        0xA1 => PduKind::GetNext,
        0xA2 => PduKind::Response,
        0xA5 => PduKind::GetBulk,
        0xA8 => PduKind::Report,
        _ => return None,
    };
    let mut pdu = Reader(pdu);
    let num = |r: &mut Reader| -> Option<i32> { i32::try_from(int_of(r.expect(0x02)?)?).ok() };
    let request_id = num(&mut pdu)?;
    let a = num(&mut pdu)?;
    let b = num(&mut pdu)?;
    let mut list = Reader(pdu.expect(0x30)?);
    let mut bindings = Vec::new();
    while !list.0.is_empty() {
        let mut one = Reader(list.expect(0x30)?);
        let oid = oid_of(one.expect(0x06)?)?;
        let (tag, body) = one.tlv()?;
        bindings.push((oid, value_of(tag, body)?));
    }
    Some(Message {
        community,
        kind,
        request_id,
        a,
        b,
        bindings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(v: Value) -> Value {
        let m = Message {
            community: b"public".to_vec(),
            kind: PduKind::Response,
            request_id: 42,
            a: 0,
            b: 0,
            bindings: vec![(vec![1, 3, 6, 1, 2, 1, 1, 5, 0], v)],
        };
        decode(&encode(&m)).unwrap().bindings.remove(0).1
    }

    #[test]
    fn values_round_trip() {
        for v in [
            Value::Int(0),
            Value::Int(127),
            Value::Int(128),
            Value::Int(-1),
            Value::Int(-129),
            Value::Int(i64::from(i32::MAX)),
            Value::Bytes(b"core-sw1".to_vec()),
            Value::Bytes(vec![0; 300]),
            Value::Oid(vec![1, 0, 8802, 1, 1, 2, 1, 4, 1, 1, 9, 0, 5, 1]),
            Value::Oid(vec![2, 999, 4_000_000_000]),
            Value::Ip([10, 0, 0, 1]),
            Value::Unsigned(0),
            Value::Unsigned(4_294_967_295),
            Value::Null,
            Value::Missing,
        ] {
            assert_eq!(round(v.clone()), v);
        }
    }

    #[test]
    fn a_known_request_encodes_byte_for_byte() {
        // GetRequest sysDescr.0, community "public", request-id 1.
        let m = Message {
            community: b"public".to_vec(),
            kind: PduKind::Get,
            request_id: 1,
            a: 0,
            b: 0,
            bindings: vec![(vec![1, 3, 6, 1, 2, 1, 1, 1, 0], Value::Null)],
        };
        assert_eq!(
            encode(&m),
            [
                0x30, 0x26, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xA0,
                0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0E, 0x30, 0x0C,
                0x06, 0x08, 0x2B, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00
            ]
        );
        assert_eq!(decode(&encode(&m)).unwrap(), m);
    }

    #[test]
    fn garbage_never_panics() {
        let good = encode(&Message {
            community: b"c".to_vec(),
            kind: PduKind::GetBulk,
            request_id: 7,
            a: 0,
            b: 25,
            bindings: vec![(vec![1, 3, 6, 1], Value::Null), (vec![1, 3], Value::Int(5))],
        });
        // Every truncation and every single-byte corruption.
        for cut in 0..good.len() {
            let _ = decode(&good[..cut]);
        }
        let mut x: u32 = 0x9E37_79B9;
        for i in 0..good.len() {
            for _ in 0..8 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let mut bad = good.clone();
                bad[i] = x as u8;
                let _ = decode(&bad);
            }
        }
        assert!(decode(&[]).is_none());
        assert!(decode(&[0x30, 0x84, 0xFF, 0xFF, 0xFF, 0xFF]).is_none());
        assert_eq!(
            Value::Bytes(b"abc\0\0".to_vec()).text().as_deref(),
            Some("abc")
        );
        assert_eq!(Value::Bytes(vec![0, 0x11, 0x22]).text(), None);
    }
}
