//! Discovery engine (P4-T13): which addresses in an in-scope range answer, with their open
//! management ports, MAC and host name. Discovery only — ICMP echo, the OS ARP table, a
//! plain TCP connect to [`scope::PORTS`] and reverse DNS; at most [`scope::CONCURRENCY`]
//! probes at once.
//!
//! Stage A: ICMP sweep alongside TCP connects to two ports on every address (this also
//! makes the OS resolve each address, filling the ARP table). Stage B: the ARP table.
//! Stage C: for live hosts, the remaining ports and reverse DNS.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use super::scope;

/// Ports tried on every address in stage A (the rest only on live hosts).
pub const FIRST_PORTS: [u16; 2] = [22, 443];
const CONNECT_TIMEOUT: Duration = Duration::from_millis(700);
const ICMP_WAIT: Duration = Duration::from_millis(1500);
const DNS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tcp {
    /// Connected: the port is open.
    Open,
    /// Refused (RST): the host is there, the port is closed.
    Closed,
    /// Nothing within the time-out (no host, or filtered).
    Silent,
}

#[async_trait::async_trait]
pub trait Prober: Send + Sync {
    async fn tcp(&self, ip: Ipv4Addr, port: u16) -> Tcp;
    /// None when ICMP is unavailable to this user / OS.
    async fn icmp(
        &self,
        hosts: Vec<Ipv4Addr>,
        cancel: Arc<AtomicBool>,
    ) -> Option<HashSet<Ipv4Addr>>;
    async fn arp(&self) -> Vec<(Ipv4Addr, String)>;
    async fn rdns(&self, ip: Ipv4Addr) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub ip: Ipv4Addr,
    pub alive: bool,
    /// Answered ping.
    pub icmp: bool,
    pub open_ports: Vec<u16>,
    pub mac: Option<String>,
    /// From the MAC's OUI (P4-T15).
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    /// This computer.
    pub own: bool,
    pub gateway: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Sweep,
    Arp,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: Stage,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub hosts: Vec<Host>,
    /// ICMP could be used (otherwise TCP + ARP only).
    pub icmp_available: bool,
    pub cancelled: bool,
}

