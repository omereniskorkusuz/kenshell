//! SessionManager state-machine tests with a fake backend and a scripted transport.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use super::backend::Backend;
use super::task::MAX_UI_BATCH;
use super::{SessionManager, SessionOutput, Timing};
use crate::console_detect::autobaud::Outcome;
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{
    BaudTrial, ConnState, ConsoleChange, ConsolePort, ConsoleSnapshot, HealthKind, OpenRequest,
    OpenTarget, RuntimeId, SerialLink, SerialSettings, SessionEvent,
};
use crate::transport::{Transport, TransportEvent, TransportSink};

const PATH: &str = "/dev/cu.fake0";
const KEY: &str = "usb:FAKE0";

// ---------------------------------------------------------------- fakes

#[derive(Default)]
struct TransportLog {
    baud: u32,
    writes: Vec<Vec<u8>>,
    resizes: Vec<(u16, u16)>,
    breaks: usize,
    disconnects: usize,
}

struct FakeTransport {
    connect: Result<()>,
    log: Arc<Mutex<TransportLog>>,
    write_result: Result<()>,
}

#[async_trait]
impl Transport for FakeTransport {
    async fn connect(&mut self) -> Result<()> {
        self.connect.clone()
    }
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.log.lock().unwrap().writes.push(data.to_vec());
        self.write_result.clone()
    }
    async fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.log.lock().unwrap().resizes.push((cols, rows));
        Ok(())
    }
    async fn send_break(&mut self) -> Result<()> {
        self.log.lock().unwrap().breaks += 1;
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<()> {
        self.log.lock().unwrap().disconnects += 1;
        Ok(())
    }
}

enum AutobaudScript {
    /// Emit these trials, then return the outcome.
    Result(Outcome),
    /// Block until cancelled (models a long detection).
    UntilCancelled,
}

struct FakeState {
    remembered: HashMap<String, u32>,
    remember_calls: Vec<(String, u32)>,
    autobaud: AutobaudScript,
    autobaud_calls: Vec<Option<u32>>,
    autobaud_cancelled: bool,
    connect_results: VecDeque<Result<()>>,
    write_result: Result<()>,
    sinks: Vec<TransportSink>,
    transports: Vec<Arc<Mutex<TransportLog>>>,
    touched: Vec<i64>,
    auto_reconnect: bool,
    net_auto_reconnect: bool,
    identify: bool,
    devices: Vec<(String, crate::device::modes::DeviceFacts)>,
}

struct FakeBackend(Mutex<FakeState>);

impl FakeBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(FakeState {
            remembered: HashMap::new(),
            remember_calls: vec![],
            autobaud: AutobaudScript::Result(outcome(None, None, vec![], true)),
            autobaud_calls: vec![],
            autobaud_cancelled: false,
            connect_results: VecDeque::new(),
            write_result: Ok(()),
            sinks: vec![],
            transports: vec![],
            touched: vec![],
            auto_reconnect: true,
            net_auto_reconnect: false,
            identify: false,
            devices: vec![],
        })))
    }

    fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }

    /// Sink of the n-th transport created (to script device output).
    fn sink(&self, n: usize) -> TransportSink {
        self.st().sinks[n].clone()
    }

    fn transport(&self, n: usize) -> Arc<Mutex<TransportLog>> {
        Arc::clone(&self.st().transports[n])
    }

    fn transport_count(&self) -> usize {
        self.st().transports.len()
    }
}

