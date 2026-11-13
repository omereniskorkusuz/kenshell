//! SNMPv2c as a source for the topology map: a device the user has no session to can be
//! asked for its LLDP neighbours, forwarding table and ARP table. Read-only (Get /
//! GetBulk), only against hosts the user names. The community string is a secret: it
//! lives in the OS keychain and never reaches logs, errors, the database or exports.
//!
//! No dependency: the wire format is the small BER subset in [`ber`].

pub mod ber;
pub mod discover;
pub mod v3;

// A fixed-table agent: for the tests here and, included by path, the device simulator.
#[cfg(test)]
pub mod agent;
#[cfg(test)]
pub mod agent_v3;

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use ber::{Message, Oid, PduKind, Value, decode, encode};

use crate::error::{AppError, ErrorCode, Result};

pub const DEFAULT_PORT: u16 = 161;
const TIMEOUT: Duration = Duration::from_millis(1500);
const TRIES: u32 = 3;
const BULK: i32 = 25;
/// Upper bound per table: a core switch's forwarding table can be huge.
pub const MAX_ROWS: usize = 8000;

/// `host` or `host:port` → address (IPv4 first).
pub fn resolve(target: &str) -> Result<SocketAddr> {
    let t = target.trim();
    let with_port = if t
        .rsplit_once(':')
        .is_some_and(|(h, p)| !h.contains(':') && p.parse::<u16>().is_ok())
    {
        t.to_owned()
    } else {
        format!("{t}:{DEFAULT_PORT}")
    };
    let mut addrs: Vec<SocketAddr> = with_port
        .to_socket_addrs()
        .map_err(|_| AppError::with(ErrorCode::HostNotFound, t.to_owned()))?
        .collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs
        .into_iter()
        .next()
        .ok_or_else(|| AppError::with(ErrorCode::HostNotFound, t.to_owned()))
}

/// How the client proves who it is.
enum Security {
    V2c(zeroize::Zeroizing<Vec<u8>>),
    V3 {
        user: Box<v3::V3User>,
        /// Learned from the agent's first answer (discovery).
        engine: Option<v3::Engine>,
        keys: Option<v3::Keys>,
    },
}

pub struct Client {
    socket: UdpSocket,
    security: Security,
    request_id: i32,
    salt: u64,
}

/// Sends one datagram and returns the first answer `accept` takes (late answers to an
/// earlier try are skipped). Retried on silence.
fn exchange_on<T>(
    socket: &UdpSocket,
    datagram: &[u8],
    mut accept: impl FnMut(&[u8]) -> Option<T>,
) -> Result<T> {
    let mut buf = vec![0u8; 65_535];
    for _ in 0..TRIES {
        socket
            .send(datagram)
            .map_err(|e| AppError::with(ErrorCode::Io, e.to_string()))?;
        for _ in 0..8 {
            let Ok(n) = socket.recv(&mut buf) else {
                break;
            };
            if let Some(t) = accept(&buf[..n]) {
                return Ok(t);
            }
        }
    }
    Err(AppError::with(ErrorCode::Timeout, "no SNMP answer"))
}

/// usmStats counters of a v3 Report (RFC 3414).
const USM_NOT_IN_TIME: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 2, 0];
const USM_UNKNOWN_USER: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 3, 0];
const USM_WRONG_DIGEST: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 5, 0];
const USM_DECRYPTION: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 6, 0];
const USM_LEVEL: &[u32] = &[1, 3, 6, 1, 6, 3, 15, 1, 1, 1, 0];

impl Client {
    pub fn connect(addr: SocketAddr, community: &str) -> Result<Self> {
        let bind = if addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket =
            UdpSocket::bind(bind).map_err(|e| AppError::with(ErrorCode::Io, e.to_string()))?;
        socket
            .connect(addr)
            .and_then(|()| socket.set_read_timeout(Some(TIMEOUT)))
            .map_err(|e| AppError::with(ErrorCode::Io, e.to_string()))?;
        Ok(Self {
            socket,
            security: Security::V2c(zeroize::Zeroizing::new(community.as_bytes().to_vec())),
            // Not a secret, only a correlation number.
            request_id: (std::process::id() as i32 & 0x7FFF) << 8,
            salt: 0,
        })
    }

