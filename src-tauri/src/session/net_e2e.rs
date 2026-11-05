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
    let (m, db, _) = manager_with_secrets();
    (m, db)
}

fn manager_with_secrets() -> (SessionManager, Arc<Db>, Arc<crate::secrets::MemoryStore>) {
    let db = Arc::new(Db::open_in_memory().unwrap());
    let consoles = Arc::new(ConsoleRegistry::new(Arc::clone(&db)));
    let secrets = Arc::new(crate::secrets::MemoryStore::default());
    let store: Arc<dyn crate::secrets::SecretStore> = secrets.clone();
    (
        SessionManager::new(Arc::clone(&db), consoles, store),
        db,
        secrets,
    )
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_ssh_saved_password_logs_in_without_prompt_and_wrong_one_falls_back() {
    use crate::secrets::SecretStore;
    let port = free_port();
    let _sim = sim(&["--ssh", "--host-key-seed", "7"], port);
    let (m, db, secrets) = manager_with_secrets();
    let rec = db
        .create_session(&crate::model::SessionInput {
            folder_id: None,
            name: "core".into(),
            protocol: Protocol::Ssh,
            host: Some("127.0.0.1".into()),
            port: Some(port),
            username: Some("admin".into()),
            serial: None,
            env_tag: None,
            tags: vec![],
            favorite: false,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
            identify: true,
        })
        .unwrap();
    secrets
        .set(&crate::secrets::session_account(rec.id), "admin")
        .unwrap();
    let mut req = net(Protocol::Ssh, port, None, false);
    req.session_id = Some(rec.id);

    let out = Arc::new(Rec::default());
    let id = m.open(req.clone(), out.clone()).unwrap();
    until("connected", || connected(&out)).await;
    assert!(out.prompts().is_empty(), "saved password must not prompt");
    let key = out
        .link()
        .and_then(|l| l.host_key)
        .expect("ssh host key info");
    assert!(
        key.kex.is_some() && key.cipher.is_some() && key.mac.is_some(),
        "{key:?}"
    );
    until("prompt", || out.text().contains("core-sw-01>")).await;
    m.close(id).unwrap();

    // Wrong saved password: tried once, then the user is asked.
    secrets
        .set(&crate::secrets::session_account(rec.id), "wrong")
        .unwrap();
    let out = Arc::new(Rec::default());
    let id = m.open(req, out.clone()).unwrap();
    until("asked", || !out.prompts().is_empty()).await;
    m.answer(id, vec!["admin".into()]).unwrap();
    until("connected after prompt", || connected(&out)).await;
    m.close(id).unwrap();
}

async fn read_some(stream: &mut tokio::net::TcpStream, want: &str) -> String {
    use tokio::io::AsyncReadExt;
    let mut got = Vec::new();
    let mut buf = [0u8; 1024];
    let start = Instant::now();
    while !String::from_utf8_lossy(&got).contains(want) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no `{want}` in {got:?}"
        );
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await
        {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

fn ssh_opts(edit: impl FnOnce(&mut SshOptions)) -> SshOptions {
    let mut o = SshOptions::default();
    edit(&mut o);
    o
}

fn ssh_req(port: u16, opts: SshOptions) -> OpenRequest {
    OpenRequest {
        session_id: None,
        target: OpenTarget::Network {
            protocol: Protocol::Ssh,
            host: "127.0.0.1".into(),
            port: Some(port),
            username: Some("admin".into()),
            ssh: opts,
        },
        cols: 80,
        rows: 24,
    }
}

/// Answers every password prompt with "admin" until connected.
async fn login(stage: &str, m: &SessionManager, id: crate::model::RuntimeId, out: &Rec) {
    let start = Instant::now();
    let mut answered = 0;
    while !connected(out) {
        assert!(
            failed(out).is_none(),
            "{stage} failed: {:?}",
            out.events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|e| match e {
                    SessionEvent::State { error: Some(e), .. } => Some(e.clone()),
                    _ => None,
                })
        );
        assert!(start.elapsed() < Duration::from_secs(20), "login timed out");
        if out.prompts().len() > answered {
            answered += 1;
            m.answer(id, vec!["admin".into()]).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_ssh_jump_host_forwards_and_proxy() {
    use crate::model::{Forward, ForwardKind, JumpHost, ProxyConfig, ProxyKind};
    let (pa, pb, pt) = (free_port(), free_port(), free_port());
    let _a = sim(&["--ssh", "--host-key-seed", "11"], pa);
    let _b = sim(
        &["--ssh", "--host-key-seed", "12", "--profile", "ruijie"],
        pb,
    );
    let _t = sim(&["--profile", "ruijie"], pt); // telnet target for forwards
    let (m, db) = manager();

    // 1. Jump host A → target B.
    let out = Arc::new(Rec::default());
    let opts = ssh_opts(|o| {
        o.jump = Some(JumpHost {
            host: "127.0.0.1".into(),
            port: Some(pa),
            username: Some("admin".into()),
        });
    });
    let id = m.open(ssh_req(pb, opts), out.clone()).unwrap();
    login("jump", &m, id, &out).await;
    assert_eq!(
        out.prompts().len(),
        2,
        "one password for the jump host, one for the target"
    );
    until("ruijie prompt via jump", || {
        out.text().contains("access-sw-02>")
    })
    .await;
    assert!(
        db.get_known_host("127.0.0.1", pa).unwrap().is_some(),
        "jump host key stored"
    );
    assert!(
        db.get_known_host("127.0.0.1", pb).unwrap().is_some(),
        "target key stored"
    );
    m.close(id).unwrap();

    // 2. Forwards through A: local, dynamic (SOCKS5) and remote → telnet target.
    let (lp, dp, rp) = (free_port(), free_port(), free_port());
    let fwd = |kind, listen_port, target: bool| Forward {
        kind,
        listen_port,
        target_host: target.then(|| "127.0.0.1".into()),
        target_port: target.then_some(pt),
    };
    let opts = ssh_opts(|o| {
        o.forwards = vec![
            fwd(ForwardKind::Local, lp, true),
            fwd(ForwardKind::Dynamic, dp, false),
            fwd(ForwardKind::Remote, rp, true),
        ];
    });
    let out = Arc::new(Rec::default());
    let id = m.open(ssh_req(pa, opts), out.clone()).unwrap();
    login("forwards", &m, id, &out).await;
    until("forward notices", || {
        out.text().matches("[KenShell] forward").count() >= 3
    })
    .await;
    assert!(!out.text().contains("failed"), "{}", out.text());

    let mut local = tokio::net::TcpStream::connect(("127.0.0.1", lp))
        .await
        .unwrap();
    read_some(&mut local, "Username:").await;

    let mut socks = tokio::net::TcpStream::connect(("127.0.0.1", dp))
        .await
        .unwrap();
    crate::transport::tunnel::socks5_connect(&mut socks, "127.0.0.1", pt)
        .await
        .unwrap();
    read_some(&mut socks, "Username:").await;

    let mut remote = tokio::net::TcpStream::connect(("127.0.0.1", rp))
        .await
        .unwrap();
    read_some(&mut remote, "Username:").await;

    // 3. Proxy: a second session to B through the SOCKS5 forward of the first one.
    let out2 = Arc::new(Rec::default());
    let opts = ssh_opts(|o| {
        o.proxy = Some(ProxyConfig {
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".into(),
            port: dp,
        });
    });
    let id2 = m.open(ssh_req(pb, opts), out2.clone()).unwrap();
    login("proxy", &m, id2, &out2).await;
    until("ruijie via proxy", || out2.text().contains("access-sw-02>")).await;
    m.close(id2).unwrap();
    m.close(id).unwrap();
}

/// A real `ssh-agent` (OpenSSH) holding a key made by our own generator.
#[cfg(unix)]
struct Agent {
    child: Child,
    dir: PathBuf,
    sock: PathBuf,
}

#[cfg(unix)]
impl Agent {
    fn start(tag: u16) -> Self {
        // Short path: unix socket paths are limited to ~104 bytes on macOS.
        let dir = PathBuf::from(format!("/tmp/ks-agent-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("s");
        let child = Proc::new("ssh-agent")
            .arg("-D")
            .arg("-a")
            .arg(&sock)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("ssh-agent (OpenSSH) must be installed");
        let start = Instant::now();
        while !sock.exists() {
            assert!(start.elapsed() < Duration::from_secs(10), "agent socket");
            std::thread::sleep(Duration::from_millis(20));
        }
        Self { child, dir, sock }
    }

    /// Generates an Ed25519 key, adds it to the agent and returns its public line.
    fn add_new_key(&self, name: &str) -> String {
        use crate::keys::{KeyFormat, KeyKind, encode, generate, public_line};
        let key = generate(KeyKind::Ed25519, name).unwrap();
        let path = self.dir.join(name);
        let text = encode(&key, KeyFormat::Openssh, None).unwrap();
        crate::platform::write_private(&path, text.as_bytes()).unwrap();
        let ok = Proc::new("ssh-add")
            .arg(&path)
            .env("SSH_AUTH_SOCK", &self.sock)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "ssh-add");
        public_line(&key).unwrap()
    }
}

#[cfg(unix)]
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator and OpenSSH ssh-agent"]
async fn net_e2e_ssh_agent_key_logs_in_and_unknown_key_falls_back_to_password() {
    use crate::transport::ssh::AGENT_TEST_SOCKET as TEST_SOCKET;
    let port = free_port();
    let agent = Agent::start(port);
    let authorized = agent.add_new_key("device-key");
    let _sim = sim(&["--ssh", "--authorized-key", &authorized], port);
    *TEST_SOCKET.lock().unwrap() = Some(agent.sock.clone());
    let (m, _db) = manager();

    let mut req = net(Protocol::Ssh, port, Some("admin"), false);
    if let OpenTarget::Network { ssh, .. } = &mut req.target {
        ssh.use_agent = true;
    }
    let out = Arc::new(Rec::default());
    let id = m.open(req.clone(), out.clone()).unwrap();
    until("connected with the agent key", || connected(&out)).await;
    assert!(out.prompts().is_empty(), "agent login must not ask");
    until("prompt", || out.text().contains("core-sw-01>")).await;
    m.close(id).unwrap();

    // An agent holding only an unrelated key: tried, rejected, then the password.
    let other = Agent::start(port.wrapping_add(1));
    other.add_new_key("unrelated-key");
    *TEST_SOCKET.lock().unwrap() = Some(other.sock.clone());
    let out = Arc::new(Rec::default());
    let id = m.open(req, out.clone()).unwrap();
    until("asked for the password", || !out.prompts().is_empty()).await;
    m.answer(id, vec!["admin".into()]).unwrap();
    until("connected after password", || connected(&out)).await;
    m.close(id).unwrap();
    *TEST_SOCKET.lock().unwrap() = None;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_modes_follow_the_prompt_and_ruijie_is_identified() {
    use crate::device::modes::CliMode;
    let port = free_port();
    let _sim = sim(&["--profile", "ruijie"], port);
    let (m, _db) = manager();
    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Telnet, port, None, false), out.clone())
        .unwrap();
    until("connected", || connected(&out)).await;
    let modes = |r: &Rec| -> Vec<(CliMode, Option<String>)> {
        r.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Mode { mode, sub } => Some((*mode, sub.clone())),
                _ => None,
            })
            .collect()
    };
    let last_mode = |r: &Rec| modes(r).last().cloned();
    until("username", || out.text().contains("Username:")).await;
    for (send, want) in [("admin\r", "Password:"), ("admin\r", "access-sw-02>")] {
        m.write(id, send.as_bytes()).unwrap();
        until(want, || out.text().contains(want)).await;
    }
    until("user mode", || {
        last_mode(&out) == Some((CliMode::User, None))
    })
    .await;
    for (send, want) in [("en\r", "Password:"), ("ruijie\r", "access-sw-02#")] {
        m.write(id, send.as_bytes()).unwrap();
        until(want, || out.text().contains(want)).await;
    }
    until("privileged", || {
        last_mode(&out) == Some((CliMode::Privileged, None))
    })
    .await;
    m.write(id, b"show version\r").unwrap();
    until("identified as Ruijie", || {
        out.events.lock().unwrap().iter().any(
            |e| matches!(e, SessionEvent::Identity { vendor: Some(v), .. } if v.family == "ruijie"),
        )
    })
    .await;
    m.write(id, b"conf t\r").unwrap();
    until("config", || {
        last_mode(&out) == Some((CliMode::Config, Some("config".into())))
    })
    .await;
    m.write(id, b"int gi 0/24\r").unwrap();
    until("interface sub-mode", || {
        last_mode(&out)
            == Some((
                CliMode::Config,
                Some("config-if-GigabitEthernet 0/24".into()),
            ))
    })
    .await;
    m.write(id, b"end\r").unwrap();
    until("back to privileged", || {
        last_mode(&out) == Some((CliMode::Privileged, None))
    })
    .await;
    // Ruijie must never be labelled Cisco.
    assert!(!out.events.lock().unwrap().iter().any(|e| {
        matches!(e, SessionEvent::Identity { vendor: Some(v), .. } if v.family == "cisco")
    }));
    m.close(id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator"]
async fn net_e2e_background_identity_is_hidden_and_parsed() {
    let port = free_port();
    let _sim = sim(&["--profile", "ruijie"], port);
    let (m, db) = manager();
    let out = Arc::new(Rec::default());
    let id = m
        .open(net(Protocol::Telnet, port, None, false), out.clone())
        .unwrap();
    until("username", || out.text().contains("Username:")).await;
    for (send, want) in [("admin\r", "Password:"), ("admin\r", "access-sw-02>")] {
        m.write(id, send.as_bytes()).unwrap();
        until(want, || out.text().contains(want)).await;
    }
    until("facts from the hidden show version", || {
        out.events.lock().unwrap().iter().any(|e| {
            matches!(e, SessionEvent::Identity { facts: Some(f), vendor: Some(v), .. }
                if f.model.as_deref() == Some("S5750-48GT4XS-E") && v.family == "ruijie")
        })
    })
    .await;
    let text = out.text();
    assert!(
        !text.contains("System serial number"),
        "hidden from the terminal"
    );
    assert!(!text.contains("show version"));
    // Typing works normally afterwards.
    m.write(id, b"en\r").unwrap();
    until("enable asks", || {
        out.text().contains("Password:\r\n") || out.text().matches("Password:").count() >= 2
    })
    .await;
    let devices = db.list_devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].serial.as_deref(), Some("G1MQ3K8000123"));
    m.close(id).unwrap();
}
