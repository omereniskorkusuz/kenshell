//! Flow collector for the topology map: listens for sFlow and NetFlow / IPFIX exports and
//! remembers which addresses were seen entering which port of which exporter. Off until
//! the user turns it on (no sockets, no threads before that); only headers are looked at,
//! nothing is stored on disk.

pub mod parse;

#[cfg(test)]
pub mod testgen;

use std::collections::HashMap;
use std::net::{IpAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{AppError, ErrorCode, Result};
use crate::netscan::oui;
use parse::{Observation, Protocol, Templates};

/// Distinct (exporter, port, address) sightings kept.
pub const MAX_ENTRIES: usize = 20_000;
/// A sighting not refreshed for this long is forgotten.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// An address seen on an exporter's port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowHost {
    /// Exporter address.
    pub agent: String,
    pub if_index: u32,
    /// 12 lowercase hex digits.
    pub mac: Option<String>,
    pub ip: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlowSnapshot {
    pub running: bool,
    pub sflow_port: Option<u16>,
    pub netflow_port: Option<u16>,
    /// Datagrams received since the collector started.
    pub sflow_datagrams: u64,
    pub netflow_datagrams: u64,
    pub hosts: Vec<FlowHost>,
}

#[derive(Default)]
struct Table {
    seen: HashMap<Observation, Instant>,
    templates: Templates,
    sflow: u64,
    netflow: u64,
}

impl Table {
    fn feed(&mut self, protocol: Protocol, datagram: &[u8], from: IpAddr, now: Instant) {
        let found = match protocol {
            Protocol::Sflow => {
                self.sflow += 1;
                parse::sflow(datagram)
            }
            Protocol::Netflow => {
                self.netflow += 1;
                parse::netflow(datagram, from, &mut self.templates)
            }
        };
        for o in found {
            if self.seen.len() >= MAX_ENTRIES && !self.seen.contains_key(&o) {
                continue;
            }
            self.seen.insert(o, now);
        }
    }

    fn hosts(&mut self, now: Instant) -> Vec<FlowHost> {
        self.seen
            .retain(|_, at| now.saturating_duration_since(*at) < MAX_AGE);
        let hex = |m: &[u8; 6]| m.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut hosts: Vec<FlowHost> = self
            .seen
            .keys()
            .map(|o| {
                let mac = o.mac.as_ref().map(hex);
                FlowHost {
                    agent: o.agent.to_string(),
                    if_index: o.if_index,
                    vendor: mac.as_deref().and_then(oui::vendor).map(str::to_owned),
                    mac,
                    ip: o.ip.map(|i| i.to_string()),
                }
            })
            .collect();
        hosts.sort_by(|a, b| {
            (&a.agent, a.if_index, &a.mac, &a.ip).cmp(&(&b.agent, b.if_index, &b.mac, &b.ip))
        });
        hosts
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
    sflow_port: Option<u16>,
    netflow_port: Option<u16>,
}

/// The collector: at most one pair of listeners per app.
#[derive(Default)]
pub struct FlowCollector {
    table: Arc<Mutex<Table>>,
    running: Mutex<Option<Running>>,
}

fn listen(port: u16) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::AddrInUse {
            ErrorCode::PortBusy
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            ErrorCode::PermissionDenied
        } else {
            ErrorCode::Io
        };
        AppError::with(code, format!("UDP {port}: {e}"))
    })?;
    // The thread wakes four times a second only to notice "stop".
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| AppError::with(ErrorCode::Io, e.to_string()))?;
    Ok(socket)
}

