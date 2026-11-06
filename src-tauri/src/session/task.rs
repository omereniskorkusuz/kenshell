//! The per-session task: open flow (baud resolution, autobaud, connect), the
//! connected loop (batched output, commands, health) and the disconnected wait.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::mpsc::{self, Receiver, UnboundedReceiver};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::backend::Backend;
use super::{Registry, SessionOutput, Timing};
use crate::console_detect::autobaud::Outcome;
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{
    BaudTrial, ConnState, HealthKind, OpenTarget, PromptRequest, RuntimeId, SerialLink,
    SerialSettings, SessionEvent,
};
use crate::transport::{Prompter, Transport, TransportEvent};

/// Baud used when autobaud heard nothing and nothing is remembered.
pub(crate) const FALLBACK_BAUD: u32 = 9600;

/// Capacity of the transport → session queue (each item ≤ 64 KiB).
pub(crate) const TRANSPORT_QUEUE: usize = 64;

/// Upper bound for one `SessionOutput::data` call.
pub(crate) const MAX_UI_BATCH: usize = 256 * 1024;

/// Auto-reconnect attempts after a cable returns (the OS may need a moment before
/// the new port can be opened).
pub(crate) const AUTO_RECONNECT_ATTEMPTS: u32 = 3;

pub(crate) enum Command {
    Write(Vec<u8>),
    Resize(u16, u16),
    Break,
    Close,
    Reconnect,
    /// The session's cable re-appeared (hotplug).
    PortAttached,
    /// The session's cable vanished (hotplug).
    PortDetached,
    /// Answers to the pending login prompt (secrets: never logged).
    Answer(Vec<String>),
    /// Run a read-only command hidden from the terminal and the log (P4-T06).
    Capture(CaptureRequest),
}

/// A hidden read-only command (diagnostics, config backup …), queued until the device
/// sits at a quiet user/privileged prompt.
pub(crate) struct CaptureRequest {
    pub command: String,
    pub timeout: Duration,
    pub reply: oneshot::Sender<Result<String>>,
}

struct RunningCapture {
    capture: crate::device::capture::Capture,
    timeout: Duration,
    reply: oneshot::Sender<Result<String>>,
}

/// How long a capture may wait in the queue for a prompt.
const CAPTURE_QUEUE_WAIT: Duration = Duration::from_secs(20);
/// Output kept per capture (a large `show running-config` fits).
const CAPTURE_MAX_BYTES: usize = 4 * 1024 * 1024;
const CAPTURE_MAX_PAGES: u32 = 10_000;

type PromptSlot = Arc<Mutex<Option<oneshot::Sender<Vec<String>>>>>;

/// Login questions go to the UI as `SessionEvent::Prompt`; the answer comes back as
/// `Command::Answer` and is handed to the waiting transport through the slot.
struct TaskPrompter {
    out: Arc<dyn SessionOutput>,
    slot: PromptSlot,
}

#[async_trait::async_trait]
impl Prompter for TaskPrompter {
    async fn ask(&self, request: PromptRequest) -> Option<Vec<String>> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut s) = self.slot.lock() {
            *s = Some(tx);
        }
        self.out.event(SessionEvent::Prompt { request });
        rx.await.ok()
    }
}

/// State visible to the manager without asking the task.
pub(crate) struct Shared {
    state: AtomicU8,
    /// Port path the session connects to.
    pub path: String,
    /// Cable identity (`Backend::port_key`), cached at open.
    pub port_key: Mutex<Option<String>>,
    /// The live SSH connection while connected (SFTP / SCP channels, P5-T01).
    pub ssh: Mutex<Option<Arc<russh::client::Handle<crate::transport::ssh::Client>>>>,
    /// Session recording while on (P5-T06); output is written to it as it is shown.
    pub recording: Mutex<Option<crate::logging::record::Recording>>,
}

impl Shared {
    pub fn new(path: String) -> Self {
        Self {
            state: AtomicU8::new(encode(ConnState::Connecting)),
            path,
            port_key: Mutex::new(None),
            ssh: Mutex::new(None),
            recording: Mutex::new(None),
        }
    }

    pub fn state(&self) -> ConnState {
        decode(self.state.load(Ordering::Acquire))
    }

    fn set(&self, s: ConnState) {
        self.state.store(encode(s), Ordering::Release);
    }

    pub fn port_key(&self) -> Option<String> {
        self.port_key.lock().ok().and_then(|k| k.clone())
    }
}

