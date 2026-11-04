//! End-to-end network sessions against the device simulator (any OS, plain TCP):
//! real SessionManager + Telnet/SSH transports + SQLite host-key store.
//!
//! `cargo build -p device-simulator && cargo test -p kenshell net_e2e -- --ignored`

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command as Proc, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SessionManager, SessionOutput};
use crate::console_detect::ConsoleRegistry;
use crate::error::ErrorCode;
use crate::model::{
    ConnState, NetLink, OpenRequest, OpenTarget, PromptRequest, Protocol, SessionEvent, SshOptions,
};
use crate::storage::Db;

#[derive(Default)]
struct Rec {
    data: Mutex<Vec<u8>>,
    events: Mutex<Vec<SessionEvent>>,
}

impl SessionOutput for Rec {
    fn data(&self, bytes: Vec<u8>) {
        self.data.lock().unwrap().extend(bytes);
    }
    fn event(&self, event: SessionEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Rec {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.data.lock().unwrap()).into_owned()
    }
    fn state(&self) -> Option<(ConnState, Option<ErrorCode>)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|e| match e {
                SessionEvent::State { state, error } => {
                    Some((*state, error.as_ref().map(|e| e.code)))
                }
                _ => None,
            })
    }
    fn prompts(&self) -> Vec<PromptRequest> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Prompt { request } => Some(request.clone()),
                _ => None,
            })
            .collect()
    }
    fn link(&self) -> Option<NetLink> {
        self.events.lock().unwrap().iter().find_map(|e| match e {
            SessionEvent::NetLink { link } => Some(link.clone()),
            _ => None,
        })
    }
}