impl FlowCollector {
    /// Starts listening (a port of 0 leaves that protocol off). Restarting with other
    /// ports replaces the listeners; what was seen so far is kept.
    pub fn start(&self, sflow_port: u16, netflow_port: u16) -> Result<()> {
        if sflow_port == 0 && netflow_port == 0 {
            return Err(AppError::invalid("no port to listen on"));
        }
        if sflow_port != 0 && sflow_port == netflow_port {
            return Err(AppError::invalid("sFlow and NetFlow need different ports"));
        }
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        let mut threads = Vec::new();
        let mut bound = [None, None];
        let wanted = [
            (Protocol::Sflow, sflow_port, "kenshell-sflow"),
            (Protocol::Netflow, netflow_port, "kenshell-netflow"),
        ];
        let mut sockets = Vec::new();
        for (i, (protocol, port, name)) in wanted.into_iter().enumerate() {
            if port == 0 {
                continue;
            }
            // Both sockets first: a failure on the second leaves nothing half started.
            let socket = listen(port)?;
            bound[i] = socket.local_addr().ok().map(|a| a.port());
            sockets.push((protocol, name, socket));
        }
        for (protocol, name, socket) in sockets {
            let (table, flag) = (Arc::clone(&self.table), Arc::clone(&stop));
            let thread = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    let mut buf = vec![0u8; 65_535];
                    while !flag.load(Ordering::Relaxed) {
                        let Ok((n, from)) = socket.recv_from(&mut buf) else {
                            continue;
                        };
                        if let Ok(mut t) = table.lock() {
                            t.feed(protocol, &buf[..n], from.ip(), Instant::now());
                        }
                    }
                })
                .map_err(|e| AppError::with(ErrorCode::Internal, e.to_string()))?;
            threads.push(thread);
        }
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = Some(Running {
            stop,
            threads,
            sflow_port: bound[0],
            netflow_port: bound[1],
        });
        Ok(())
    }

    /// Idempotent. The sockets are closed when this returns.
    pub fn stop(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(r) = running {
            r.stop.store(true, Ordering::Relaxed);
            for t in r.threads {
                let _ = t.join();
            }
        }
    }

    /// Forgets everything seen (the listeners keep running).
    pub fn clear(&self) {
        if let Ok(mut t) = self.table.lock() {
            t.seen.clear();
        }
    }

    pub fn snapshot(&self) -> FlowSnapshot {
        let (running, sflow_port, netflow_port) = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or((false, None, None), |r| {
                (true, r.sflow_port, r.netflow_port)
            });
        let mut t = self.table.lock().unwrap_or_else(|e| e.into_inner());
        FlowSnapshot {
            running,
            sflow_port,
            netflow_port,
            sflow_datagrams: t.sflow,
            netflow_datagrams: t.netflow,
            hosts: t.hosts(Instant::now()),
        }
    }
}