fn encode(s: ConnState) -> u8 {
    match s {
        ConnState::Connecting => 0,
        ConnState::Detecting => 1,
        ConnState::Connected => 2,
        ConnState::Disconnected => 3,
    }
}

fn decode(v: u8) -> ConnState {
    match v {
        0 => ConnState::Connecting,
        1 => ConnState::Detecting,
        2 => ConnState::Connected,
        _ => ConnState::Disconnected,
    }
}

/// How the baud rate was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaudPlan {
    pub baud: u32,
    pub auto_detected: bool,
    /// Persist the rate for this cable once connected.
    pub remember: bool,
}

/// Accepted → use + remember; suggestion → use, don't remember; nothing → remembered
/// or [`FALLBACK_BAUD`].
pub(crate) fn plan_from_outcome(outcome: &Outcome, remembered: Option<u32>) -> BaudPlan {
    if let Some(baud) = outcome.accepted {
        BaudPlan {
            baud,
            auto_detected: true,
            remember: true,
        }
    } else if let Some(baud) = outcome.suggestion {
        BaudPlan {
            baud,
            auto_detected: true,
            remember: false,
        }
    } else {
        BaudPlan {
            baud: remembered.unwrap_or(FALLBACK_BAUD),
            auto_detected: false,
            remember: false,
        }
    }
}

type Link = (Box<dyn Transport>, Receiver<TransportEvent>);

enum Phase {
    Connected(Link),
    Disconnected,
}

/// Why a session (re)opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reopen {
    /// First open or the user asked (Enter / button): one attempt.
    Manual,
    /// The console cable returned: a few quick attempts.
    Cable,
    /// A network link dropped and `connection.autoReconnect` is on: backoff attempts.
    Network,
}

enum Next {
    Disconnected,
    Reopen(Reopen),
    Exit,
}

enum OpenResult {
    Ok(Link),
    Err(AppError),
    Cancelled,
}

/// Everything the open flow needs; borrowed while the command queue is polled.
struct Ctx {
    id: RuntimeId,
    session_id: Option<i64>,
    target: OpenTarget,
    out: Arc<dyn SessionOutput>,
    backend: Arc<dyn Backend>,
    shared: Arc<Shared>,
    timing: Timing,
    /// Latest terminal size (network transports need it at connect time).
    size: Mutex<(u16, u16)>,
    prompt: PromptSlot,
}

impl Ctx {
    fn set_state(&self, state: ConnState, error: Option<AppError>) {
        tracing::debug!(session = self.id, ?state, error = ?error.as_ref().map(|e| e.code), "session state");
        self.shared.set(state);
        self.out.event(SessionEvent::State { state, error });
    }

    fn health(&self, kind: HealthKind) {
        tracing::debug!(session = self.id, ?kind, "session health");
        self.out.event(SessionEvent::Health { kind });
    }