struct Sim(Child);
impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn sim(args: &[&str], port: u16) -> Sim {
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../target/debug/device-simulator{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        bin.exists(),
        "build it first: cargo build -p device-simulator"
    );
    let port_s = port.to_string();
    let child = Proc::new(bin)
        .args(args)
        .args(["--port", &port_s])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn simulator");
    let start = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "simulator did not listen"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Sim(child)
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

fn manager() -> (SessionManager, Arc<Db>) {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let consoles = Arc::new(ConsoleRegistry::new(Arc::clone(&db)));
    (SessionManager::new(Arc::clone(&db), consoles), db)
}

fn net(protocol: Protocol, port: u16, user: Option<&str>, legacy: bool) -> OpenRequest {
    OpenRequest {
        session_id: None,
        target: OpenTarget::Network {
            protocol,
            host: "127.0.0.1".into(),
            port: Some(port),
            username: user.map(Into::into),
            ssh: SshOptions {
                legacy_algorithms: legacy,
                ..SshOptions::default()
            },
        },
        cols: 100,
        rows: 30,
    }
}

fn connected(r: &Rec) -> bool {
    matches!(r.state(), Some((ConnState::Connected, _)))
}

fn failed(r: &Rec) -> Option<ErrorCode> {
    match r.state() {
        Some((ConnState::Disconnected, Some(code))) => Some(code),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_telnet_login_enable_and_config_mode() {
    let port = free_port();
    let _sim = sim(&["--profile", "ruijie"], port);
    let (m, _db) = manager();
    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Telnet, port, None, false), out.clone())
        .unwrap();
    until("connected", || connected(&out)).await;
    assert_eq!(out.link().unwrap().protocol, Protocol::Telnet);
    until("username prompt", || out.text().contains("Username:")).await;
    for (send, want) in [
        ("admin\r", "Password:"),
        ("admin\r", "access-sw-02>"),
        ("en\r", "Password:"),
        ("ruijie\r", "access-sw-02#"),
        ("conf t\r", "access-sw-02(config)#"),
        ("int gi 0/24\r", "(config-if-GigabitEthernet 0/24)#"),
    ] {
        m.write(id, send.as_bytes()).unwrap();
        until(want, || out.text().contains(want)).await;
    }
    assert!(
        !out.text().contains("admin\r\nadmin"),
        "password must not be echoed"
    );
    m.write(id, b"end\r").unwrap();
    m.write(id, b"exit\r").unwrap();
    until("remote closed", || {
        matches!(out.state(), Some((ConnState::Disconnected, _)))
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_ssh_host_key_learn_accept_and_block_on_change() {
    let port = free_port();
    let (m, db) = manager();
    {
        let _sim = sim(&["--ssh", "--host-key-seed", "1"], port);
        // 1st connection: no user name → "login as:" (echo) then password (no echo).
        let out = Arc::new(Rec::default());
        let id = m
            .open(net(Protocol::Ssh, port, None, false), out.clone())
            .unwrap();
        until("login prompt", || out.prompts().len() == 1).await;
        let p = &out.prompts()[0];
        assert!(p.items[0].echo && p.items[0].text.contains("login as"));
        m.answer(id, vec!["admin".into()]).unwrap();
        until("password prompt", || out.prompts().len() == 2).await;
        assert!(
            !out.prompts()[1].items[0].echo,
            "password prompt must be hidden"
        );
        m.answer(id, vec!["admin".into()]).unwrap();
        until("connected", || connected(&out)).await;
        let link = out.link().unwrap();
        let hk = link.host_key.expect("host key info");
        assert!(hk.first_seen && hk.fingerprint.starts_with("SHA256:"));
        assert_eq!(link.username.as_deref(), Some("admin"));
        until("shell prompt", || out.text().contains("core-sw-01>")).await;
        m.write(id, b"terminal length 0\rshow ver\r").unwrap();
        until("show version", || {
            out.text().contains("Cisco IOS XE Software")
        })
        .await;
        m.close(id).unwrap();

        // 2nd connection, same key → accepted silently, not first seen.
        let out = Arc::new(Rec::default());
        let id = m
            .open(net(Protocol::Ssh, port, Some("admin"), false), out.clone())
            .unwrap();
        until("password prompt", || out.prompts().len() == 1).await;
        m.answer(id, vec!["admin".into()]).unwrap();
        until("connected again", || connected(&out)).await;
        assert!(!out.link().unwrap().host_key.unwrap().first_seen);
        m.close(id).unwrap();
    }
    // Device "replaced": different host key on the same host:port → BLOCKED.
    let _sim = sim(&["--ssh", "--host-key-seed", "2"], port);
    let out = Arc::new(Rec::default());
    m.open(net(Protocol::Ssh, port, Some("admin"), false), out.clone())
        .unwrap();
    until("blocked", || failed(&out).is_some()).await;
    assert_eq!(failed(&out), Some(ErrorCode::HostKeyChanged));
    assert!(
        out.prompts().is_empty(),
        "no credentials may be sent to a changed host"
    );
    // Explicitly forgetting the key lets the next connection learn the new one.
    db.forget_known_host("127.0.0.1", port).unwrap();
    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Ssh, port, Some("admin"), false), out.clone())
        .unwrap();
    until("password prompt", || out.prompts().len() == 1).await;
    m.answer(id, vec!["admin".into()]).unwrap();
    until("connected after forget", || connected(&out)).await;
    assert!(out.link().unwrap().host_key.unwrap().first_seen);
    m.close(id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_ssh_legacy_algorithms_and_wrong_password() {
    let port = free_port();
    let _sim = sim(&["--ssh", "--legacy-only", "--profile", "ruijie"], port);
    let (m, _db) = manager();

    let out = Arc::new(Rec::default());
    m.open(net(Protocol::Ssh, port, Some("admin"), false), out.clone())
        .unwrap();
    until("no common algorithm", || failed(&out).is_some()).await;
    assert_eq!(failed(&out), Some(ErrorCode::NoCommonAlgorithm));

    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Ssh, port, Some("admin"), true), out.clone())
        .unwrap();
    until("password prompt", || out.prompts().len() == 1).await;
    m.answer(id, vec!["admin".into()]).unwrap();
    until("legacy connected", || connected(&out)).await;
    until("ruijie prompt", || out.text().contains("access-sw-02>")).await;
    m.close(id).unwrap();

    // Wrong password everywhere → AuthFailed after the allowed attempts.
    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Ssh, port, Some("admin"), true), out.clone())
        .unwrap();
    for n in 1..=4 {
        until("prompt", || {
            out.prompts().len() >= n || failed(&out).is_some()
        })
        .await;
        if failed(&out).is_some() {
            break;
        }
        m.answer(id, vec!["wrong".into()]).unwrap();
    }
    until("auth failed", || failed(&out).is_some()).await;
    assert_eq!(failed(&out), Some(ErrorCode::AuthFailed));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_session_log_has_output_but_never_typed_passwords() {
    let port = free_port();
    let _sim = sim(&["--profile", "cisco"], port);
    let (m, db) = manager();
    let dir = tempfile::tempdir().unwrap();
    db.set_setting("log.enabled", &serde_json::json!(true))
        .unwrap();
    db.set_setting("log.dir", &serde_json::json!(dir.path()))
        .unwrap();
    db.set_setting("log.template", &serde_json::json!("{name}"))
        .unwrap();

    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Telnet, port, None, false), out.clone())
        .unwrap();
    until("connected", || connected(&out)).await;
    let path = out
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match e {
            SessionEvent::Logging { path, .. } => path.clone(),
            _ => None,
        })
        .expect("log path event");
    until("username", || out.text().contains("Username:")).await;
    for (send, want) in [
        ("admin\r", "Password:"),
        ("Sup3rS3cret!\r", "Login invalid"),
        ("admin\r", "Password:"),
        ("admin\r", "core-sw-01>"),
        ("enable\r", "Password:"),
        ("cisco\r", "core-sw-01#"),
    ] {
        m.write(id, send.as_bytes()).unwrap();
        until(want, || out.text().contains(want)).await;
    }
    m.close(id).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = std::fs::read_to_string(&path).unwrap();
    assert!(log.contains("core-sw-01#"), "{log}");
    assert!(!log.contains('\x1b'), "printable mode strips escapes");
    for secret in ["Sup3rS3cret!", "cisco\n"] {
        assert!(
            !log.contains(secret),
            "typed secret {secret:?} leaked into the log:\n{log}"
        );
    }
}