impl Drop for FlowCollector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::parse::{Templates, netflow, sflow};
    use super::testgen::{frame, netflow_v5, sflow_v5, template_flows};
    use super::*;
    use std::net::Ipv4Addr;

    const PC: [u8; 6] = [0x3c, 0x52, 0x82, 0x00, 0x0d, 0x05];
    const FROM: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

    #[test]
    fn sflow_samples_give_port_mac_and_address() {
        for expanded in [false, true] {
            let d = sflow_v5(
                [10, 0, 0, 12],
                &[
                    (7, frame(PC, [10, 10, 0, 77], None)),
                    (
                        8,
                        frame([0x00, 0x15, 0x9a, 0, 0x0a, 1], [10, 10, 0, 31], Some(10)),
                    ),
                    // A multicast source and input 0 are not sightings.
                    (9, frame([0x01, 0, 0x5e, 0, 0, 1], [224, 0, 0, 1], None)),
                    (0, frame(PC, [10, 10, 0, 77], None)),
                ],
                expanded,
            );
            let o = sflow(&d);
            assert_eq!(o.len(), 2, "expanded={expanded}: {o:?}");
            assert_eq!(o[0].agent.to_string(), "10.0.0.12");
            assert_eq!((o[0].if_index, o[0].mac), (7, Some(PC)));
            assert_eq!(o[0].ip, Some(Ipv4Addr::new(10, 10, 0, 77)));
            assert_eq!(
                o[1].ip,
                Some(Ipv4Addr::new(10, 10, 0, 31)),
                "VLAN tag skipped"
            );
        }
    }

    #[test]
    fn netflow_v5_v9_and_ipfix() {
        let mut t = Templates::default();
        let o = netflow(
            &netflow_v5(&[([10, 10, 0, 77], 3), ([0, 0, 0, 0], 3)]),
            FROM,
            &mut t,
        );
        assert_eq!(o.len(), 1);
        assert_eq!((o[0].agent, o[0].if_index, o[0].mac), (FROM, 3, None));

        for ipfix in [false, true] {
            let mut t = Templates::default();
            // Data before its template is unknown: nothing, no panic.
            let flows = [([10, 10, 0, 77], 12u32, PC), ([10, 10, 0, 78], 13, [0; 6])];
            assert!(netflow(&template_flows(ipfix, &flows, false), FROM, &mut t).is_empty());
            let o = netflow(&template_flows(ipfix, &flows, true), FROM, &mut t);
            assert_eq!(o.len(), 2, "ipfix={ipfix}: {o:?}");
            assert_eq!((o[0].if_index, o[0].mac), (12, Some(PC)));
            assert_eq!((o[1].if_index, o[1].mac), (13, None));
            assert_eq!(t.len(), 1);
            // The template is remembered for later packets of that exporter only.
            assert_eq!(
                netflow(&template_flows(ipfix, &flows, false), FROM, &mut t).len(),
                2
            );
            let other = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
            assert!(netflow(&template_flows(ipfix, &flows, false), other, &mut t).is_empty());
        }
    }

    #[test]
    fn hostile_datagrams_never_panic() {
        let seeds = [
            sflow_v5(
                [10, 0, 0, 1],
                &[(7, frame(PC, [10, 1, 1, 1], Some(5)))],
                false,
            ),
            sflow_v5([10, 0, 0, 1], &[(7, frame(PC, [10, 1, 1, 1], None))], true),
            netflow_v5(&[([10, 1, 1, 1], 3)]),
            template_flows(false, &[([10, 1, 1, 1], 3, PC)], true),
            template_flows(true, &[([10, 1, 1, 1], 3, PC)], true),
        ];
        let mut x: u32 = 0x1234_5678;
        let mut t = Templates::default();
        for seed in &seeds {
            for cut in 0..seed.len() {
                let _ = sflow(&seed[..cut]);
                let _ = netflow(&seed[..cut], FROM, &mut t);
            }
            for i in 0..seed.len() {
                for _ in 0..6 {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    let mut bad = seed.clone();
                    bad[i] = x as u8;
                    let _ = sflow(&bad);
                    let _ = netflow(&bad, FROM, &mut t);
                }
            }
        }
        // Huge counts and lengths in tiny datagrams.
        let _ = sflow(&[
            0, 0, 0, 5, 0, 0, 0, 1, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF,
            0xFF, 0xFF,
        ]);
        let _ = netflow(
            &[
                0, 9, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0xFF, 0xFF,
            ],
            FROM,
            &mut t,
        );
        assert!(t.len() <= 1024);
    }

    #[test]
    fn collector_listens_counts_and_forgets() {
        let c = FlowCollector::default();
        assert!(!c.snapshot().running);
        assert!(c.start(0, 0).is_err());
        // Free ports: bind to 0 is not possible through the API, so pick two.
        let free = || {
            UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        };
        let (sp, np) = (free(), free());
        c.start(sp, np).unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(
            &sflow_v5(
                [10, 0, 0, 12],
                &[(7, frame(PC, [10, 10, 0, 77], None))],
                false,
            ),
            ("127.0.0.1", sp),
        )
        .unwrap();
        tx.send_to(&netflow_v5(&[([10, 10, 0, 90], 4)]), ("127.0.0.1", np))
            .unwrap();
        tx.send_to(b"not a flow datagram", ("127.0.0.1", np))
            .unwrap();
        let start = Instant::now();
        let snap = loop {
            let s = c.snapshot();
            if (s.sflow_datagrams >= 1 && s.netflow_datagrams >= 2)
                || start.elapsed() > Duration::from_secs(5)
            {
                break s;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(snap.running);
        assert_eq!((snap.sflow_port, snap.netflow_port), (Some(sp), Some(np)));
        assert_eq!(snap.hosts.len(), 2, "{snap:?}");
        let pc = snap.hosts.iter().find(|h| h.agent == "10.0.0.12").unwrap();
        assert_eq!(
            (pc.if_index, pc.mac.as_deref(), pc.ip.as_deref()),
            (7, Some("3c5282000d05"), Some("10.10.0.77"))
        );
        assert!(
            snap.hosts
                .iter()
                .any(|h| h.agent == "127.0.0.1" && h.if_index == 4 && h.mac.is_none())
        );

        // A second collector cannot take the same port; this one releases it on stop.
        let other = FlowCollector::default();
        assert_eq!(other.start(sp, 0).unwrap_err().code, ErrorCode::PortBusy);
        c.stop();
        assert!(!c.snapshot().running);
        other.start(sp, 0).unwrap();
        other.stop();

        // Old sightings age out; the table is bounded.
        let mut t = Table::default();
        let old = Instant::now()
            .checked_sub(MAX_AGE + Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        t.feed(
            Protocol::Netflow,
            &netflow_v5(&[([10, 1, 1, 1], 3)]),
            FROM,
            old,
        );
        t.feed(
            Protocol::Netflow,
            &netflow_v5(&[([10, 1, 1, 2], 3)]),
            FROM,
            Instant::now(),
        );
        assert_eq!(t.hosts(Instant::now()).len(), 1);
        c.clear();
        assert!(c.snapshot().hosts.is_empty());
    }
}