    /// SNMPv3 with the given user (authentication / privacy as configured).
    pub fn connect_v3(addr: SocketAddr, user: v3::V3User) -> Result<Self> {
        let mut c = Self::connect(addr, "")?;
        // Per-message IV salt: a fresh start per client (not a secret, must not repeat).
        c.salt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        c.security = Security::V3 {
            user: Box::new(user),
            engine: None,
            keys: None,
        };
        Ok(c)
    }

    fn request_v3(&mut self, pdu: &Message) -> Result<Vec<(Oid, Value)>> {
        // Engine discovery (unauthenticated Report with the agent's engine id and clock).
        if matches!(&self.security, Security::V3 { engine: None, .. }) {
            self.request_id = self.request_id.wrapping_add(1) & 0x7FFF_FFFF;
            let id = self.request_id;
            let probe = Message {
                community: Vec::new(),
                kind: PduKind::Get,
                request_id: id,
                a: 0,
                b: 0,
                bindings: Vec::new(),
            };
            let wire = v3::encode(
                id,
                v3::FLAG_REPORTABLE,
                &v3::Engine::default(),
                b"",
                None,
                0,
                &probe,
            )
            .ok_or_else(|| AppError::internal("v3 encode"))?;
            let engine = exchange_on(&self.socket, &wire, |d| {
                v3::decode(d, None)
                    .ok()
                    .filter(|m| m.msg_id == id && !m.engine.id.is_empty())
                    .map(|m| m.engine)
            })?;
            if let Security::V3 {
                user,
                engine: e,
                keys,
            } = &mut self.security
            {
                *keys = Some(user.keys(&engine.id));
                *e = Some(engine);
            }
        }
        for _ in 0..2 {
            self.request_id = self.request_id.wrapping_add(1) & 0x7FFF_FFFF;
            self.salt = self.salt.wrapping_add(1);
            let (id, salt) = (self.request_id, self.salt);
            let Security::V3 {
                user,
                engine: Some(engine),
                keys: Some(keys),
            } = &self.security
            else {
                return Err(AppError::internal("v3 state"));
            };
            let flags = v3::FLAG_REPORTABLE
                | if user.auth.is_some() {
                    v3::FLAG_AUTH
                } else {
                    0
                }
                | if user.auth.is_some() && user.privacy.is_some() {
                    v3::FLAG_PRIV
                } else {
                    0
                };
            let mut m = pdu.clone();
            m.request_id = id;
            let wire = v3::encode(
                id,
                flags,
                engine,
                user.user.as_bytes(),
                Some(keys),
                salt,
                &m,
            )
            .ok_or_else(|| AppError::internal("v3 encode"))?;
            let reply = exchange_on(&self.socket, &wire, |d| match v3::decode(d, Some(keys)) {
                Ok(r) if r.msg_id == id => Some(Ok(r)),
                Ok(_) => None,
                // A refused answer: say so (decode errors never contain secrets).
                Err(e) if v3::peek_msg_id(d) == Some(id) => Some(Err(e)),
                Err(_) => None,
            })?;
            let r = reply
                .map_err(|e| AppError::with(ErrorCode::AuthFailed, format!("SNMPv3: {e:?}")))?;
            if r.pdu.kind == PduKind::Report {
                let oid = r
                    .pdu
                    .bindings
                    .first()
                    .map(|(o, _)| o.clone())
                    .unwrap_or_default();
                if oid == USM_NOT_IN_TIME {
                    // The agent's clock: take it and try once more.
                    if let Security::V3 {
                        engine: Some(e), ..
                    } = &mut self.security
                    {
                        e.boots = r.engine.boots;
                        e.time = r.engine.time;
                        e.at = r.engine.at;
                    }
                    continue;
                }
                let why = match oid.as_slice() {
                    o if o == USM_UNKNOWN_USER => "unknown user",
                    o if o == USM_WRONG_DIGEST => "wrong authentication password",
                    o if o == USM_DECRYPTION => "wrong privacy password",
                    o if o == USM_LEVEL => "security level not supported for this user",
                    _ => "refused",
                };
                return Err(AppError::with(
                    ErrorCode::AuthFailed,
                    format!("SNMPv3: {why}"),
                ));
            }
            if r.pdu.a != 0 {
                return Err(AppError::with(
                    ErrorCode::Unsupported,
                    format!("agent error {}", r.pdu.a),
                ));
            }
            return Ok(r.pdu.bindings);
        }
        Err(AppError::with(
            ErrorCode::Timeout,
            "SNMPv3: clock not in time window",
        ))
    }