impl Backend for FakeBackend {
    fn remembered_baud(&self, path: &str) -> Option<u32> {
        self.st().remembered.get(path).copied()
    }
    fn remember(&self, path: &str, baud: u32) -> Result<()> {
        let mut st = self.st();
        st.remember_calls.push((path.to_string(), baud));
        st.remembered.insert(path.to_string(), baud);
        Ok(())
    }
    fn display_name(&self, path: &str) -> String {
        path.rsplit('.').next().unwrap_or(path).to_string()
    }
    fn chip_label(&self, _path: &str) -> Option<String> {
        Some("FTDI FT232R".into())
    }
    fn port_key(&self, _path: &str) -> String {
        KEY.into()
    }
    fn run_autobaud(
        &self,
        _settings: &SerialSettings,
        remembered: Option<u32>,
        cancel: &AtomicBool,
        on_trial: &mut dyn FnMut(&BaudTrial),
    ) -> Result<Outcome> {
        let script = {
            let mut st = self.st();
            st.autobaud_calls.push(remembered);
            match &st.autobaud {
                AutobaudScript::Result(o) => Some(o.clone()),
                AutobaudScript::UntilCancelled => None,
            }
        };
        match script {
            Some(o) => {
                for t in &o.trials {
                    on_trial(t);
                }
                Ok(o)
            }
            None => {
                let start = std::time::Instant::now();
                while !cancel.load(Ordering::Acquire) {
                    assert!(start.elapsed() < Duration::from_secs(5), "never cancelled");
                    std::thread::sleep(Duration::from_millis(2));
                }
                self.st().autobaud_cancelled = true;
                Err(AppError::invalid("autobaud cancelled"))
            }
        }
    }
    fn make_transport(
        &self,
        _settings: &SerialSettings,
        baud: u32,
        sink: TransportSink,
    ) -> Box<dyn Transport> {
        let mut st = self.st();
        let log = Arc::new(Mutex::new(TransportLog {
            baud,
            ..TransportLog::default()
        }));
        st.sinks.push(sink);
        st.transports.push(Arc::clone(&log));
        Box::new(FakeTransport {
            connect: st.connect_results.pop_front().unwrap_or(Ok(())),
            log,
            write_result: st.write_result.clone(),
        })
    }
    fn make_network_transport(
        &self,
        _target: &OpenTarget,
        _size: (u16, u16),
        sink: TransportSink,
        _prompter: Arc<dyn crate::transport::Prompter>,
        _creds: Option<crate::secrets::Credentials>,
    ) -> Result<Box<dyn Transport>> {
        Ok(self.make_transport(&SerialSettings::default(), 0, sink))
    }

    fn touch_session(&self, id: i64) -> Result<()> {
        self.st().touched.push(id);
        Ok(())
    }
    fn auto_reconnect_enabled(&self) -> bool {
        self.st().auto_reconnect
    }

    fn net_auto_reconnect_enabled(&self) -> bool {
        self.st().net_auto_reconnect
    }

    fn log_config(&self) -> Option<crate::logging::LogConfig> {
        None
    }

    fn credentials(&self, _session_id: i64) -> Option<crate::secrets::Credentials> {
        None
    }

    fn identify_enabled(&self, _session_id: Option<i64>) -> bool {
        self.st().identify
    }

    fn record_device(
        &self,
        _session_id: Option<i64>,
        vendor: &str,
        facts: &crate::device::modes::DeviceFacts,
    ) {
        self.st().devices.push((vendor.to_owned(), facts.clone()));
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Rec {
    Data(Vec<u8>),
    Ev(SessionEvent),
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Rec>>);

impl SessionOutput for Recorder {
    fn data(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().push(Rec::Data(bytes));
    }
    fn event(&self, event: SessionEvent) {
        self.0.lock().unwrap().push(Rec::Ev(event));
    }
}

impl Recorder {
    fn all(&self) -> Vec<Rec> {
        self.0.lock().unwrap().clone()
    }
    fn events(&self) -> Vec<SessionEvent> {
        self.all()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Ev(e) => Some(e),
                Rec::Data(_) => None,
            })
            .collect()
    }
    fn data(&self) -> Vec<Vec<u8>> {
        self.all()
            .into_iter()
            .filter_map(|r| match r {
                Rec::Data(d) => Some(d),
                Rec::Ev(_) => None,
            })
            .collect()
    }
    fn states(&self) -> Vec<ConnState> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::State { state, .. } => Some(state),
                _ => None,
            })
            .collect()
    }
    fn health(&self) -> Vec<HealthKind> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::Health { kind } => Some(kind),
                _ => None,
            })
            .collect()
    }
    fn links(&self) -> Vec<SerialLink> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::Link { link } => Some(link),
                _ => None,
            })
            .collect()
    }
    fn count_state(&self, s: ConnState) -> usize {
        self.states().into_iter().filter(|x| *x == s).count()
    }
    fn last_error(&self) -> Option<AppError> {
        self.events().into_iter().rev().find_map(|e| match e {
            SessionEvent::State { error, .. } => error,
            _ => None,
        })
    }
}

