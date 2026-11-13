//! SNMPv3 with the User-based Security Model (RFC 3414): engine discovery, key
//! localisation, message authentication (HMAC-MD5-96, HMAC-SHA-96, HMAC-SHA-256-192 from
//! RFC 7860) and privacy (AES-128-CFB from RFC 3826, DES-CBC). Only what a read-only
//! poller needs; the crypto primitives are the ones the SSH stack already builds.
//!
//! Passwords and keys live in `Zeroizing` buffers and never reach logs or errors.

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use zeroize::Zeroizing;

use super::ber::{Message, Reader, decode_pdu, encode_pdu, int_body, int_of, tlv};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthProto {
    Md5,
    Sha,
    Sha256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivProto {
    Aes,
    Des,
}

/// A v3 user as the operator configured it on the devices.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V3User {
    pub user: String,
    pub auth: Option<AuthProto>,
    #[serde(default, with = "zstr")]
    pub auth_pass: Zeroizing<String>,
    pub privacy: Option<PrivProto>,
    #[serde(default, with = "zstr")]
    pub priv_pass: Zeroizing<String>,
}

/// Serde for secret strings: the plain copy only lives inside serde's call.
mod zstr {
    use zeroize::Zeroizing;
    pub fn serialize<S: serde::Serializer>(v: &Zeroizing<String>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(v)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Zeroizing<String>, D::Error> {
        <String as serde::Deserialize>::deserialize(d).map(Zeroizing::new)
    }
}

impl std::fmt::Debug for V3User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V3User")
            .field("user", &self.user)
            .field("auth", &self.auth)
            .field("privacy", &self.privacy)
            .finish_non_exhaustive()
    }
}

impl AuthProto {
    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            AuthProto::Md5 => md5::compute(data).0.to_vec(),
            AuthProto::Sha => ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, data)
                .as_ref()
                .to_vec(),
            AuthProto::Sha256 => ring::digest::digest(&ring::digest::SHA256, data)
                .as_ref()
                .to_vec(),
        }
    }

    /// Length of the authentication parameter in a message.
    pub fn mac_len(self) -> usize {
        match self {
            AuthProto::Md5 | AuthProto::Sha => 12,
            AuthProto::Sha256 => 24,
        }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        let full = match self {
            AuthProto::Md5 => {
                // HMAC (RFC 2104) over MD5, which ring does not offer.
                let mut k = [0u8; 64];
                k[..key.len().min(64)].copy_from_slice(&key[..key.len().min(64)]);
                let mut inner = Vec::with_capacity(64 + data.len());
                inner.extend(k.iter().map(|b| b ^ 0x36));
                inner.extend_from_slice(data);
                let ih = md5::compute(&inner);
                let mut outer = Vec::with_capacity(80);
                outer.extend(k.iter().map(|b| b ^ 0x5c));
                outer.extend_from_slice(&ih.0);
                md5::compute(&outer).0.to_vec()
            }
            AuthProto::Sha => {
                let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
                ring::hmac::sign(&k, data).as_ref().to_vec()
            }
            AuthProto::Sha256 => {
                let k = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
                ring::hmac::sign(&k, data).as_ref().to_vec()
            }
        };
        full[..self.mac_len()].to_vec()
    }
}

/// Password → key (RFC 3414 A.2): the hash of 1 MB of the repeated password.
pub fn password_to_key(proto: AuthProto, password: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut buf = Zeroizing::new(Vec::with_capacity(1_048_576));
    if !password.is_empty() {
        while buf.len() < 1_048_576 {
            let need = 1_048_576 - buf.len();
            buf.extend_from_slice(&password[..password.len().min(need)]);
        }
    }
    Zeroizing::new(proto.digest(&buf))
}

/// Key localised to one engine: hash(Ku ‖ engineID ‖ Ku).
pub fn localize(proto: AuthProto, ku: &[u8], engine_id: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(Vec::with_capacity(ku.len() * 2 + engine_id.len()));
    v.extend_from_slice(ku);
    v.extend_from_slice(engine_id);
    v.extend_from_slice(ku);
    Zeroizing::new(proto.digest(&v))
}