    /// One request, retried on silence. A wrong community is silence too (by design of
    /// SNMP), so a timeout says "no answer", never why.
    fn request(&mut self, kind: PduKind, b: i32, oids: &[Oid]) -> Result<Vec<(Oid, Value)>> {
        if matches!(self.security, Security::V3 { .. }) {
            return self.request_v3(&Message {
                community: Vec::new(),
                kind,
                request_id: 0,
                a: 0,
                b,
                bindings: oids.iter().map(|o| (o.clone(), Value::Null)).collect(),
            });
        }
        let Security::V2c(community) = &self.security else {
            return Err(AppError::internal("v2c state"));
        };
        self.request_id = self.request_id.wrapping_add(1) & 0x7FFF_FFFF;
        let datagram = encode(&Message {
            community: community.to_vec(),
            kind,
            request_id: self.request_id,
            a: 0,
            b,
            bindings: oids.iter().map(|o| (o.clone(), Value::Null)).collect(),
        });
        let mut buf = vec![0u8; 65_535];
        for _ in 0..TRIES {
            self.socket
                .send(&datagram)
                .map_err(|e| AppError::with(ErrorCode::Io, e.to_string()))?;
            // Skip stray datagrams (late answers to an earlier try).
            for _ in 0..8 {
                let Ok(n) = self.socket.recv(&mut buf) else {
                    break;
                };
                let Some(m) = decode(&buf[..n]) else {
                    continue;
                };
                if m.kind != PduKind::Response || m.request_id != self.request_id {
                    continue;
                }
                if m.a != 0 {
                    return Err(AppError::with(
                        ErrorCode::Unsupported,
                        format!("agent error {}", m.a),
                    ));
                }
                return Ok(m.bindings);
            }
        }
        Err(AppError::with(ErrorCode::Timeout, "no SNMP answer"))
    }

    pub fn get(&mut self, oids: &[Oid]) -> Result<Vec<(Oid, Value)>> {
        self.request(PduKind::Get, 0, oids)
    }