// ---------------------------------------------------------------- helpers

fn timing() -> Timing {
    Timing {
        detach_grace: Duration::from_millis(40),
        reconnect_settle: Duration::from_millis(1),
        net_retry: [Duration::from_millis(1); 4],
    }
}

fn setup() -> (Arc<FakeBackend>, SessionManager, Arc<Recorder>) {
    let backend = FakeBackend::new();
    let mgr = SessionManager::for_tests(backend.clone(), timing());
    (backend, mgr, Arc::new(Recorder::default()))
}

fn request(baud: Option<u32>, session_id: Option<i64>) -> OpenRequest {
    OpenRequest {
        session_id,
        target: OpenTarget::Serial {
            settings: SerialSettings {
                port: PATH.into(),
                baud,
                ..SerialSettings::default()
            },
        },
        cols: 80,
        rows: 24,
    }
}

fn trial(baud: u32, score: f32, prompt: bool) -> BaudTrial {
    BaudTrial {
        baud,
        score,
        prompt_match: prompt,
        bytes: 12,
    }
}

fn outcome(
    accepted: Option<u32>,
    suggestion: Option<u32>,
    trials: Vec<BaudTrial>,
    no_data: bool,
) -> Outcome {
    Outcome {
        accepted,
        suggestion,
        trials,
        no_data,
    }
}

/// Polls `cond` (yielding to the session task) until it holds or 3 s pass.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn open_connected(
    backend: &FakeBackend,
    mgr: &SessionManager,
    out: &Arc<Recorder>,
    baud: Option<u32>,
) -> RuntimeId {
    let before = out.count_state(ConnState::Connected);
    let id = mgr.open(request(baud, None), out.clone()).unwrap();
    until("connected", || {
        out.count_state(ConnState::Connected) > before
    })
    .await;
    let _ = backend;
    id
}

fn change(attached: &[&str], detached: &[&str], ports: Vec<ConsolePort>) -> ConsoleChange {
    ConsoleChange {
        snapshot: ConsoleSnapshot {
            ports,
            driver_hints: vec![],
        },
        attached: attached.iter().map(|s| s.to_string()).collect(),
        detached: detached.iter().map(|s| s.to_string()).collect(),
    }
}

fn port(key: &str, path: &str) -> ConsolePort {
    ConsolePort {
        key: key.into(),
        path: path.into(),
        alt_path: None,
        display_name: "fake0".into(),
        chip: None,
        manufacturer: None,
        product: None,
        serial_number: None,
        remembered_baud: None,
        last_device: None,
    }
}

// ---------------------------------------------------------------- open flow

#[tokio::test]
async fn manual_baud_connects_without_autobaud_and_is_remembered() {
    let (backend, mgr, out) = setup();
    let id = mgr
        .open(request(Some(115200), Some(7)), out.clone())
        .unwrap();
    assert_eq!(id, 1);
    until("connected", || out.count_state(ConnState::Connected) == 1).await;

    assert_eq!(
        out.states(),
        vec![ConnState::Connecting, ConnState::Connected]
    );
    let links = out.links();
    assert_eq!(
        links,
        vec![SerialLink {
            path: PATH.into(),
            display_name: "fake0".into(),
            baud: 115200,
            framing: "8N1".into(),
            chip: Some("FTDI FT232R".into()),
            auto_detected: false,
        }]
    );
    // Link precedes Connected.
    let evs = out.events();
    let link_at = evs
        .iter()
        .position(|e| matches!(e, SessionEvent::Link { .. }));
    let conn_at = evs.iter().position(|e| {
        matches!(
            e,
            SessionEvent::State {
                state: ConnState::Connected,
                ..
            }
        )
    });
    assert!(link_at < conn_at);

    let st = backend.st();
    assert!(st.autobaud_calls.is_empty());
    assert_eq!(st.remember_calls, vec![(PATH.to_string(), 115200)]);
    assert_eq!(st.touched, vec![7]);
    drop(st);
    assert_eq!(backend.transport(0).lock().unwrap().baud, 115200);

    let id2 = mgr.open(request(Some(9600), None), out.clone()).unwrap();
    assert_eq!(id2, 2, "runtime ids are unique and increasing");
}

