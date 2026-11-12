//! SNMPv2c as a source for the topology map: a device the user has no session to can be
//! asked for its LLDP neighbours, forwarding table and ARP table. Read-only (Get /
//! GetBulk), only against hosts the user names. The community string is a secret: it
//! lives in the OS keychain and never reaches logs, errors, the database or exports.
//!
//! No dependency: the wire format is the small BER subset in [`ber`].

pub mod ber;
pub mod discover;

// A fixed-table agent: for the tests here and, included by path, the device simulator.
#[cfg(test)]
pub mod agent;

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

pub struct Client {
    socket: UdpSocket,
    community: zeroize::Zeroizing<Vec<u8>>,
    request_id: i32,
}

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
            community: zeroize::Zeroizing::new(community.as_bytes().to_vec()),
            // Not a secret, only a correlation number.
            request_id: (std::process::id() as i32 & 0x7FFF) << 8,
        })
    }

    /// One request, retried on silence. A wrong community is silence too (by design of
    /// SNMP), so a timeout says "no answer", never why.
    fn request(&mut self, kind: PduKind, b: i32, oids: &[Oid]) -> Result<Vec<(Oid, Value)>> {
        self.request_id = self.request_id.wrapping_add(1) & 0x7FFF_FFFF;
        let datagram = encode(&Message {
            community: self.community.to_vec(),
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