    /// Runs a backend call on the blocking pool.
    async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&dyn Backend) -> T + Send + 'static,
    {
        let backend = Arc::clone(&self.backend);
        tokio::task::spawn_blocking(move || f(backend.as_ref()))
            .await
            .map_err(|e| AppError::internal(format!("blocking task: {e}")))
    }

    async fn open(&self, settle: Option<Duration>, cancel: Arc<AtomicBool>) -> Result<Link> {
        if let Some(d) = settle {
            tokio::time::sleep(d).await;
        }
        self.set_state(ConnState::Connecting, None);
        let settings = match &self.target {
            OpenTarget::Serial { settings } => settings,
            OpenTarget::Network { .. } => return self.open_network().await,
        };
        let path = settings.port.clone();

        let p = path.clone();
        let (remembered, key) = self
            .call(move |b| (b.remembered_baud(&p), b.port_key(&p)))
            .await?;
        if let Ok(mut k) = self.shared.port_key.lock() {
            *k = Some(key);
        }

        let plan = match settings.baud {
            Some(baud) => BaudPlan {
                baud,
                auto_detected: false,
                remember: true,
            },
            None => self.autobaud(settings, remembered, cancel).await?,
        };
        if plan.baud == 0 {
            return Err(AppError::invalid("baud rate must be > 0"));
        }

        let (tx, rx) = mpsc::channel(TRANSPORT_QUEUE);
        let mut transport = self.backend.make_transport(settings, plan.baud, tx);
        transport.connect().await?;

        let p = path.clone();
        let (display_name, chip) = self
            .call(move |b| {
                if plan.remember
                    && let Err(e) = b.remember(&p, plan.baud)
                {
                    tracing::warn!("remember baud for {p}: {e}");
                }
                (b.display_name(&p), b.chip_label(&p))
            })
            .await?;
        if let Some(sid) = self.session_id {
            let touched = self.call(move |b| b.touch_session(sid)).await;
            if let Err(e) | Ok(Err(e)) = touched {
                tracing::warn!("touch session {sid}: {e}");
            }
        }

        self.out.event(SessionEvent::Link {
            link: SerialLink {
                path,
                display_name,
                baud: plan.baud,
                framing: settings.framing(),
                chip,
                auto_detected: plan.auto_detected,
            },
        });
        self.set_state(ConnState::Connected, None);
        Ok((transport, rx))
    }

    async fn open_network(&self) -> Result<Link> {
        let (tx, rx) = mpsc::channel(TRANSPORT_QUEUE);
        let prompter = Arc::new(TaskPrompter {
            out: Arc::clone(&self.out),
            slot: Arc::clone(&self.prompt),
        });
        let size = self.size.lock().map(|s| *s).unwrap_or((80, 24));
        let creds = match self.session_id {
            Some(sid) => self.call(move |b| b.credentials(sid)).await?,
            None => None,
        };
        let mut transport =
            self.backend
                .make_network_transport(&self.target, size, tx, prompter, creds)?;
        transport.connect().await?;
        // Size may have changed while we were logging in.
        let now = self.size.lock().map(|s| *s).unwrap_or(size);
        if now != size {
            let _ = transport.resize(now.0, now.1).await;
        }
        if let Some(sid) = self.session_id {
            let touched = self.call(move |b| b.touch_session(sid)).await;
            if let Err(e) | Ok(Err(e)) = touched {
                tracing::warn!("touch session {sid}: {e}");
            }
        }
        if let Some(link) = transport.net_link() {
            self.out.event(SessionEvent::NetLink { link });
        }
        self.set_state(ConnState::Connected, None);
        Ok((transport, rx))
    }

    async fn autobaud(
        &self,
        settings: &SerialSettings,
        remembered: Option<u32>,
        cancel: Arc<AtomicBool>,
    ) -> Result<BaudPlan> {
        self.set_state(ConnState::Detecting, None);
        let out = Arc::clone(&self.out);
        let s = settings.clone();
        let outcome = self
            .call(move |b| {
                let mut trials: Vec<BaudTrial> = Vec::new();
                b.run_autobaud(&s, remembered, &cancel, &mut |t: &BaudTrial| {
                    trials.push(t.clone());
                    out.event(SessionEvent::Autobaud {
                        trials: trials.clone(),
                        current: Some(t.baud),
                        result: None,
                        suggestion: None,
                    });
                })
            })
            .await??;
        tracing::debug!(
            session = self.id,
            accepted = ?outcome.accepted,
            suggestion = ?outcome.suggestion,
            no_data = outcome.no_data,
            "autobaud finished"
        );
        self.out.event(SessionEvent::Autobaud {
            trials: outcome.trials.clone(),
            current: None,
            result: outcome.accepted,
            suggestion: outcome.suggestion,
        });
        if outcome.no_data {
            self.health(HealthKind::NoData);
        }
        Ok(plan_from_outcome(&outcome, remembered))
    }
}

/// Network failures worth an automatic reconnect (never auth or host-key problems).
fn net_retryable(e: &AppError) -> bool {
    matches!(
        e.code,
        ErrorCode::Timeout
            | ErrorCode::ConnectionClosed
            | ErrorCode::ConnectionRefused
            | ErrorCode::HostNotFound
            | ErrorCode::Io
    )
}

/// Transient open errors worth another automatic attempt.
fn retryable(e: &AppError) -> bool {
    matches!(
        e.code,
        ErrorCode::PortNotFound | ErrorCode::PortBusy | ErrorCode::PermissionDenied | ErrorCode::Io
    )
}

pub(crate) struct SessionTask {
    ctx: Ctx,
    cmds: UnboundedReceiver<Command>,
    registry: Weak<Registry>,
    /// Mode / identity / reload from the output (keeps no output itself, P3-T03).
    observer: crate::device::observer::Observer,
    /// Background identity command (P3-T04).
    prober: crate::device::probe::Prober,
    /// User input that arrived while the identity command ran (sent afterwards).
    held: Vec<Vec<u8>>,
    /// Hidden read-only commands waiting for a prompt, and the one running (P4-T06).
    captures: std::collections::VecDeque<(CaptureRequest, std::time::Instant)>,
    capture: Option<RunningCapture>,
}