#[tokio::test]
async fn autobaud_accepted_is_used_and_remembered() {
    let (backend, mgr, out) = setup();
    {
        let mut st = backend.st();
        st.remembered.insert(PATH.into(), 115200);
        st.autobaud = AutobaudScript::Result(outcome(
            Some(9600),
            None,
            vec![trial(115200, 0.2, false), trial(9600, 0.97, true)],
            false,
        ));
    }
    open_connected(&backend, &mgr, &out, None).await;

    assert_eq!(
        out.states(),
        vec![
            ConnState::Connecting,
            ConnState::Detecting,
            ConnState::Connected
        ]
    );
    let progress: Vec<_> = out
        .events()
        .into_iter()
        .filter(|e| matches!(e, SessionEvent::Autobaud { .. }))
        .collect();
    assert_eq!(progress.len(), 3, "one per trial + final");
    match &progress[0] {
        SessionEvent::Autobaud {
            trials,
            current,
            result,
            ..
        } => {
            assert_eq!(trials.len(), 1);
            assert_eq!(*current, Some(115200));
            assert_eq!(*result, None);
        }
        _ => unreachable!(),
    }
    match &progress[1] {
        SessionEvent::Autobaud {
            trials, current, ..
        } => {
            assert_eq!(trials.len(), 2);
            assert_eq!(*current, Some(9600));
        }
        _ => unreachable!(),
    }
    match &progress[2] {
        SessionEvent::Autobaud {
            trials,
            current,
            result,
            suggestion,
        } => {
            assert_eq!(trials.len(), 2);
            assert_eq!(*current, None);
            assert_eq!(*result, Some(9600));
            assert_eq!(*suggestion, None);
        }
        _ => unreachable!(),
    }
    let link = &out.links()[0];
    assert_eq!(link.baud, 9600);
    assert!(link.auto_detected);
    let st = backend.st();
    assert_eq!(
        st.autobaud_calls,
        vec![Some(115200)],
        "remembered rate goes first"
    );
    assert_eq!(st.remember_calls, vec![(PATH.to_string(), 9600)]);
    assert!(out.health().is_empty());
}

#[tokio::test]
async fn autobaud_suggestion_is_used_but_not_remembered() {
    let (backend, mgr, out) = setup();
    backend.st().autobaud = AutobaudScript::Result(outcome(
        None,
        Some(38400),
        vec![trial(9600, 0.3, false), trial(38400, 0.8, false)],
        false,
    ));
    open_connected(&backend, &mgr, &out, None).await;
    let link = &out.links()[0];
    assert_eq!(link.baud, 38400);
    assert!(link.auto_detected);
    assert!(backend.st().remember_calls.is_empty());
    match out
        .events()
        .iter()
        .rev()
        .find(|e| matches!(e, SessionEvent::Autobaud { .. }))
    {
        Some(SessionEvent::Autobaud {
            result, suggestion, ..
        }) => {
            assert_eq!(*result, None);
            assert_eq!(*suggestion, Some(38400));
        }
        _ => panic!("final autobaud event missing"),
    }
}

#[tokio::test]
async fn no_data_emits_health_and_falls_back_to_9600() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, None).await;
    assert_eq!(out.health(), vec![HealthKind::NoData]);
    let link = &out.links()[0];
    assert_eq!(link.baud, 9600);
    assert!(!link.auto_detected);
    assert!(backend.st().remember_calls.is_empty());
}

#[tokio::test]
async fn no_data_uses_remembered_baud() {
    let (backend, mgr, out) = setup();
    backend.st().remembered.insert(PATH.into(), 19200);
    open_connected(&backend, &mgr, &out, None).await;
    assert_eq!(out.health(), vec![HealthKind::NoData]);
    assert_eq!(out.links()[0].baud, 19200);
}

#[tokio::test]
async fn open_failure_keeps_session_and_reconnect_retries() {
    let (backend, mgr, out) = setup();
    backend
        .st()
        .connect_results
        .push_back(Err(AppError::with(ErrorCode::PortBusy, "busy")));
    let id = mgr.open(request(Some(9600), None), out.clone()).unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert_eq!(out.last_error().unwrap().code, ErrorCode::PortBusy);
    assert!(out.health().is_empty());
    assert_eq!(
        mgr.write(id, b"x").unwrap_err().detail.as_deref(),
        Some("disconnected")
    );

    mgr.reconnect(id).unwrap();
    until("connected", || out.count_state(ConnState::Connected) == 1).await;
    assert_eq!(backend.transport_count(), 2);
}

