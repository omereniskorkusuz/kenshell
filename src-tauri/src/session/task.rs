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
}

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
}

impl Shared {
    pub fn new(path: String) -> Self {
        Self {
            state: AtomicU8::new(encode(ConnState::Connecting)),
            path,
            port_key: Mutex::new(None),
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
        let mut transport =
            self.backend
                .make_network_transport(&self.target, size, tx, prompter)?;
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
        }
    }

    pub async fn run(mut self) {
        let mut next = Next::Reopen(Reopen::Manual);
        loop {
            next = match next {
                Next::Reopen(why) => match self.connect(why).await {
                    Some(Phase::Connected(link)) => self.connected(link).await,
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

    async fn connected(&mut self, link: Link) -> Next {
        let (mut transport, mut rx) = link;
        let mut carry: Option<TransportEvent> = None;
        let mut detach_deadline: Option<Instant> = None;
        loop {
            enum Ev {
                Transport(Option<TransportEvent>),
                Cmd(Option<Command>),
                DetachTimeout,
            }
            let ev = if let Some(ev) = carry.take() {
                Ev::Transport(Some(ev))
            } else {
                let deadline = detach_deadline;
                tokio::select! {
                    ev = rx.recv() => Ev::Transport(ev),
                    cmd = self.cmds.recv() => Ev::Cmd(cmd),
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
                    self.ctx.out.data(buf);
                }
                Ev::Transport(Some(TransportEvent::Closed(err))) => {
                    return self.link_lost(transport, err).await;
                }
                Ev::Transport(None) => {
                    let err = AppError::with(ErrorCode::Io, "transport ended unexpectedly");
                    return self.link_lost(transport, Some(err)).await;
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
                    Command::Write(data) => {
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
                Some(
                    Command::Write(_) | Command::Break | Command::PortDetached | Command::Answer(_),
                ) => {}
            }
        }
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