fn aes_cfb(key: &[u8], iv: [u8; 16], data: &[u8], decrypt: bool) -> Option<Vec<u8>> {
    let cipher = aes::Aes128::new_from_slice(key.get(..16)?).ok()?;
    let mut feedback = iv;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(16) {
        let mut block = feedback.into();
        cipher.encrypt_block(&mut block);
        let ks: [u8; 16] = block.into();
        let mut next = [0u8; 16];
        for (i, b) in chunk.iter().enumerate() {
            let o = b ^ ks[i];
            out.push(o);
            next[i] = if decrypt { *b } else { o };
        }
        feedback = next;
    }
    Some(out)
}

fn des_cbc(key: &[u8], iv: [u8; 8], data: &[u8], decrypt: bool) -> Option<Vec<u8>> {
    let cipher = des::Des::new_from_slice(key.get(..8)?).ok()?;
    let mut prev = iv;
    let mut out = Vec::with_capacity(data.len() + 8);
    if decrypt {
        if !data.len().is_multiple_of(8) {
            return None;
        }
        for chunk in data.chunks(8) {
            let c: [u8; 8] = chunk.try_into().ok()?;
            let mut block = c.into();
            cipher.decrypt_block(&mut block);
            let p: [u8; 8] = block.into();
            out.extend(p.iter().zip(prev).map(|(a, b)| a ^ b));
            prev = c;
        }
    } else {
        let mut padded = data.to_vec();
        while !padded.len().is_multiple_of(8) {
            padded.push(0);
        }
        for chunk in padded.chunks(8) {
            let mut x = [0u8; 8];
            for i in 0..8 {
                x[i] = chunk[i] ^ prev[i];
            }
            let mut block = x.into();
            cipher.encrypt_block(&mut block);
            let c: [u8; 8] = block.into();
            out.extend_from_slice(&c);
            prev = c;
        }
    }
    Some(out)
}

/// What the agent's engine looks like to us.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    pub id: Vec<u8>,
    pub boots: i64,
    pub time: i64,
    /// When `time` was learned, to estimate the agent's clock later.
    pub at: Option<std::time::Instant>,
}

impl Engine {
    pub fn now(&self) -> i64 {
        self.time
            + self
                .at
                .map_or(0, |a| i64::try_from(a.elapsed().as_secs()).unwrap_or(0))
    }
}

/// Keys for one engine.
pub struct Keys {
    auth: Option<(AuthProto, Zeroizing<Vec<u8>>)>,
    privacy: Option<(PrivProto, Zeroizing<Vec<u8>>)>,
}

impl V3User {
    pub fn keys(&self, engine_id: &[u8]) -> Keys {
        let auth = self.auth.map(|p| {
            let ku = password_to_key(p, self.auth_pass.as_bytes());
            (p, localize(p, &ku, engine_id))
        });
        // The privacy key is localised with the authentication hash.
        let privacy = match (self.privacy, self.auth) {
            (Some(pp), Some(ap)) => {
                let ku = password_to_key(ap, self.priv_pass.as_bytes());
                Some((pp, localize(ap, &ku, engine_id)))
            }
            _ => None,
        };
        Keys { auth, privacy }
    }
}

/// A decoded v3 message (only what the poller and the test agent need).
#[derive(Debug, Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct V3Message {
    pub msg_id: i32,
    pub flags: u8,
    pub engine: Engine,
    pub user: Vec<u8>,
    pub pdu: Message,
}

pub const FLAG_AUTH: u8 = 1;
pub const FLAG_PRIV: u8 = 2;
pub const FLAG_REPORTABLE: u8 = 4;