#[tokio::test]
async fn invalid_requests_are_rejected_synchronously() {
    let (_backend, mgr, out) = setup();
    let mut req = request(Some(9600), None);
    let OpenTarget::Serial { settings } = &mut req.target else {
        unreachable!()
    };
    settings.port = "  ".into();
    assert_eq!(
        mgr.open(req, out.clone()).unwrap_err().code,
        ErrorCode::Invalid
    );
    assert_eq!(
        mgr.open(request(Some(0), None), out.clone())
            .unwrap_err()
            .code,
        ErrorCode::Invalid
    );
}

// ---------------------------------------------------------------- data + commands

#[tokio::test]
async fn queued_reads_are_batched_into_one_ui_message() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    let sink = backend.sink(0);
    // No await between the sends: the task sees all three at once.
    for chunk in [&b"Switch"[..], b">", b"\r\n"] {
        sink.try_send(TransportEvent::Data(chunk.to_vec())).unwrap();
    }
    until("data", || !out.data().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(out.data(), vec![b"Switch>\r\n".to_vec()]);
}

#[tokio::test]
async fn ui_batches_are_capped() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    let sink = backend.sink(0);
    let chunk = vec![b'x'; 64 * 1024];
    for _ in 0..10 {
        sink.try_send(TransportEvent::Data(chunk.clone())).unwrap();
    }
    until("all data", || {
        out.data().iter().map(Vec::len).sum::<usize>() == 640 * 1024
    })
    .await;
    let sends = out.data();
    assert!(sends.iter().all(|d| d.len() <= MAX_UI_BATCH));
    assert_eq!(sends.len(), 3, "256 + 256 + 128 KiB");
}

#[tokio::test]
async fn commands_reach_the_transport() {
    let (backend, mgr, out) = setup();
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.write(id, b"enable\r").unwrap();
    mgr.write(id, b"").unwrap();
    mgr.resize(id, 120, 40).unwrap();
    mgr.send_break(id).unwrap();
    let log = backend.transport(0);
    until("commands", || log.lock().unwrap().breaks == 1).await;
    let log = log.lock().unwrap();
    assert_eq!(log.writes, vec![b"enable\r".to_vec()]);
    assert_eq!(log.resizes, vec![(120, 40)]);
}

