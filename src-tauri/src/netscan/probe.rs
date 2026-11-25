//! Discovery engine (P4-T13, R-T24): which addresses in an in-scope range answer, with
//! their response time, open management ports, MAC and name. Discovery only — ICMP echo,
//! the OS ARP table, a plain TCP connect to [`scope::PORTS`], reverse DNS and NetBIOS.
//!
//! Stage A: an ICMP sweep (one socket, paced; the system `ping` when no ICMP socket can
//! be opened), and for ranges up to [`scope::FULL_TCP_LIMIT`] TCP connects to two ports
//! on every address too (finds devices that ignore ping). Stage B: the ARP table (only
//! for a connected subnet: a routed one is behind the gateway). Stage C: for live hosts,
//! the other ports and the name.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use super::scope;

/// Ports tried on every address in stage A (the rest only on live hosts).
pub const FIRST_PORTS: [u16; 2] = [22, 443];
const CONNECT_TIMEOUT: Duration = Duration::from_millis(700);
const ICMP_WAIT: Duration = Duration::from_millis(1500);
const PING_TIMEOUT: Duration = Duration::from_secs(1);
/// Parallel system `ping` processes in the fallback.
pub const PING_PARALLEL: usize = 128;
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
const NETBIOS_TIMEOUT: Duration = Duration::from_millis(700);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tcp {
    /// Connected: the port is open.
    Open,
    /// Refused (RST): the host is there, the port is closed.
    Closed,
    /// Nothing within the time-out (no host, or filtered).
    Silent,
}

/// Told how many echo requests went out so far.
pub type SentFn = Arc<dyn Fn(u64) + Send + Sync>;

#[async_trait::async_trait]
pub trait Prober: Send + Sync {
    async fn tcp(&self, ip: Ipv4Addr, port: u16) -> Tcp;
    /// One ICMP socket for all hosts. None when ICMP sockets are unavailable to this
    /// user / OS (then [`Prober::ping`] is used per host).
    async fn icmp(
        &self,
        hosts: Vec<Ipv4Addr>,
        cancel: Arc<AtomicBool>,
        sent: SentFn,
    ) -> Option<HashMap<Ipv4Addr, Duration>>;
    /// One echo through the system ping.
    async fn ping(&self, ip: Ipv4Addr) -> Option<Duration>;
    async fn arp(&self) -> Vec<(Ipv4Addr, String)>;
    async fn rdns(&self, ip: Ipv4Addr) -> Option<String>;
    async fn netbios(&self, ip: Ipv4Addr) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Reverse DNS for live hosts (NetBIOS is asked when there is no DNS name).
    pub resolve_dns: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub ip: Ipv4Addr,
    /// Answered ping, with the round-trip time in milliseconds.
    pub rtt_ms: Option<f64>,
    pub open_ports: Vec<u16>,
    pub mac: Option<String>,
    /// From the MAC's OUI (P4-T15).
    pub vendor: Option<String>,
    /// Reverse DNS, otherwise NetBIOS.
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

/// How the sweep pinged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// One ICMP socket, paced.
    IcmpSocket,
    /// The system `ping`, one process per address (no ICMP socket allowed).
    PingCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// The scanned addresses (inclusive).
    pub range: scope::Range,
    /// Live hosts only, in address order.
    pub hosts: Vec<Host>,
    pub method: Method,
    /// TCP connects reached every address (small ranges), not only live ones.
    pub full_tcp: bool,
    /// A connected subnet (ARP table used); false = routed.
    pub local: bool,
    pub cancelled: bool,
    pub duration_ms: u64,
}