impl SessionTask {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: RuntimeId,
        session_id: Option<i64>,
        target: OpenTarget,
        out: Arc<dyn SessionOutput>,
        backend: Arc<dyn Backend>,
        shared: Arc<Shared>,
        timing: Timing,
        size: (u16, u16),
        cmds: UnboundedReceiver<Command>,
        registry: Weak<Registry>,
    ) -> Self {
        Self {
            ctx: Ctx {
                id,
                session_id,
                target,
                out,
                backend,
                shared,
                timing,
                size: Mutex::new(size),
                prompt: Arc::new(Mutex::new(None)),
            },
            cmds,
            registry,
            observer: crate::device::observer::Observer::new(),
            prober: crate::device::probe::Prober::new(false),
            held: Vec::new(),
            captures: std::collections::VecDeque::new(),
            capture: None,
        }
    }

    pub async fn run(mut self) {
        let mut next = Next::Reopen(Reopen::Manual);
        loop {
            next = match next {
                Next::Reopen(why) => match self.connect(why).await {
                    Some(Phase::Connected(link)) => {
                        if let Ok(mut slot) = self.ctx.shared.ssh.lock() {
                            *slot = link.0.ssh_handle();
                        }
                        let next = self.connected(link).await;
                        if let Ok(mut slot) = self.ctx.shared.ssh.lock() {
                            *slot = None;
                        }
                        next
                    }
                    Some(Phase::Disconnected) => Next::Disconnected,
                    None => Next::Exit,
                },
                Next::Disconnected => self.disconnected().await,
                Next::Exit => break,
            };
        }
        if let Some(reg) = self.registry.upgrade()
            && let Ok(mut map) = reg.lock()
        {
            map.remove(&self.ctx.id);
        }
        tracing::debug!(session = self.ctx.id, "session task ended");
    }

    /// Open flow, cancellable by `Close` at any point. `None` = closed.
    async fn connect(&mut self, why: Reopen) -> Option<Phase> {
        let net_delays = self.ctx.timing.net_retry;
        let attempts = match why {
            Reopen::Manual => 1,
            Reopen::Cable => AUTO_RECONNECT_ATTEMPTS,
            Reopen::Network => net_delays.len() as u32,
        };
        for attempt in 1..=attempts {
            let settle = match why {
                Reopen::Manual => None,
                Reopen::Cable => Some(self.ctx.timing.reconnect_settle),
                Reopen::Network => net_delays.get(attempt as usize - 1).copied(),
            };
            match self.open_cancellable(settle).await {
                OpenResult::Ok(link) => return Some(Phase::Connected(link)),
                OpenResult::Cancelled => {
                    self.ctx.set_state(ConnState::Disconnected, None);
                    return None;
                }
                OpenResult::Err(e)
                    if attempt < attempts
                        && (retryable(&e) || (why == Reopen::Network && net_retryable(&e))) =>
                {
                    tracing::debug!(session = self.ctx.id, attempt, code = ?e.code, "auto reconnect retry");
                }
                OpenResult::Err(e) => {
                    self.ctx.set_state(ConnState::Disconnected, Some(e));
                    return Some(Phase::Disconnected);
                }
            }
        }
        unreachable!("loop returns on the last attempt")
    }

    async fn open_cancellable(&mut self, settle: Option<Duration>) -> OpenResult {
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = &self.ctx;
        let cmds = &mut self.cmds;
        let open = ctx.open(settle, Arc::clone(&cancel));
        tokio::pin!(open);
        loop {
            tokio::select! {
                res = &mut open => {
                    return match res {
                        Ok(link) => OpenResult::Ok(link),
                        Err(e) => OpenResult::Err(e),
                    };
                }
                cmd = cmds.recv() => match cmd {
                    None | Some(Command::Close) => {
                        // Dropping `open` drops a half-open transport; the flag stops a
                        // running autobaud at its next candidate.
                        cancel.store(true, Ordering::Release);
                        if let Ok(mut slot) = ctx.prompt.lock() {
                            slot.take(); // a pending login prompt resolves as cancelled
                        }
                        return OpenResult::Cancelled;
                    }
                    Some(Command::Answer(answers)) => {
                        let tx = ctx.prompt.lock().ok().and_then(|mut s| s.take());
                        if let Some(tx) = tx {
                            let _ = tx.send(answers);
                        }
                    }
                    Some(Command::Resize(cols, rows)) => {
                        if let Ok(mut s) = ctx.size.lock() {
                            *s = (cols, rows);
                        }
                    }
                    // Not connected yet: the manager rejects writes/breaks; anything
                    // racing in is dropped. Reconnect/hotplug are moot while opening.
                    Some(_) => {}
                },
            }
        }
    }

    /// Opens the session log if logging is enabled (never fails the session).
    async fn open_log(&self) -> Option<crate::logging::SessionLog> {
        let path = self.ctx.shared.path.clone();
        let serial = matches!(self.ctx.target, OpenTarget::Serial { .. });
        let res = self
            .ctx
            .call(move |b| {
                let cfg = b.log_config()?;
                let name = if serial { b.display_name(&path) } else { path };
                Some(crate::logging::SessionLog::open(&cfg, &name))
            })
            .await;
        match res {
            Ok(Some(Ok(log))) => {
                self.ctx.out.event(SessionEvent::Logging {
                    path: Some(log.path().display().to_string()),
                    error: None,
                });
                Some(log)
            }
            Ok(Some(Err(e))) | Err(e) => {
                tracing::warn!(session = self.ctx.id, "session log: {e}");
                self.ctx.out.event(SessionEvent::Logging {
                    path: None,
                    error: Some(e),
                });
                None
            }
            Ok(None) => None,
        }
    }

    /// Output for the terminal: logged, observed (mode / identity), sent to the UI.
    fn show(&mut self, buf: Vec<u8>, log: Option<&crate::logging::SessionLog>) {
        if let Some(log) = log {
            log.write(&buf);
        }
        if let Ok(r) = self.ctx.shared.recording.lock()
            && let Some(r) = r.as_ref()
        {
            r.output(&buf);
        }
        let seen = self.observer.feed(&buf);
        self.ctx.out.data(buf);
        for o in seen {
            self.ctx.out.event(observation_event(o));
        }
        self.prober
            .after_output(self.observer.at_prompt(), std::time::Instant::now());
    }

    /// Starts the next queued capture when the device is quietly at a prompt.
    async fn start_capture(&mut self, transport: &mut Box<dyn crate::transport::Transport>) {
        use crate::device::modes::CliMode;
        if self.capture.is_some()
            || self.prober.capturing()
            || !matches!(
                self.observer.at_prompt(),
                Some(CliMode::User | CliMode::Privileged)
            )
        {
            return;
        }
        let Some((req, _)) = self.captures.pop_front() else {
            return;
        };
        let line = format!("{}\r", req.command.trim());
        if let Err(e) = transport.write(line.as_bytes()).await {
            let _ = req.reply.send(Err(e));
            return;
        }
        self.capture = Some(RunningCapture {
            capture: crate::device::capture::Capture::new(
                std::time::Instant::now(),
                CAPTURE_MAX_BYTES,
                CAPTURE_MAX_PAGES,
            ),
            timeout: req.timeout,
            reply: req.reply,
        });
    }

    /// When the capture timer must fire: the running one's time-out or the queue wait.
    fn capture_deadline(&self) -> Option<std::time::Instant> {
        match &self.capture {
            Some(c) => Some(c.capture.started() + c.timeout),
            None => self
                .captures
                .front()
                .map(|(_, at)| *at + CAPTURE_QUEUE_WAIT),
        }
    }

    /// Input typed while the identity command ran.
    async fn flush_held(&mut self, transport: &mut Box<dyn crate::transport::Transport>) {
        for data in std::mem::take(&mut self.held) {
            if let Err(e) = transport.write(&data).await {
                tracing::debug!(session = self.ctx.id, "held write: {e}");
                break;
            }
        }
    }

    /// The identity command's output: vendor (may refine the profile), facts, stored device.
    async fn identified(&mut self, text: String) {
        use crate::device::observer::VendorInfo;
        let mut seen = Vec::new();
        if let Some(p) = crate::device::modes::identify(&text) {
            self.observer.adopt(p, &mut seen);
        }
        for o in seen {
            self.ctx.out.event(observation_event(o));
        }
        let profile = self.observer.profile();
        let facts = profile.parse_facts(&text);
        if facts.is_empty() {
            return;
        }
        let (sid, vendor) = (self.ctx.session_id, profile.id.clone());
        let stored = facts.clone();
        let device_id = self
            .ctx
            .call(move |b| b.record_device(sid, &vendor, &stored))
            .await
            .ok()
            .flatten();
        self.ctx.out.event(SessionEvent::Identity {
            hostname: facts.hostname.clone(),
            vendor: Some(VendorInfo::of(profile)),
            facts: Some(facts),
            device_id,
        });
    }

    async fn connected(&mut self, link: Link) -> Next {
        let (mut transport, mut rx) = link;
        let log = self.open_log().await;
        self.observer.reconnected();
        let sid = self.ctx.session_id;
        let identify = self
            .ctx
            .call(move |b| b.identify_enabled(sid))
            .await
            .unwrap_or(false);
        self.prober = crate::device::probe::Prober::new(identify);
        self.held.clear();
        let mut carry: Option<TransportEvent> = None;
        let mut detach_deadline: Option<Instant> = None;
        loop {
            enum Ev {
                Transport(Option<TransportEvent>),
                Cmd(Option<Command>),
                DetachTimeout,
                Probe,
                CaptureTimer,
            }
            let ev = if let Some(ev) = carry.take() {
                Ev::Transport(Some(ev))
            } else {
                let deadline = detach_deadline;
                let probe_at = self.prober.deadline().map(Instant::from_std);
                let capture_at = self.capture_deadline().map(Instant::from_std);
                tokio::select! {
                    ev = rx.recv() => Ev::Transport(ev),
                    cmd = self.cmds.recv() => Ev::Cmd(cmd),
                    _ = async {
                        match probe_at {
                            Some(d) => tokio::time::sleep_until(d).await,
                            None => std::future::pending().await,
                        }
                    } => Ev::Probe,
                    _ = async {
                        match capture_at {
                            Some(d) => tokio::time::sleep_until(d).await,
                            None => std::future::pending().await,
                        }
                    } => Ev::CaptureTimer,
                    _ = async {
                        match deadline {
                            Some(d) => tokio::time::sleep_until(d).await,
                            None => std::future::pending().await,
                        }
                    } => Ev::DetachTimeout,
                }
            };
            match ev {
                Ev::Transport(Some(TransportEvent::Data(mut buf))) => {
                    // Batch whatever else is already queued into one UI message.
                    while buf.len() < MAX_UI_BATCH {
                        match rx.try_recv() {
                            Ok(TransportEvent::Data(more))
                                if buf.len() + more.len() <= MAX_UI_BATCH =>
                            {
                                buf.extend_from_slice(&more);
                            }
                            Ok(other) => {
                                carry = Some(other);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    if self.prober.capturing() {
                        // The identity command's output is never shown or logged.
                        use crate::device::probe::Step;
                        match self.prober.capture(&buf, self.observer.profile()) {
                            Step::Continue => {}
                            Step::NextPage => {
                                let _ = transport.write(b" ").await;
                            }
                            Step::Finished(text) => {
                                self.identified(text).await;
                                self.flush_held(&mut transport).await;
                                self.start_capture(&mut transport).await;
                            }
                        }
                        continue;
                    }
                    if let Some(run) = self.capture.as_mut() {
                        // A requested read-only command: hidden, not logged (P4-T06).
                        use crate::device::capture::Step;
                        match run.capture.feed(&buf, self.observer.profile()) {
                            Step::Continue => {}
                            Step::NextPage => {
                                let _ = transport.write(b" ").await;
                            }
                            Step::Finished(text) => {
                                if let Some(run) = self.capture.take() {
                                    let _ = run.reply.send(Ok(text));
                                }
                                self.flush_held(&mut transport).await;
                                self.start_capture(&mut transport).await;
                            }
                        }
                        continue;
                    }
                    self.show(buf, log.as_ref());
                    self.start_capture(&mut transport).await;
                    // P3-T14: answer the pager when auto-continue is on (read live, so
                    // the setting applies at once; only when a pager prompt shows).
                    if self.observer.at_more()
                        && self
                            .ctx
                            .call(|b| b.auto_more_enabled())
                            .await
                            .unwrap_or(false)
                        && let Err(e) = transport.write(b" ").await
                    {
                        tracing::debug!(session = self.ctx.id, "auto more: {e}");
                    }
                }
                Ev::Transport(Some(TransportEvent::Closed(err))) => {
                    return self.link_lost(transport, err).await;
                }
                Ev::Transport(None) => {
                    let err = AppError::with(ErrorCode::Io, "transport ended unexpectedly");
                    return self.link_lost(transport, Some(err)).await;
                }
                Ev::CaptureTimer => {
                    let now = std::time::Instant::now();
                    let expired = self
                        .capture
                        .as_ref()
                        .is_some_and(|c| now >= c.capture.started() + c.timeout);
                    if expired && let Some(run) = self.capture.take() {
                        let _ = run.reply.send(Err(AppError::with(
                            ErrorCode::Timeout,
                            "the device did not return to its prompt",
                        )));
                        // Show what it said after all.
                        self.show(run.capture.into_raw(), log.as_ref());
                        self.flush_held(&mut transport).await;
                    } else if self.capture.is_none()
                        && self
                            .captures
                            .front()
                            .is_some_and(|(_, at)| now >= *at + CAPTURE_QUEUE_WAIT)
                        && let Some((req, _)) = self.captures.pop_front()
                    {
                        let _ = req.reply.send(Err(AppError::with(
                            ErrorCode::Timeout,
                            "the device is not at a prompt",
                        )));
                    }
                    self.start_capture(&mut transport).await;
                }
                Ev::Probe if self.capture.is_some() => {
                    // Not while a requested command runs: wait for the next quiet prompt.
                    self.prober.user_input();
                }
                Ev::Probe => {
                    let now = std::time::Instant::now();
                    if let Some(raw) = self.prober.time_out(now) {
                        // The device never returned to a prompt: show what it said.
                        tracing::debug!(session = self.ctx.id, "identity command timed out");
                        self.show(raw, log.as_ref());
                        self.flush_held(&mut transport).await;
                    } else if let Some(cmd) =
                        self.prober
                            .fire(self.observer.at_prompt(), self.observer.profile(), now)
                        && let Err(e) = transport.write(&cmd).await
                    {
                        tracing::debug!(session = self.ctx.id, "identity command: {e}");
                    }
                }
                Ev::DetachTimeout => {
                    // Hotplug saw the cable go but the reader never failed (some
                    // Windows drivers just time out forever on a dead handle).
                    let err = AppError::with(
                        ErrorCode::DeviceGone,
                        format!("{}: console cable removed", self.ctx.shared.path),
                    );
                    return self.link_lost(transport, Some(err)).await;
                }
                Ev::Cmd(None) => {
                    let _ = transport.disconnect().await;
                    return Next::Exit;
                }
                Ev::Cmd(Some(cmd)) => match cmd {
                    Command::Capture(req) => {
                        if crate::device::capture::read_only(&req.command) {
                            self.captures.push_back((req, std::time::Instant::now()));
                            self.start_capture(&mut transport).await;
                        } else {
                            let _ = req.reply.send(Err(AppError::invalid(
                                "only read-only show/display commands",
                            )));
                        }
                    }
                    Command::Write(data) if self.prober.capturing() || self.capture.is_some() => {
                        self.held.push(data);
                    }
                    Command::Write(data) => {
                        self.prober.user_input();
                        if let Err(e) = transport.write(&data).await {
                            if e.code == ErrorCode::DeviceGone {
                                return self.link_lost(transport, Some(e)).await;
                            }
                            // Never log `data` (may contain passwords).
                            tracing::warn!(session = self.ctx.id, "write failed: {e}");
                        }
                    }
                    Command::Resize(cols, rows) => {
                        if let Ok(mut s) = self.ctx.size.lock() {
                            *s = (cols, rows);
                        }
                        if let Err(e) = transport.resize(cols, rows).await {
                            tracing::debug!(session = self.ctx.id, "resize: {e}");
                        }
                    }
                    Command::Break => {
                        if let Err(e) = transport.send_break().await {
                            if e.code == ErrorCode::DeviceGone {
                                return self.link_lost(transport, Some(e)).await;
                            }
                            tracing::warn!(session = self.ctx.id, "break failed: {e}");
                        }
                    }
                    Command::Close => {
                        if let Err(e) = transport.disconnect().await {
                            tracing::debug!(session = self.ctx.id, "disconnect: {e}");
                        }
                        self.ctx.set_state(ConnState::Disconnected, None);
                        return Next::Exit;
                    }
                    Command::PortDetached => {
                        if detach_deadline.is_none() {
                            detach_deadline = Some(Instant::now() + self.ctx.timing.detach_grace);
                        }
                    }
                    Command::Reconnect | Command::PortAttached | Command::Answer(_) => {}
                },
            }
        }
    }

    /// The link failed (or closed): release the port, report, keep the tab.
    async fn link_lost(
        &mut self,
        mut transport: Box<dyn Transport>,
        err: Option<AppError>,
    ) -> Next {
        if let Err(e) = transport.disconnect().await {
            tracing::debug!(session = self.ctx.id, "disconnect after loss: {e}");
        }
        drop(transport);
        let gone = err
            .as_ref()
            .is_some_and(|e| e.code == ErrorCode::DeviceGone);
        let net_retry = matches!(self.ctx.target, OpenTarget::Network { .. })
            && err.as_ref().is_some_and(net_retryable);
        self.ctx.set_state(ConnState::Disconnected, err);
        if gone {
            self.ctx.health(HealthKind::CableRemoved);
        }
        if net_retry {
            match self.ctx.call(|b| b.net_auto_reconnect_enabled()).await {
                Ok(true) => {
                    tracing::debug!(session = self.ctx.id, "network auto reconnect");
                    return Next::Reopen(Reopen::Network);
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("auto reconnect setting: {e}"),
            }
        }
        Next::Disconnected
    }

    /// Waits for `Reconnect` / `Close`; a returning cable may reconnect automatically.
    async fn disconnected(&mut self) -> Next {
        loop {
            match self.cmds.recv().await {
                None => return Next::Exit,
                Some(Command::Close) => {
                    self.ctx.set_state(ConnState::Disconnected, None);
                    return Next::Exit;
                }
                Some(Command::Reconnect) => return Next::Reopen(Reopen::Manual),
                Some(Command::PortAttached) => {
                    self.ctx.health(HealthKind::CableReturned);
                    match self.ctx.call(|b| b.auto_reconnect_enabled()).await {
                        Ok(true) => return Next::Reopen(Reopen::Cable),
                        Ok(false) => {}
                        Err(e) => tracing::warn!("auto reconnect setting: {e}"),
                    }
                }
                Some(Command::Resize(cols, rows)) => {
                    if let Ok(mut s) = self.ctx.size.lock() {
                        *s = (cols, rows);
                    }
                }
                Some(Command::Capture(req)) => {
                    let _ = req.reply.send(Err(AppError::with(
                        ErrorCode::ConnectionClosed,
                        "not connected",
                    )));
                }
                Some(
                    Command::Write(_) | Command::Break | Command::PortDetached | Command::Answer(_),
                ) => {}
            }
        }
    }
}

fn observation_event(o: crate::device::observer::Observation) -> SessionEvent {
    use crate::device::observer::Observation;
    match o {
        Observation::Mode { mode, sub } => SessionEvent::Mode { mode, sub },
        Observation::Identity { hostname, vendor } => SessionEvent::Identity {
            hostname,
            vendor,
            facts: None,
            device_id: None,
        },
        Observation::Reload => SessionEvent::Reload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(accepted: Option<u32>, suggestion: Option<u32>, no_data: bool) -> Outcome {
        Outcome {
            accepted,
            suggestion,
            trials: vec![],
            no_data,
        }
    }

    #[test]
    fn plan_prefers_accepted_then_suggestion_then_remembered() {
        assert_eq!(
            plan_from_outcome(&outcome(Some(9600), Some(19200), false), Some(115200)),
            BaudPlan {
                baud: 9600,
                auto_detected: true,
                remember: true
            }
        );
        assert_eq!(
            plan_from_outcome(&outcome(None, Some(19200), false), Some(115200)),
            BaudPlan {
                baud: 19200,
                auto_detected: true,
                remember: false
            }
        );
        assert_eq!(
            plan_from_outcome(&outcome(None, None, true), Some(115200)),
            BaudPlan {
                baud: 115200,
                auto_detected: false,
                remember: false
            }
        );
        assert_eq!(
            plan_from_outcome(&outcome(None, None, true), None),
            BaudPlan {
                baud: FALLBACK_BAUD,
                auto_detected: false,
                remember: false
            }
        );
    }

    #[test]
    fn state_encoding_round_trips() {
        for s in [
            ConnState::Connecting,
            ConnState::Detecting,
            ConnState::Connected,
            ConnState::Disconnected,
        ] {
            assert_eq!(decode(encode(s)), s);
        }
    }
}