#[tokio::test]
async fn unknown_ids_are_not_found() {
    let (_backend, mgr, _out) = setup();
    assert_eq!(mgr.write(42, b"x").unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(
        mgr.resize(42, 80, 24).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(mgr.send_break(42).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(mgr.close(42).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(mgr.reconnect(42).unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn write_while_connecting_is_rejected() {
    let (_backend, mgr, out) = setup();
    let id = mgr.open(request(Some(9600), None), out.clone()).unwrap();
    // The task has not run yet (current-thread runtime, no await since open).
    let e = mgr.write(id, b"x").unwrap_err();
    assert_eq!(e.code, ErrorCode::Invalid);
    assert_eq!(e.detail.as_deref(), Some("connecting"));
    assert_eq!(mgr.send_break(id).unwrap_err().code, ErrorCode::Invalid);
}

// ---------------------------------------------------------------- close

#[tokio::test]
async fn close_disconnects_and_forgets_the_session() {
    let (backend, mgr, out) = setup();
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.close(id).unwrap();
    assert_eq!(mgr.write(id, b"x").unwrap_err().code, ErrorCode::NotFound);
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert_eq!(out.last_error(), None);
    assert_eq!(backend.transport(0).lock().unwrap().disconnects, 1);
    assert_eq!(mgr.close(id).unwrap_err().code, ErrorCode::NotFound);
    assert!(out.health().is_empty());
}

#[tokio::test]
async fn close_during_autobaud_cancels_detection() {
    let (backend, mgr, out) = setup();
    backend.st().autobaud = AutobaudScript::UntilCancelled;
    let id = mgr.open(request(None, None), out.clone()).unwrap();
    until("detecting", || out.count_state(ConnState::Detecting) == 1).await;
    mgr.close(id).unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    until("autobaud cancelled", || backend.st().autobaud_cancelled).await;
    assert_eq!(
        backend.transport_count(),
        0,
        "no transport after a cancelled open"
    );
    assert_eq!(out.count_state(ConnState::Connected), 0);
}

#[tokio::test]
async fn close_of_disconnected_session() {
    let (backend, mgr, out) = setup();
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(ErrorCode::Io))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    mgr.close(id).unwrap();
    until("closed", || out.count_state(ConnState::Disconnected) == 2).await;
    assert_eq!(mgr.reconnect(id).unwrap_err().code, ErrorCode::NotFound);
}

// ---------------------------------------------------------------- health

#[tokio::test]
async fn device_gone_disconnects_keeps_tab_and_reconnects_on_demand() {
    let (backend, mgr, out) = setup();
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    let sink = backend.sink(0);
    sink.try_send(TransportEvent::Data(b"tail".to_vec()))
        .unwrap();
    sink.try_send(TransportEvent::Closed(Some(AppError::with(
        ErrorCode::DeviceGone,
        "gone",
    ))))
    .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert_eq!(
        out.data(),
        vec![b"tail".to_vec()],
        "data before the loss is delivered"
    );
    assert_eq!(out.last_error().unwrap().code, ErrorCode::DeviceGone);
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
    assert_eq!(
        backend.transport(0).lock().unwrap().disconnects,
        1,
        "port released"
    );

    let e = mgr.write(id, b"x").unwrap_err();
    assert_eq!(
        (e.code, e.detail.as_deref()),
        (ErrorCode::Invalid, Some("disconnected"))
    );

    mgr.reconnect(id).unwrap();
    until("reconnected", || out.count_state(ConnState::Connected) == 2).await;
    assert_eq!(backend.transport_count(), 2);
    mgr.write(id, b"x").unwrap();
}

#[tokio::test]
async fn io_error_disconnects_without_cable_removed() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(ErrorCode::Io))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert!(out.health().is_empty());
}

#[tokio::test]
async fn write_failing_with_device_gone_disconnects() {
    let (backend, mgr, out) = setup();
    backend.st().write_result = Err(AppError::new(ErrorCode::DeviceGone));
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.write(id, b"x").unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
}

#[tokio::test]
async fn cable_return_triggers_auto_reconnect_with_remembered_baud() {
    let (backend, mgr, out) = setup();
    backend.st().autobaud = AutobaudScript::Result(outcome(
        Some(9600),
        None,
        vec![trial(9600, 1.0, true)],
        false,
    ));
    open_connected(&backend, &mgr, &out, None).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;

    // Unrelated cable: nothing happens.
    mgr.on_console_change(&change(
        &["usb:OTHER"],
        &[],
        vec![port("usb:OTHER", "/dev/cu.x")],
    ));
    mgr.on_console_change(&change(&[KEY], &[], vec![port(KEY, PATH)]));
    until("reconnected", || out.count_state(ConnState::Connected) == 2).await;
    assert_eq!(
        out.health(),
        vec![HealthKind::CableRemoved, HealthKind::CableReturned]
    );
    assert_eq!(
        backend.st().autobaud_calls,
        vec![None, Some(9600)],
        "reconnect tries the remembered rate first"
    );
}

#[tokio::test]
async fn cable_return_without_auto_reconnect_only_reports() {
    let (backend, mgr, out) = setup();
    backend.st().auto_reconnect = false;
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    mgr.on_console_change(&change(&[KEY], &[], vec![port(KEY, PATH)]));
    until("cable returned", || {
        out.health().contains(&HealthKind::CableReturned)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(out.count_state(ConnState::Connected), 1);
    assert_eq!(backend.transport_count(), 1);
}

#[tokio::test]
async fn cable_return_matches_by_path_when_key_differs() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    mgr.on_console_change(&change(
        &["usb:NEWSERIAL"],
        &[],
        vec![port("usb:NEWSERIAL", PATH)],
    ));
    until("reconnected", || out.count_state(ConnState::Connected) == 2).await;
}

#[tokio::test]
async fn auto_reconnect_retries_transient_open_errors() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    backend
        .st()
        .connect_results
        .push_back(Err(AppError::new(ErrorCode::PortNotFound)));
    mgr.on_console_change(&change(&[KEY], &[], vec![port(KEY, PATH)]));
    until("reconnected", || out.count_state(ConnState::Connected) == 2).await;
    assert_eq!(
        out.count_state(ConnState::Disconnected),
        1,
        "no flicker between attempts"
    );
    assert_eq!(backend.transport_count(), 3);
}

#[tokio::test]
async fn hotplug_detach_with_silent_reader_disconnects_after_grace() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.on_console_change(&change(&[], &[KEY], vec![]));
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    assert_eq!(out.last_error().unwrap().code, ErrorCode::DeviceGone);
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
    // A late reader failure and repeated hotplug events produce no duplicates.
    let _ = backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))));
    mgr.on_console_change(&change(&[], &[KEY], vec![]));
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
    assert_eq!(out.count_state(ConnState::Disconnected), 1);
}

