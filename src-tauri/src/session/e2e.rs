//! End-to-end: real SessionManager + real autobaud + real SerialTransport + real SQLite
//! against the device simulator on a pseudo-terminal that only answers cleanly at 115200.
//!
//! Linux only (macOS ptys reject IOSSIOSPEED). Needs the simulator binary: `cargo build -p device-simulator`, then
//! `cargo test -p kenshell e2e -- --ignored`.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command as Proc, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SessionManager, SessionOutput};
use crate::console_detect::ConsoleRegistry;
use crate::model::{ConnState, OpenRequest, OpenTarget, SerialSettings, SessionEvent};
use crate::storage::Db;

#[derive(Default)]
struct Recorder {
    data: Mutex<Vec<u8>>,
    events: Mutex<Vec<SessionEvent>>,
}

impl SessionOutput for Recorder {
    fn data(&self, bytes: Vec<u8>) {
        self.data.lock().unwrap().extend(bytes);
    }
    fn event(&self, event: SessionEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.data.lock().unwrap()).into_owned()
    }
    fn connected(&self) -> bool {
        self.events.lock().unwrap().iter().any(|e| {
            matches!(
                e,
                SessionEvent::State {
                    state: ConnState::Connected,
                    ..
                }
            )
        })
    }
}

struct Simulator(Child);
impl Drop for Simulator {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn simulator_bin() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/device-simulator");
    p.exists().then_some(p)
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs target/debug/device-simulator (cargo build -p device-simulator)"]
async fn autobaud_connects_to_simulated_ruijie_console_and_remembers_speed() {
    let Some(bin) = simulator_bin() else {
        panic!("build the simulator first: cargo build -p device-simulator");
    };
    let mut child = Proc::new(bin)
        .args(["--pty", "--baud", "115200", "--profile", "ruijie"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn simulator");
    let mut path = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut path)
        .unwrap();
    let path = path.trim().to_string();
    let _sim = Simulator(child);
    assert!(path.starts_with("/dev/"), "simulator printed {path:?}");

    let db = Arc::new(Db::open_in_memory().unwrap());
    let consoles = Arc::new(ConsoleRegistry::new(Arc::clone(&db)));
    let manager = SessionManager::new(
        Arc::clone(&db),
        Arc::clone(&consoles),
        Arc::new(crate::secrets::MemoryStore::default()),
    );

    let open = |out: Arc<Recorder>| {
        manager
            .open(
                OpenRequest {
                    session_id: None,
                    target: OpenTarget::Serial {
                        settings: SerialSettings {
                            port: path.clone(),
                            ..SerialSettings::default()
                        },
                    },
                    cols: 80,
                    rows: 24,
                },
                out,
            )
            .unwrap()
    };

    // 1st connection: 9600 is garbage, 115200 answers "Username:" → accepted.
    let out = Arc::new(Recorder::default());
    let id = open(Arc::clone(&out));
    let o = Arc::clone(&out);
    let start = Instant::now();
    while !o.connected() && start.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        o.connected(),
        "events: {:#?}\ndata: {:?}",
        o.events.lock().unwrap(),
        o.text()
    );
    let result = out.events.lock().unwrap().iter().find_map(|e| match e {
        SessionEvent::Autobaud {
            result: Some(b), ..
        } => Some(*b),
        _ => None,
    });
    assert_eq!(result, Some(115_200));
    let link = out.events.lock().unwrap().iter().find_map(|e| match e {
        SessionEvent::Link { link } => Some(link.clone()),
        _ => None,
    });
    let link = link.expect("link event");
    assert_eq!(
        (link.baud, link.framing.as_str(), link.auto_detected),
        (115_200, "8N1", true)
    );
    assert_eq!(
        consoles.remembered_baud(&path),
        Some(115_200),
        "speed remembered"
    );

    // Talk to the device through the session.
    manager.write(id, b"admin\r").unwrap();
    wait_until("password prompt", || out.text().contains("Password:")).await;
    manager.write(id, b"admin\r").unwrap();
    wait_until("user prompt", || out.text().contains("access-sw-02>")).await;
    manager.close(id).unwrap();

    // 2nd connection: remembered 115200 is tried first → a single trial.
    let out2 = Arc::new(Recorder::default());
    let id2 = open(Arc::clone(&out2));
    wait_until("reconnected", || out2.connected()).await;
    let trials = out2
        .events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|e| match e {
            SessionEvent::Autobaud { trials, result, .. } if result.is_some() => Some(trials.len()),
            _ => None,
        });
    assert_eq!(trials, Some(1), "remembered baud must be tried first");
    manager.close(id2).unwrap();
}