/// Builds a v3 message. `keys` None = noAuthNoPriv (engine discovery). `salt` makes the
/// privacy IV unique per message.
pub fn encode(
    msg_id: i32,
    flags: u8,
    engine: &Engine,
    user: &[u8],
    keys: Option<&Keys>,
    salt: u64,
    pdu: &Message,
) -> Option<Vec<u8>> {
    let auth = keys
        .and_then(|k| k.auth.as_ref())
        .filter(|_| flags & FLAG_AUTH != 0);
    let privacy = keys
        .and_then(|k| k.privacy.as_ref())
        .filter(|_| flags & FLAG_PRIV != 0);
    let boots = engine.boots;
    let time = engine.now();

    // Scoped PDU, encrypted when asked.
    let mut scoped_body = Vec::new();
    tlv(0x04, &engine.id, &mut scoped_body);
    tlv(0x04, b"", &mut scoped_body);
    scoped_body.extend_from_slice(&encode_pdu(pdu));
    let mut scoped = Vec::new();
    tlv(0x30, &scoped_body, &mut scoped);
    let (data, priv_params) = match privacy {
        Some((PrivProto::Aes, key)) => {
            let salt = salt.to_be_bytes();
            let mut iv = [0u8; 16];
            iv[..4].copy_from_slice(&(boots as u32).to_be_bytes());
            iv[4..8].copy_from_slice(&(time as u32).to_be_bytes());
            iv[8..].copy_from_slice(&salt);
            let mut out = Vec::new();
            tlv(0x04, &aes_cfb(key, iv, &scoped, false)?, &mut out);
            (out, salt.to_vec())
        }
        Some((PrivProto::Des, key)) => {
            let mut s = [0u8; 8];
            s[..4].copy_from_slice(&(boots as u32).to_be_bytes());
            s[4..].copy_from_slice(&(salt as u32).to_be_bytes());
            let pre: [u8; 8] = key.get(8..16)?.try_into().ok()?;
            let iv: [u8; 8] = std::array::from_fn(|i| pre[i] ^ s[i]);
            let mut out = Vec::new();
            tlv(0x04, &des_cbc(key, iv, &scoped, false)?, &mut out);
            (out, s.to_vec())
        }
        None => (scoped, Vec::new()),
    };

    // USM security parameters, with a zero placeholder for the MAC.
    let mac_len = auth.map_or(0, |(p, _)| p.mac_len());
    let mut usm_body = Vec::new();
    tlv(0x04, &engine.id, &mut usm_body);
    tlv(0x02, &int_body(boots), &mut usm_body);
    tlv(0x02, &int_body(time), &mut usm_body);
    tlv(0x04, user, &mut usm_body);
    let mac_at_in_usm = usm_body.len() + 2;
    tlv(0x04, &vec![0u8; mac_len], &mut usm_body);
    tlv(0x04, &priv_params, &mut usm_body);
    let mut usm = Vec::new();
    tlv(0x30, &usm_body, &mut usm);
    let usm_header = usm.len() - usm_body.len();

    let mut global = Vec::new();
    tlv(0x02, &int_body(i64::from(msg_id)), &mut global);
    tlv(0x02, &int_body(65_507), &mut global);
    tlv(0x04, &[flags], &mut global);
    tlv(0x02, &[3], &mut global);
    let mut body = Vec::new();
    tlv(0x02, &[3], &mut body);
    tlv(0x30, &global, &mut body);
    let usm_at = body.len();
    tlv(0x04, &usm, &mut body);
    let usm_octet_header = body.len() - usm_at - usm.len();
    body.extend_from_slice(&data);
    let mut whole = Vec::new();
    tlv(0x30, &body, &mut whole);
    let header = whole.len() - body.len();

    if let Some((proto, key)) = auth {
        let at = header + usm_at + usm_octet_header + usm_header + mac_at_in_usm;
        let mac = proto.hmac(key, &whole);
        whole.get_mut(at..at + mac_len)?.copy_from_slice(&mac);
    }
    Some(whole)
}

/// Why a v3 message was refused (never the secret).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V3Error {
    Malformed,
    WrongDigest,
    Decryption,
}

/// The message id of a v3 message, without checking or decrypting anything.
pub fn peek_msg_id(datagram: &[u8]) -> Option<i32> {
    let mut msg = Reader(Reader(datagram).expect(0x30)?);
    if int_of(msg.expect(0x02)?)? != 3 {
        return None;
    }
    let mut global = Reader(msg.expect(0x30)?);
    i32::try_from(int_of(global.expect(0x02)?)?).ok()
}