#[tokio::test]
async fn reader_report_before_hotplug_detach_is_not_duplicated() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::new(
            ErrorCode::DeviceGone,
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    mgr.on_console_change(&change(&[], &[KEY], vec![]));
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
    assert_eq!(out.count_state(ConnState::Disconnected), 1);
}

#[tokio::test]
async fn reader_report_within_grace_wins() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.on_console_change(&change(&[], &[KEY], vec![]));
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(Some(AppError::with(
            ErrorCode::DeviceGone,
            "reader",
        ))))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(out.last_error().unwrap().detail.as_deref(), Some("reader"));
    assert_eq!(out.health(), vec![HealthKind::CableRemoved]);
    assert_eq!(out.count_state(ConnState::Disconnected), 1);
}

#[tokio::test]
async fn dropping_the_manager_ends_tasks() {
    let (backend, mgr, out) = setup();
    open_connected(&backend, &mgr, &out, Some(9600)).await;
    drop(mgr);
    let log = backend.transport(0);
    until("transport released", || {
        log.lock().unwrap().disconnects == 1
    })
    .await;
}

// ---------------------------------------------------------------- network auto reconnect

fn net_request() -> OpenRequest {
    OpenRequest {
        session_id: None,
        target: OpenTarget::Network {
            protocol: crate::model::Protocol::Telnet,
            host: "10.0.0.5".into(),
            port: Some(2001),
            username: None,
            ssh: crate::model::SshOptions::default(),
        },
        cols: 80,
        rows: 24,
    }
}