/// What [`run`] scans.
pub struct Plan {
    pub range: scope::Range,
    /// This computer's address when the range is in a connected subnet.
    pub own: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub options: Options,
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

/// Probes every address of `plan.range` (already checked by [`scope::check`]).
pub async fn run(
    plan: Plan,
    prober: Arc<dyn Prober>,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(Progress) + Send + Sync,
) -> ScanResult {
    let started = Instant::now();
    let Plan {
        range,
        own,
        gateway,
        options,
    } = plan;
    let local = own.is_some();
    let hosts: Vec<Ipv4Addr> = range.hosts().collect();
    let total = hosts.len() as u64;
    let full_tcp = range.len() <= scope::FULL_TCP_LIMIT;
    let limit = Arc::new(Semaphore::new(scope::CONCURRENCY));
    let stopped = || cancel.load(Ordering::Relaxed);

    // ---- stage A: ICMP sweep (+ first TCP ports on every address for small ranges)
    let pinged = Arc::new(AtomicU64::new(0));
    let mut icmp_task = {
        let (p, h, c, n) = (
            Arc::clone(&prober),
            hosts.clone(),
            Arc::clone(&cancel),
            Arc::clone(&pinged),
        );
        tokio::spawn(async move {
            let counter = Arc::clone(&n);
            let sent: SentFn = Arc::new(move |k| counter.store(k, Ordering::Relaxed));
            if let Some(found) = p.icmp(h.clone(), Arc::clone(&c), sent).await {
                return (Method::IcmpSocket, found);
            }
            (Method::PingCommand, ping_all(p, h, c, n).await)
        })
    };
    let mut tcp: HashMap<Ipv4Addr, Vec<(u16, Tcp)>> = HashMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    if full_tcp {
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
    }
    let mut finished: HashMap<Ipv4Addr, usize> = HashMap::new();
    let mut tcp_done = 0u64;
    let mut icmp: Option<(Method, HashMap<Ipv4Addr, Duration>)> = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    let report = |tcp_done: u64| {
        let pings = pinged.load(Ordering::Relaxed).min(total);
        progress(Progress {
            stage: Stage::Sweep,
            done: if full_tcp { tcp_done.min(pings) } else { pings },
            total,
        });
    };
    while icmp.is_none() || !tasks.is_empty() {
        tokio::select! {
            r = tasks.join_next(), if !tasks.is_empty() => {
                let Some(Ok((ip, port, outcome))) = r else { continue };
                tcp.entry(ip).or_default().push((port, outcome));
                let n = finished.entry(ip).or_default();
                *n += 1;
                if *n == FIRST_PORTS.len() {
                    tcp_done += 1;
                }
            }
            r = &mut icmp_task, if icmp.is_none() => {
                icmp = Some(r.unwrap_or((Method::IcmpSocket, HashMap::new())));
                pinged.store(total, Ordering::Relaxed);
            }
            _ = ticker.tick() => report(tcp_done),
        }
    }
    report(tcp_done);
    let (method, icmp) = icmp.unwrap_or((Method::IcmpSocket, HashMap::new()));

    // ---- stage B: ARP table (filled by the probes above), connected subnets only
    progress(Progress {
        stage: Stage::Arp,
        done: 0,
        total: 1,
    });
    let in_range = |ip: &Ipv4Addr| {
        u32::from(*ip) >= u32::from(range.first) && u32::from(*ip) <= u32::from(range.last)
    };
    let arp: HashMap<Ipv4Addr, String> = if stopped() || !local {
        HashMap::new()
    } else {
        prober
            .arp()
            .await
            .into_iter()
            .filter(|(ip, _)| in_range(ip))
            .collect()
    };

    let mut result: Vec<Host> = hosts
        .iter()
        .filter_map(|ip| {
            let outcomes = tcp.get(ip).cloned().unwrap_or_default();
            let tcp_alive = outcomes.iter().any(|(_, o)| *o != Tcp::Silent);
            let alive =
                own == Some(*ip) || icmp.contains_key(ip) || tcp_alive || arp.contains_key(ip);
            alive.then(|| Host {
                ip: *ip,
                rtt_ms: icmp.get(ip).copied().map(ms),
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
                own: own == Some(*ip),
                gateway: gateway == Some(*ip),
            })
        })
        .collect();

    // ---- stage C: remaining ports + names on live hosts
    if !stopped() {
        let ports: Vec<u16> = scope::PORTS
            .iter()
            .copied()
            .filter(|p| !full_tcp || !FIRST_PORTS.contains(p))
            .collect();
        let live: Vec<Ipv4Addr> = result.iter().map(|h| h.ip).collect();
        let total = live.len() as u64;
        let mut tasks = tokio::task::JoinSet::new();
        for ip in live {
            let (p, l, c, ports) = (
                Arc::clone(&prober),
                Arc::clone(&limit),
                Arc::clone(&cancel),
                ports.clone(),
            );
            tasks.spawn(async move {
                let Ok(_permit) = l.acquire_owned().await else {
                    return (ip, Vec::new(), None);
                };
                if c.load(Ordering::Relaxed) {
                    return (ip, Vec::new(), None);
                }
                let name = async {
                    let dns = if options.resolve_dns {
                        tokio::time::timeout(DNS_TIMEOUT, p.rdns(ip))
                            .await
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                    match dns {
                        Some(n) => Some(n),
                        None => tokio::time::timeout(NETBIOS_TIMEOUT * 2, p.netbios(ip))
                            .await
                            .ok()
                            .flatten(),
                    }
                };
                let open = async {
                    let mut open = Vec::new();
                    for port in &ports {
                        if p.tcp(ip, *port).await == Tcp::Open {
                            open.push(*port);
                        }
                    }
                    open
                };
                let (open, name) = futures_lite::future::zip(open, name).await;
                (ip, open, name)
            });
        }
        let mut done = 0u64;
        while let Some(r) = tasks.join_next().await {
            let Ok((ip, open, name)) = r else { continue };
            if let Some(h) = result.iter_mut().find(|h| h.ip == ip) {
                h.open_ports.extend(open);
                h.open_ports.sort_unstable();
                h.open_ports.dedup();
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
        range,
        hosts: result,
        method,
        full_tcp,
        local,
        cancelled: stopped(),
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// The fallback sweep: the system ping per address, [`PING_PARALLEL`] at a time.
async fn ping_all(
    prober: Arc<dyn Prober>,
    hosts: Vec<Ipv4Addr>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
) -> HashMap<Ipv4Addr, Duration> {
    let limit = Arc::new(Semaphore::new(PING_PARALLEL));
    let mut tasks = tokio::task::JoinSet::new();
    for ip in hosts {
        let (p, l, c, d) = (
            Arc::clone(&prober),
            Arc::clone(&limit),
            Arc::clone(&cancel),
            Arc::clone(&done),
        );
        tasks.spawn(async move {
            let _permit = l.acquire_owned().await.ok()?;
            let rtt = if c.load(Ordering::Relaxed) {
                None
            } else {
                p.ping(ip).await
            };
            d.fetch_add(1, Ordering::Relaxed);
            rtt.map(|t| (ip, t))
        });
    }
    let mut out = HashMap::new();
    while let Some(r) = tasks.join_next().await {
        if let Ok(Some((ip, t))) = r {
            out.insert(ip, t);
        }
    }
    out
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
        sent: SentFn,
    ) -> Option<HashMap<Ipv4Addr, Duration>> {
        tokio::task::spawn_blocking(move || {
            crate::platform::net::icmp_sweep(&hosts, ICMP_WAIT, &cancel, sent.as_ref())
        })
        .await
        .ok()
        .flatten()
    }

    async fn ping(&self, ip: Ipv4Addr) -> Option<Duration> {
        tokio::task::spawn_blocking(move || crate::platform::net::ping_once(ip, PING_TIMEOUT))
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

    async fn netbios(&self, ip: Ipv4Addr) -> Option<String> {
        tokio::task::spawn_blocking(move || super::netbios::node_name(ip, NETBIOS_TIMEOUT))
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// .1 gateway (ARP only, no ports), .2 SSH open, .3 refuses everything, .4 pings
    /// (NetBIOS name), .5 web server, the rest silent.
    struct Fake {
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        probed: Mutex<Vec<(Ipv4Addr, u16)>>,
        /// None = no ICMP socket: the ping fallback answers instead.
        socket: bool,
        pinged: AtomicUsize,
        dns_asked: AtomicUsize,
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
        async fn icmp(
            &self,
            hosts: Vec<Ipv4Addr>,
            _: Arc<AtomicBool>,
            sent: SentFn,
        ) -> Option<HashMap<Ipv4Addr, Duration>> {
            if !self.socket {
                return None;
            }
            sent(hosts.len() as u64);
            Some(
                [
                    (ip(4), Duration::from_micros(1234)),
                    (ip(5), Duration::ZERO),
                ]
                .into_iter()
                .filter(|(a, _)| hosts.contains(a))
                .collect(),
            )
        }
        async fn ping(&self, a: Ipv4Addr) -> Option<Duration> {
            self.pinged.fetch_add(1, Ordering::SeqCst);
            (a == ip(4)).then_some(Duration::from_millis(3))
        }
        async fn arp(&self) -> Vec<(Ipv4Addr, String)> {
            vec![
                (ip(1), "aa:bb:cc:dd:ee:01".into()),
                (ip(2), "aa:bb:cc:dd:ee:02".into()),
                (Ipv4Addr::new(10, 0, 0, 1), "aa:bb:cc:dd:ee:03".into()),
            ]
        }
        async fn rdns(&self, a: Ipv4Addr) -> Option<String> {
            self.dns_asked.fetch_add(1, Ordering::SeqCst);
            (a == ip(2)).then(|| "sw1.lab".to_owned())
        }
        async fn netbios(&self, a: Ipv4Addr) -> Option<String> {
            (a == ip(4)).then(|| "PC-MUHASEBE".to_owned())
        }
    }

    fn fake(socket: bool) -> Arc<Fake> {
        Arc::new(Fake {
            in_flight: 0.into(),
            max_in_flight: 0.into(),
            probed: Mutex::new(Vec::new()),
            socket,
            pinged: 0.into(),
            dns_asked: 0.into(),
        })
    }

    fn plan(first: Ipv4Addr, last: Ipv4Addr, local: bool) -> Plan {
        Plan {
            range: scope::Range { first, last },
            own: local.then(|| ip(34)),
            gateway: Some(ip(1)),
            options: Options { resolve_dns: true },
        }
    }

    fn alive(r: &ScanResult) -> Vec<u8> {
        r.hosts.iter().map(|h| h.ip.octets()[3]).collect()
    }

    #[tokio::test]
    async fn live_hosts_ports_times_mac_and_names() {
        let f = fake(true);
        let seen = Mutex::new(Vec::new());
        let r = run(
            plan(ip(1), ip(254), true),
            f.clone(),
            Arc::new(AtomicBool::new(false)),
            |p| seen.lock().unwrap().push(p),
        )
        .await;
        assert_eq!(alive(&r), [1, 2, 3, 4, 5, 34], "only live hosts, in order");
        let h = |n: u8| r.hosts.iter().find(|h| h.ip == ip(n)).unwrap();
        assert!(h(1).gateway && h(1).mac.is_some() && h(1).open_ports.is_empty());
        assert_eq!(h(1).rtt_ms, None, "found through ARP only");
        assert_eq!(h(2).open_ports, [22]);
        assert_eq!(h(2).hostname.as_deref(), Some("sw1.lab"));
        assert!(h(3).open_ports.is_empty(), "refused = alive, not open");
        assert_eq!(h(4).rtt_ms, Some(1.2));
        assert_eq!(h(4).hostname.as_deref(), Some("PC-MUHASEBE"), "NetBIOS");
        assert_eq!(h(5).open_ports, [80, 443]);
        assert!(h(34).own);
        assert!(r.local && r.full_tcp && !r.cancelled);
        assert_eq!(r.method, Method::IcmpSocket);
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
    async fn routed_networks_skip_arp_and_own_address() {
        let f = fake(true);
        let r = run(
            plan(ip(1), ip(254), false),
            f,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await;
        assert_eq!(
            alive(&r),
            [2, 3, 4, 5],
            "no ARP-only gateway, no own address"
        );
        assert!(!r.local && r.hosts.iter().all(|h| h.mac.is_none()));
    }

    #[tokio::test]
    async fn wide_ranges_connect_only_to_hosts_that_answered() {
        let f = fake(true);
        let r = run(
            plan(
                Ipv4Addr::new(192, 168, 0, 1),
                Ipv4Addr::new(192, 168, 7, 254),
                true,
            ),
            f.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await;
        assert!(!r.full_tcp);
        // .2 and .3 answer only TCP: a wide scan does not find them (documented).
        assert_eq!(alive(&r), [1, 2, 4, 5, 34]);
        let probed = f.probed.lock().unwrap().clone();
        let targets: HashSet<Ipv4Addr> = probed.iter().map(|(a, _)| *a).collect();
        assert_eq!(targets.len(), 5, "ports only on live hosts");
        let h5 = r.hosts.iter().find(|h| h.ip == ip(5)).unwrap();
        assert_eq!(h5.open_ports, [80, 443], "all ports tried on live hosts");
    }

    #[tokio::test]
    async fn without_an_icmp_socket_the_system_ping_is_used() {
        let f = fake(false);
        let mut p = plan(ip(1), ip(20), false);
        p.options.resolve_dns = false;
        let r = run(p, f.clone(), Arc::new(AtomicBool::new(false)), |_| {}).await;
        assert_eq!(r.method, Method::PingCommand);
        assert_eq!(f.pinged.load(Ordering::SeqCst), 20);
        let h4 = r.hosts.iter().find(|h| h.ip == ip(4)).unwrap();
        assert_eq!(h4.rtt_ms, Some(3.0));
        assert_eq!(f.dns_asked.load(Ordering::SeqCst), 0, "DNS off");
        assert_eq!(h4.hostname.as_deref(), Some("PC-MUHASEBE"));
    }

    #[tokio::test]
    async fn cancel_stops_early() {
        let f = fake(false);
        let r = run(
            plan(ip(1), ip(254), true),
            f.clone(),
            Arc::new(AtomicBool::new(true)),
            |_| {},
        )
        .await;
        assert!(r.cancelled);
        assert!(
            f.probed.lock().unwrap().is_empty(),
            "nothing probed after cancel"
        );
        assert_eq!(f.pinged.load(Ordering::SeqCst), 0);
    }

    /// Real network, read-only discovery of `KENSHELL_SCAN_TARGET` (default: this
    /// computer's first subnet): `KENSHELL_SCAN_TARGET=10.0.0.0/24 cargo test -p kenshell
    /// real_scan -- --ignored --nocapture`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "touches the real network"]
    async fn real_scan() {
        let locals = scope::local_networks();
        let target = std::env::var("KENSHELL_SCAN_TARGET")
            .ok()
            .or_else(|| locals.first().map(scope::default_target))
            .expect("a target");
        let (range, net) = scope::check(&target, &locals).expect("in scope");
        let plan = Plan {
            range,
            own: net.as_ref().map(|n| n.ip),
            gateway: crate::platform::net::default_gateway(),
            options: Options { resolve_dns: true },
        };
        let r = run(
            plan,
            Arc::new(NetProber),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await;
        println!(
            "{target}: {} live of {} in {} ms, {:?}, local {}, full tcp {}",
            r.hosts.len(),
            range.len(),
            r.duration_ms,
            r.method,
            r.local,
            r.full_tcp
        );
        for h in &r.hosts {
            println!(
                "  {:<15} {:>8} {:<17} {:<28} {:<24} {:?}",
                h.ip,
                h.rtt_ms.map_or("-".into(), |t| format!("{t} ms")),
                h.mac.as_deref().unwrap_or(""),
                h.vendor.as_deref().unwrap_or(""),
                h.hostname.as_deref().unwrap_or(""),
                h.open_ports
            );
        }
    }
}