/// Decodes a v3 message; with `keys`, checks the MAC and decrypts.
pub fn decode(datagram: &[u8], keys: Option<&Keys>) -> Result<V3Message, V3Error> {
    use V3Error::Malformed as M;
    let mut top = Reader(datagram);
    let body = top.expect(0x30).ok_or(M)?;
    let mut msg = Reader(body);
    if int_of(msg.expect(0x02).ok_or(M)?) != Some(3) {
        return Err(M);
    }
    let mut global = Reader(msg.expect(0x30).ok_or(M)?);
    let msg_id = i32::try_from(int_of(global.expect(0x02).ok_or(M)?).ok_or(M)?).map_err(|_| M)?;
    let _max = global.expect(0x02).ok_or(M)?;
    let flags = *global.expect(0x04).ok_or(M)?.first().ok_or(M)?;
    let usm_bytes = msg.expect(0x04).ok_or(M)?;
    let mut usm = Reader(Reader(usm_bytes).expect(0x30).ok_or(M)?);
    let id = usm.expect(0x04).ok_or(M)?.to_vec();
    let boots = int_of(usm.expect(0x02).ok_or(M)?).ok_or(M)?;
    let time = int_of(usm.expect(0x02).ok_or(M)?).ok_or(M)?;
    let user = usm.expect(0x04).ok_or(M)?.to_vec();
    let mac = usm.expect(0x04).ok_or(M)?;
    let salt = usm.expect(0x04).ok_or(M)?;
    let engine = Engine {
        id,
        boots,
        time,
        at: Some(std::time::Instant::now()),
    };

    if flags & FLAG_AUTH != 0 {
        let (proto, key) = keys
            .and_then(|k| k.auth.as_ref())
            .ok_or(V3Error::WrongDigest)?;
        if mac.len() != proto.mac_len() {
            return Err(V3Error::WrongDigest);
        }
        // The MAC is computed with its own field zeroed: find it by address in the buffer.
        let at = mac.as_ptr() as usize - datagram.as_ptr() as usize;
        let mut zeroed = datagram.to_vec();
        zeroed[at..at + mac.len()].fill(0);
        let want = proto.hmac(key, &zeroed);
        if want.len() != mac.len() || want.iter().zip(mac).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
            return Err(V3Error::WrongDigest);
        }
    }
    let scoped_owned;
    let scoped: &[u8] = if flags & FLAG_PRIV != 0 {
        let cipher = msg.expect(0x04).ok_or(M)?;
        let (proto, key) = keys
            .and_then(|k| k.privacy.as_ref())
            .ok_or(V3Error::Decryption)?;
        scoped_owned = match proto {
            PrivProto::Aes => {
                let mut iv = [0u8; 16];
                iv[..4].copy_from_slice(&(boots as u32).to_be_bytes());
                iv[4..8].copy_from_slice(&(time as u32).to_be_bytes());
                iv[8..].copy_from_slice(salt.get(..8).ok_or(V3Error::Decryption)?);
                aes_cfb(key, iv, cipher, true).ok_or(V3Error::Decryption)?
            }
            PrivProto::Des => {
                let pre: [u8; 8] = key
                    .get(8..16)
                    .ok_or(V3Error::Decryption)?
                    .try_into()
                    .map_err(|_| V3Error::Decryption)?;
                let s: [u8; 8] = salt.try_into().map_err(|_| V3Error::Decryption)?;
                let iv: [u8; 8] = std::array::from_fn(|i| pre[i] ^ s[i]);
                des_cbc(key, iv, cipher, true).ok_or(V3Error::Decryption)?
            }
        };
        &scoped_owned
    } else {
        let (tag, _) = Reader(msg.0).tlv().ok_or(M)?;
        if tag != 0x30 {
            return Err(M);
        }
        msg.0
    };
    // The decrypted bytes may carry padding after the sequence.
    let mut s = Reader(Reader(scoped).expect(0x30).ok_or(V3Error::Decryption)?);
    let _context_engine = s.expect(0x04).ok_or(M)?;
    let _context = s.expect(0x04).ok_or(M)?;
    let pdu = decode_pdu(s.0).ok_or(M)?;
    Ok(V3Message {
        msg_id,
        flags,
        engine,
        user,
        pdu,
    })
}