async fn net_drop(auto: bool, err: Option<AppError>) -> (Arc<FakeBackend>, Arc<Recorder>) {
    let (backend, mgr, out) = setup();
    backend.st().net_auto_reconnect = auto;
    mgr.open(net_request(), out.clone()).unwrap();
    until("connected", || out.count_state(ConnState::Connected) == 1).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Closed(err))
        .unwrap();
    until("disconnected", || {
        out.count_state(ConnState::Disconnected) >= 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::mem::forget(mgr); // keep the session task alive for the assertions
    (backend, out)
}

#[tokio::test]
async fn network_drop_reconnects_automatically_when_enabled() {
    let (backend, out) =
        net_drop(true, Some(AppError::with(ErrorCode::Timeout, "keepalive"))).await;
    until("reconnected", || out.count_state(ConnState::Connected) == 2).await;
    assert_eq!(backend.transport_count(), 2);
}

#[tokio::test]
async fn network_drop_stays_disconnected_when_disabled_or_clean_or_auth() {
    let (backend, out) = net_drop(false, Some(AppError::with(ErrorCode::Timeout, "x"))).await;
    assert_eq!(out.count_state(ConnState::Connected), 1);
    assert_eq!(backend.transport_count(), 1);

    // Remote side logged us out (clean EOF): never reconnect behind the user's back.
    let (backend, out) = net_drop(true, None).await;
    assert_eq!(out.count_state(ConnState::Connected), 1);
    assert_eq!(backend.transport_count(), 1);
}

// ---------------------------------------------------------------- moving between windows (P2-T19)

#[tokio::test]
async fn parked_session_moves_to_a_new_output_without_losing_bytes() {
    let (backend, mgr, out) = setup();
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    mgr.set_owner(id, "main").unwrap();
    let sink = backend.sink(0);
    sink.try_send(TransportEvent::Data(b"before".to_vec()))
        .unwrap();
    until("first data", || !out.data().is_empty()).await;

    mgr.park(id).unwrap();
    assert!(matches!(out.events().last(), Some(SessionEvent::Parked)));
    assert_eq!(
        out.data().last(),
        Some(&Vec::new()),
        "empty batch = end marker"
    );
    sink.try_send(TransportEvent::Data(b"while moving".to_vec()))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let moved = Arc::new(Recorder::default());
    mgr.attach(id, moved.clone(), "session-1").unwrap();
    sink.try_send(TransportEvent::Data(b" after".to_vec()))
        .unwrap();
    until("data in new window", || {
        moved.data().concat().ends_with(b" after")
    })
    .await;
    assert_eq!(moved.data().concat(), b"while moving after");
    assert_eq!(
        out.data().concat(),
        b"before",
        "old window got nothing more"
    );

    // Input still reaches the device, and the session now dies with its new window.
    mgr.write(id, b"show ver\r").unwrap();
    assert_eq!(mgr.close_owned_by("main"), 0);
    assert_eq!(mgr.close_owned_by("session-1"), 1);
    assert_eq!(mgr.park(id).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(
        mgr.attach(id, moved, "x").unwrap_err().code,
        ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------- identity command (P3-T04)

#[tokio::test]
async fn identity_command_runs_hidden_at_a_quiet_prompt_and_holds_input() {
    let (backend, mgr, out) = setup();
    backend.st().identify = true;
    let id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    let sink = backend.sink(0);
    let log = backend.transport(0);
    sink.try_send(TransportEvent::Data(b"\r\naccess-sw-02#".to_vec()))
        .unwrap();
    until("show version sent", || {
        log.lock()
            .unwrap()
            .writes
            .iter()
            .any(|w| w == b"show version\r")
    })
    .await;
    // Typed while the command runs: held, not interleaved.
    mgr.write(id, b"x").unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!log.lock().unwrap().writes.iter().any(|w| w == b"x"));
    sink.try_send(TransportEvent::Data(
        b"show version\r\nSystem description      : Ruijie Switch(S5750-48GT4XS-E) By Ruijie Networks\r\n\
          System software version : S5750_RGOS 11.4(1)B12P10S2\r\n --More-- "
            .to_vec(),
    ))
    .unwrap();
    until("next page", || {
        log.lock().unwrap().writes.iter().any(|w| w == b" ")
    })
    .await;
    sink.try_send(TransportEvent::Data(
        b"\r          \rSystem serial number    : G1MQ3K8000123\r\naccess-sw-02#".to_vec(),
    ))
    .unwrap();
    until("identity with facts", || {
        out.events().iter().any(|e| {
            matches!(e, SessionEvent::Identity { facts: Some(f), vendor: Some(v), .. }
                if f.serial.as_deref() == Some("G1MQ3K8000123") && v.id == "ruijie-rgos")
        })
    })
    .await;
    until("held input sent", || {
        log.lock().unwrap().writes.iter().any(|w| w == b"x")
    })
    .await;
    let shown = String::from_utf8_lossy(&out.data().concat()).into_owned();
    assert!(shown.contains("access-sw-02#"));
    assert!(
        !shown.contains("show version"),
        "the command's output stays hidden"
    );
    assert!(!shown.contains("G1MQ3K8000123"));
    let devices = backend.st().devices.clone();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].0, "ruijie-rgos");
    assert_eq!(devices[0].1.model.as_deref(), Some("S5750-48GT4XS-E"));
}

#[tokio::test]
async fn identity_command_is_off_when_disabled() {
    let (backend, mgr, out) = setup();
    let _id = open_connected(&backend, &mgr, &out, Some(9600)).await;
    backend
        .sink(0)
        .try_send(TransportEvent::Data(b"\r\nsw1#".to_vec()))
        .unwrap();
    tokio::time::sleep(crate::device::probe::QUIET + Duration::from_millis(200)).await;
    assert!(backend.transport(0).lock().unwrap().writes.is_empty());
}