    /// Every value under `root`, in order (at most [`MAX_ROWS`]).
    pub fn walk(&mut self, root: &[u32]) -> Result<Vec<(Oid, Value)>> {
        let mut out = Vec::new();
        let mut cur: Oid = root.to_vec();
        loop {
            let batch = self.request(PduKind::GetBulk, BULK, std::slice::from_ref(&cur))?;
            if batch.is_empty() {
                return Ok(out);
            }
            for (oid, value) in batch {
                // Done at the end of the subtree; a non-increasing OID is a broken agent
                // (would loop forever).
                if value == Value::Missing || !oid.starts_with(root) || oid <= cur {
                    return Ok(out);
                }
                cur.clone_from(&oid);
                out.push((oid, value));
                if out.len() >= MAX_ROWS {
                    return Ok(out);
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::agent::{Mib, serve};
    use std::net::{SocketAddr, UdpSocket};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A running agent on a free local port; stops when dropped.
    pub struct Agent {
        pub addr: SocketAddr,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Agent {
        /// An SNMPv3 agent for `user` (engine id fixed, clock from now).
        pub fn start_v3(mib: Mib, user: super::v3::V3User) -> Self {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let addr = socket.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                let engine = super::v3::Engine {
                    id: b"\x80\x00\x1f\x88\x04test-agent".to_vec(),
                    boots: 3,
                    time: 1000,
                    at: Some(std::time::Instant::now()),
                };
                let _ = socket.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                let mut buf = [0u8; 4096];
                while !flag.load(Ordering::Relaxed) {
                    let Ok((n, from)) = socket.recv_from(&mut buf) else {
                        continue;
                    };
                    if let Some(r) = super::agent_v3::answer_v3(&mib, &user, &engine, &buf[..n]) {
                        let _ = socket.send_to(&r, from);
                    }
                }
            });
            Self {
                addr,
                stop,
                thread: Some(thread),
            }
        }

        pub fn start(mib: Mib, community: &'static str) -> Self {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            let addr = socket.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                serve(&socket, &mib, community.as_bytes(), &flag);
            });
            Self {
                addr,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Agent {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_walk_and_wrong_community() {
        let mut mib = agent::Mib::new();
        for i in 1..=120u32 {
            mib.insert(
                vec![1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1, i],
                Value::Bytes(format!("Gi0/{i}").into_bytes()),
            );
        }
        mib.insert(
            vec![1, 3, 6, 1, 2, 1, 1, 5, 0],
            Value::Bytes(b"sw1".to_vec()),
        );
        mib.insert(vec![1, 3, 6, 1, 2, 1, 99, 1], Value::Int(1));
        let a = testing::Agent::start(mib, "s3cret");

        let mut c = Client::connect(a.addr, "s3cret").unwrap();
        let got = c
            .get(&[vec![1, 3, 6, 1, 2, 1, 1, 5, 0], vec![1, 3, 9]])
            .unwrap();
        assert_eq!(got[0].1.text().as_deref(), Some("sw1"));
        assert_eq!(got[1].1, Value::Missing);
        // More rows than one GetBulk carries; stops at the end of the subtree.
        let names = c.walk(&[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1]).unwrap();
        assert_eq!(names.len(), 120);
        assert_eq!(names[119].1.text().as_deref(), Some("Gi0/120"));
        assert!(c.walk(&[1, 3, 6, 1, 2, 1, 50]).unwrap().is_empty());

        // A wrong community gets no answer: a timeout whose text does not contain it.
        let mut bad = Client::connect(a.addr, "wr0ng-community").unwrap();
        bad.socket
            .set_read_timeout(Some(Duration::from_millis(60)))
            .unwrap();
        let e = bad.get(&[vec![1, 3, 6, 1, 2, 1, 1, 5, 0]]).unwrap_err();
        assert_eq!(e.code, ErrorCode::Timeout);
        assert!(!format!("{e:?}").contains("wr0ng"));
    }

    #[test]
    fn snmpv3_get_walk_and_refusals() {
        use super::v3::{AuthProto, PrivProto, V3User};
        use zeroize::Zeroizing;
        let mut mib = agent::Mib::new();
        for i in 1..=60u32 {
            mib.insert(
                vec![1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1, i],
                Value::Bytes(format!("Gi0/{i}").into_bytes()),
            );
        }
        mib.insert(
            vec![1, 3, 6, 1, 2, 1, 1, 5, 0],
            Value::Bytes(b"sw-v3".to_vec()),
        );
        for (auth, privacy) in [
            (Some(AuthProto::Sha), Some(PrivProto::Aes)),
            (Some(AuthProto::Sha256), Some(PrivProto::Aes)),
            (Some(AuthProto::Md5), Some(PrivProto::Des)),
            (Some(AuthProto::Sha), None),
        ] {
            let user = V3User {
                user: "noc-ro".into(),
                auth,
                auth_pass: Zeroizing::new("Auth#Pass-2026".into()),
                privacy,
                priv_pass: Zeroizing::new("Priv#Pass-2026".into()),
            };
            let a = testing::Agent::start_v3(mib.clone(), user.clone());
            let mut c = Client::connect_v3(a.addr, user.clone()).unwrap();
            let got = c.get(&[vec![1, 3, 6, 1, 2, 1, 1, 5, 0]]).unwrap();
            assert_eq!(
                got[0].1.text().as_deref(),
                Some("sw-v3"),
                "{auth:?}/{privacy:?}"
            );
            assert_eq!(
                c.walk(&[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1]).unwrap().len(),
                60
            );

            // Wrong password: a clear refusal, without the password in it.
            let wrong = V3User {
                auth_pass: Zeroizing::new("nope-nope".into()),
                ..user.clone()
            };
            let mut c = Client::connect_v3(a.addr, wrong).unwrap();
            c.socket
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let e = c.get(&[vec![1, 3, 6, 1, 2, 1, 1, 5, 0]]).unwrap_err();
            assert_eq!(e.code, ErrorCode::AuthFailed, "{auth:?}/{privacy:?}: {e:?}");
            assert!(!format!("{e:?}").contains("nope"));
            // Unknown user.
            let other = V3User {
                user: "someone".into(),
                ..user.clone()
            };
            let e = Client::connect_v3(a.addr, other)
                .unwrap()
                .get(&[vec![1, 3]])
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::AuthFailed);
        }
    }

    #[test]
    fn targets() {
        assert_eq!(resolve("127.0.0.1").unwrap().port(), 161);
        assert_eq!(resolve(" 127.0.0.1:16161 ").unwrap().port(), 16161);
        assert_eq!(resolve("::1").unwrap().port(), 161);
        assert_eq!(
            resolve("no such host.invalid").unwrap_err().code,
            ErrorCode::HostNotFound
        );
    }
}