/// Probes every address of `range` (already checked by [`scope::check`]).
pub async fn run(
    range: scope::Range,
    own: Ipv4Addr,
    gateway: Option<Ipv4Addr>,
    prober: Arc<dyn Prober>,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(Progress) + Send + Sync,
) -> ScanResult {
    let hosts: Vec<Ipv4Addr> = range.hosts().collect();
    let total = hosts.len() as u64;
    let limit = Arc::new(Semaphore::new(scope::CONCURRENCY));
    let stopped = || cancel.load(Ordering::Relaxed);

    // ---- stage A: ICMP sweep + first TCP ports on every address
    let icmp_task = {
        let (p, h, c) = (Arc::clone(&prober), hosts.clone(), Arc::clone(&cancel));
        tokio::spawn(async move { p.icmp(h, c).await })
    };
    let mut tcp: HashMap<Ipv4Addr, Vec<(u16, Tcp)>> = HashMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    for ip in &hosts {
        for port in FIRST_PORTS {
            let (p, l, c, ip) = (
                Arc::clone(&prober),
                Arc::clone(&limit),
                Arc::clone(&cancel),
                *ip,
            );
            tasks.spawn(async move {
                let Ok(_permit) = l.acquire_owned().await else {
                    return (ip, port, Tcp::Silent);
                };
                if c.load(Ordering::Relaxed) {
                    return (ip, port, Tcp::Silent);
                }
                (ip, port, p.tcp(ip, port).await)
            });
        }
    }
    let mut finished: HashMap<Ipv4Addr, usize> = HashMap::new();
    let mut done = 0u64;
    while let Some(r) = tasks.join_next().await {
        let Ok((ip, port, outcome)) = r else { continue };
        tcp.entry(ip).or_default().push((port, outcome));
        let n = finished.entry(ip).or_default();
        *n += 1;
        if *n == FIRST_PORTS.len() {
            done += 1;
            progress(Progress {
                stage: Stage::Sweep,
                done,
                total,
            });
        }
    }
    let icmp = icmp_task.await.ok().flatten();
    let icmp_available = icmp.is_some();
    let icmp = icmp.unwrap_or_default();

    // ---- stage B: ARP table (filled by the probes above)
    progress(Progress {
        stage: Stage::Arp,
        done: 0,
        total: 1,
    });
    let arp: HashMap<Ipv4Addr, String> = if stopped() {
        HashMap::new()
    } else {
        prober.arp().await.into_iter().collect()
    };

    let in_range = |ip: &Ipv4Addr| {
        u32::from(*ip) >= u32::from(range.first) && u32::from(*ip) <= u32::from(range.last)
    };
    let mut result: Vec<Host> = hosts
        .iter()
        .map(|ip| {
            let outcomes = tcp.get(ip).cloned().unwrap_or_default();
            let tcp_alive = outcomes.iter().any(|(_, o)| *o != Tcp::Silent);
            Host {
                ip: *ip,
                alive: *ip == own || icmp.contains(ip) || tcp_alive || arp.contains_key(ip),
                icmp: icmp.contains(ip),
                open_ports: outcomes
                    .iter()
                    .filter(|(_, o)| *o == Tcp::Open)
                    .map(|(p, _)| *p)
                    .collect(),
                mac: arp.get(ip).cloned(),
                vendor: arp
                    .get(ip)
                    .and_then(|m| super::oui::vendor(m))
                    .map(str::to_owned),
                hostname: None,
                own: *ip == own,
                gateway: gateway == Some(*ip),
            }
        })
        .collect();
    // The gateway is alive even if it hides from every probe (ARP may still know it).
    for h in &mut result {
        if h.gateway && in_range(&h.ip) && arp.contains_key(&h.ip) {
            h.alive = true;
        }
    }

    // ---- stage C: remaining ports + reverse DNS on live hosts
    if !stopped() {
        let live: Vec<Ipv4Addr> = result.iter().filter(|h| h.alive).map(|h| h.ip).collect();
        let rest: Vec<u16> = scope::PORTS
            .iter()
            .copied()
            .filter(|p| !FIRST_PORTS.contains(p))
            .collect();
        let total = live.len() as u64;
        let mut tasks = tokio::task::JoinSet::new();
        for ip in &live {
            let (p, l, c, ip, ports) = (
                Arc::clone(&prober),
                Arc::clone(&limit),
                Arc::clone(&cancel),
                *ip,
                rest.clone(),
            );
            tasks.spawn(async move {
                let Ok(_permit) = l.acquire_owned().await else {
                    return (ip, Vec::new(), None);
                };
                if c.load(Ordering::Relaxed) {
                    return (ip, Vec::new(), None);
                }
                let dns = tokio::time::timeout(DNS_TIMEOUT, p.rdns(ip));
                let probes = futures_lite::future::zip(
                    async {
                        let mut open = Vec::new();
                        for port in &ports {
                            if p.tcp(ip, *port).await == Tcp::Open {
                                open.push(*port);
                            }
                        }
                        open
                    },
                    dns,
                );
                let (open, name) = probes.await;
                (ip, open, name.ok().flatten())
            });
        }
        let mut done = 0u64;
        while let Some(r) = tasks.join_next().await {
            let Ok((ip, open, name)) = r else { continue };
            if let Some(h) = result.iter_mut().find(|h| h.ip == ip) {
                h.open_ports.extend(open);
                h.open_ports.sort_unstable();
                h.hostname = name;
            }
            done += 1;
            progress(Progress {
                stage: Stage::Details,
                done,
                total,
            });
        }
    }
    ScanResult {
        hosts: result,
        icmp_available,
        cancelled: stopped(),
    }
}

/// The real probes.
pub struct NetProber;

