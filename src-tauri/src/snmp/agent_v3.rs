//! The test agent's SNMPv3 side: engine discovery reports, USM checks, encrypted answers.
//! Used by the tests here and, included by path, by the device simulator (`--snmp-user`).

use super::agent::{Mib, respond};
use super::ber::{Message, PduKind, Value};
use super::v3::{self, Engine, FLAG_AUTH, FLAG_PRIV, V3Error, V3User};

/// usmStats counters a Report carries (RFC 3414).
const UNKNOWN_ENGINE: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 4, 0];
const UNKNOWN_USER: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 3, 0];
const WRONG_DIGEST: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 5, 0];

fn report(engine: &Engine, msg_id: i32, request_id: i32, oid: &[u32]) -> Option<Vec<u8>> {
    let pdu = Message {
        community: Vec::new(),
        kind: PduKind::Report,
        request_id,
        a: 0,
        b: 0,
        bindings: vec![(oid.to_vec(), Value::Unsigned(1))],
    };
    v3::encode(msg_id, 0, engine, b"", None, 0, &pdu)
}

/// The response to one v3 datagram for `user` on an agent with `engine`.
pub fn answer_v3(mib: &Mib, user: &V3User, engine: &Engine, datagram: &[u8]) -> Option<Vec<u8>> {
    let keys = user.keys(&engine.id);
    match v3::decode(datagram, Some(&keys)) {
        // Discovery: no engine id yet, no security.
        Ok(m) if m.engine.id.is_empty() => {
            report(engine, m.msg_id, m.pdu.request_id, UNKNOWN_ENGINE)
        }
        Ok(m) if m.user != user.user.as_bytes() => {
            report(engine, m.msg_id, m.pdu.request_id, UNKNOWN_USER)
        }
        Ok(m) => {
            let reply = respond(mib, &m.pdu)?;
            let flags = m.flags & (FLAG_AUTH | FLAG_PRIV);
            v3::encode(
                m.msg_id,
                flags,
                engine,
                user.user.as_bytes(),
                Some(&keys),
                0x5eed,
                &reply,
            )
        }
        Err(V3Error::WrongDigest | V3Error::Decryption) => {
            report(engine, v3::peek_msg_id(datagram)?, 0, WRONG_DIGEST)
        }
        Err(V3Error::Malformed) => None,
    }
}