#[cfg(test)]
mod tests {
    use super::super::ber::{PduKind, Value};
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn rfc3414_key_localisation_vectors() {
        // RFC 3414 A.3: password "maplesyrup", engine 00…02.
        let engine = [0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        let md5 = localize(
            AuthProto::Md5,
            &password_to_key(AuthProto::Md5, b"maplesyrup"),
            &engine,
        );
        assert_eq!(hex(&md5), "526f5eed9fcce26f8964c2930787d82b");
        let sha = localize(
            AuthProto::Sha,
            &password_to_key(AuthProto::Sha, b"maplesyrup"),
            &engine,
        );
        assert_eq!(hex(&sha), "6695febc9288e36282235fc7151f128497b38f3f");
    }

    #[test]
    fn hmac_md5_matches_rfc2104() {
        // RFC 2104 test 2: key "Jefe", data "what do ya want for nothing?".
        let full = {
            let p = AuthProto::Md5;
            let mut k = [0u8; 64];
            k[..4].copy_from_slice(b"Jefe");
            let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
            inner.extend_from_slice(b"what do ya want for nothing?");
            let ih = md5::compute(&inner);
            let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
            outer.extend_from_slice(&ih.0);
            let _ = p;
            md5::compute(&outer).0
        };
        assert_eq!(hex(&full), "750c783e6ab0b503eaa86e310a5db738");
        assert_eq!(
            hex(&AuthProto::Md5.hmac(b"Jefe", b"what do ya want for nothing?")),
            "750c783e6ab0b503eaa86e31"
        );
    }

    #[test]
    fn messages_round_trip_with_every_protocol() {
        let engine = Engine {
            id: b"\x80\x00\x1f\x88\x04kenshell".to_vec(),
            boots: 7,
            time: 12345,
            at: None,
        };
        let pdu = Message {
            community: Vec::new(),
            kind: PduKind::Get,
            request_id: 99,
            a: 0,
            b: 0,
            bindings: vec![(vec![1, 3, 6, 1, 2, 1, 1, 5, 0], Value::Null)],
        };
        for (auth, privacy) in [
            (None, None),
            (Some(AuthProto::Md5), None),
            (Some(AuthProto::Sha), Some(PrivProto::Aes)),
            (Some(AuthProto::Sha256), Some(PrivProto::Aes)),
            (Some(AuthProto::Md5), Some(PrivProto::Des)),
            (Some(AuthProto::Sha), Some(PrivProto::Des)),
        ] {
            let user = V3User {
                user: "noc".into(),
                auth,
                auth_pass: Zeroizing::new("auth-pass-123".into()),
                privacy,
                priv_pass: Zeroizing::new("priv-pass-456".into()),
            };
            let keys = user.keys(&engine.id);
            let flags = FLAG_REPORTABLE
                | if auth.is_some() { FLAG_AUTH } else { 0 }
                | if privacy.is_some() { FLAG_PRIV } else { 0 };
            let wire = encode(5, flags, &engine, b"noc", Some(&keys), 0xdead_beef, &pdu).unwrap();
            if privacy.is_some() {
                // The OID is not visible in the clear.
                assert!(
                    !wire.windows(3).any(|w| w == [0x2b, 6, 1]),
                    "{auth:?}/{privacy:?}"
                );
            }
            let back = decode(&wire, Some(&keys)).unwrap();
            assert_eq!(
                (back.msg_id, back.flags, back.user.as_slice()),
                (5, flags, &b"noc"[..])
            );
            assert_eq!(back.pdu.request_id, 99);
            assert_eq!(back.pdu.bindings, pdu.bindings);
            if auth.is_some() {
                // One flipped bit anywhere → refused.
                let mut bad = wire.clone();
                let last = bad.len() - 1;
                bad[last] ^= 1;
                assert!(decode(&bad, Some(&keys)).is_err(), "{auth:?}/{privacy:?}");
                // Another password → refused.
                let other = V3User {
                    auth_pass: Zeroizing::new("wrong".into()),
                    ..user.clone()
                };
                assert_eq!(
                    decode(&wire, Some(&other.keys(&engine.id))).unwrap_err(),
                    V3Error::WrongDigest
                );
            }
        }
    }

    #[test]
    fn garbage_never_panics() {
        for len in 0..64 {
            let junk: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let _ = decode(&junk, None);
        }
        let mut seq = vec![
            0x30, 0x10, 0x02, 0x01, 0x03, 0x30, 0x84, 0xff, 0xff, 0xff, 0xff,
        ];
        seq.extend([0; 8]);
        assert!(decode(&seq, None).is_err());
        let user = V3User {
            user: "x".into(),
            auth: Some(AuthProto::Sha),
            auth_pass: Zeroizing::new("p".repeat(8)),
            privacy: None,
            priv_pass: Zeroizing::default(),
        };
        assert!(!format!("{user:?}").contains("pppppppp"));
    }
}