#[async_trait::async_trait]
impl Prober for NetProber {
    async fn tcp(&self, ip: Ipv4Addr, port: u16) -> Tcp {
        let addr = std::net::SocketAddr::from((ip, port));
        match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_stream)) => Tcp::Open,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => Tcp::Closed,
            _ => Tcp::Silent,
        }
    }

    async fn icmp(
        &self,
        hosts: Vec<Ipv4Addr>,
        cancel: Arc<AtomicBool>,
    ) -> Option<HashSet<Ipv4Addr>> {
        tokio::task::spawn_blocking(move || {
            crate::platform::net::icmp_sweep(&hosts, ICMP_WAIT, &cancel)
        })
        .await
        .ok()
        .flatten()
    }

    async fn arp(&self) -> Vec<(Ipv4Addr, String)> {
        tokio::task::spawn_blocking(crate::platform::net::arp_table)
            .await
            .unwrap_or_default()
    }

    async fn rdns(&self, ip: Ipv4Addr) -> Option<String> {
        tokio::task::spawn_blocking(move || crate::platform::net::reverse_dns(ip))
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// .1 gateway (ARP only, no ports), .2 SSH open, .3 refuses everything, .4 pings,
    /// .5 web server, the rest silent.
    struct Fake {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        probed: Mutex<Vec<(Ipv4Addr, u16)>>,
    }

    fn ip(n: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 1, n)
    }

    #[async_trait::async_trait]
    impl Prober for Fake {
        async fn tcp(&self, a: Ipv4Addr, port: u16) -> Tcp {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(2)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.probed.lock().unwrap().push((a, port));
            match (a.octets()[3], port) {
                (2, 22) => Tcp::Open,
                (3, _) => Tcp::Closed,
                (5, 80 | 443) => Tcp::Open,
                _ => Tcp::Silent,
            }
        }
        async fn icmp(&self, _: Vec<Ipv4Addr>, _: Arc<AtomicBool>) -> Option<HashSet<Ipv4Addr>> {
            Some([ip(4)].into())
        }
        async fn arp(&self) -> Vec<(Ipv4Addr, String)> {
            vec![
                (ip(1), "aa:bb:cc:dd:ee:01".into()),
                (ip(2), "aa:bb:cc:dd:ee:02".into()),
            ]
        }
        async fn rdns(&self, a: Ipv4Addr) -> Option<String> {
            (a == ip(2)).then(|| "sw1.lab".to_owned())
        }
    }

    fn fake() -> Arc<Fake> {
        Arc::new(Fake {
            in_flight: 0.into(),
            max_in_flight: 0.into(),
            probed: Mutex::new(Vec::new()),
        })
    }

    #[tokio::test]
    async fn live_hosts_ports_mac_and_names() {
        let f = fake();
        let range = scope::Range {
            first: ip(1),
            last: ip(254),
        };
        let seen = Mutex::new(Vec::new());
        let r = run(
            range,
            ip(34),
            Some(ip(1)),
            f.clone(),
            Arc::new(AtomicBool::new(false)),
            |p| {
                seen.lock().unwrap().push(p);
            },
        )
        .await;
        let alive: Vec<u8> = r
            .hosts
            .iter()
            .filter(|h| h.alive)
            .map(|h| h.ip.octets()[3])
            .collect();
        assert_eq!(alive, [1, 2, 3, 4, 5, 34]);
        let h = |n: u8| r.hosts.iter().find(|h| h.ip == ip(n)).unwrap();
        assert!(h(1).gateway && h(1).mac.is_some() && h(1).open_ports.is_empty());
        assert_eq!(h(2).open_ports, [22]);
        assert_eq!(h(2).hostname.as_deref(), Some("sw1.lab"));
        assert!(h(3).open_ports.is_empty(), "refused = alive, not open");
        assert!(h(4).icmp);
        assert_eq!(h(5).open_ports, [80, 443]);
        assert!(h(34).own);
        assert!(r.icmp_available && !r.cancelled);
        // Only the scope's ports; the remaining ones only on live hosts.
        let probed = f.probed.lock().unwrap().clone();
        assert!(probed.iter().all(|(_, p)| scope::PORTS.contains(p)));
        assert!(!probed.iter().any(|(a, p)| *p == 3389 && *a == ip(100)));
        assert!(f.max_in_flight.load(Ordering::SeqCst) <= scope::CONCURRENCY);
        let s = seen.lock().unwrap();
        assert_eq!(
            s.iter().rfind(|p| p.stage == Stage::Sweep).unwrap().done,
            254
        );
    }

    #[tokio::test]
    async fn cancel_stops_early() {
        let f = fake();
        let cancel = Arc::new(AtomicBool::new(true));
        let r = run(
            scope::Range {
                first: ip(1),
                last: ip(254),
            },
            ip(34),
            None,
            f.clone(),
            cancel,
            |_| {},
        )
        .await;
        assert!(r.cancelled);
        assert!(
            f.probed.lock().unwrap().is_empty(),
            "nothing probed after cancel"
        );
    }
}
