//! Live session lifecycle — P1-T06 (+ autobaud wiring P1-T12, health P1-T15).
//!
//! `SessionManager` owns one tokio task per open session. The task drives a
//! [`crate::transport::Transport`], forwards received bytes to the UI through
//! [`SessionOutput`] (batched), and reports state via [`SessionEvent`]s.
//! Tabs never close on disconnect: the session stays in `Disconnected` and can be
//! reconnected (`reconnect`) — automatically when the same console cable returns
//! and setting `console.autoReconnect` (default true) is on.
//!
//! Serial open flow: resolve baud (manual > autobaud with remembered first) →
//! connect → remember baud via `ConsoleRegistry::remember` → emit `Link` → `Connected`.
//!
//! All external effects go through [`backend::Backend`] so the state machine is
//! unit-tested with fakes (`tests.rs`).

mod backend;
mod router;
mod task;
#[cfg(test)]
mod tests;
// macOS: serialport sets the speed with IOSSIOSPEED, which pseudo-terminals reject
// (ENOTTY), so the pty-based end-to-end test runs on Linux only (CI job `e2e-linux`).
#[cfg(all(test, target_os = "linux"))]
mod e2e;
#[cfg(test)]
mod net_e2e;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedSender};

use self::backend::{Backend, RealBackend};
use self::router::Router;
use self::task::{Command, SessionTask, Shared};
use crate::console_detect::ConsoleRegistry;
use crate::error::{AppError, Result};
use crate::model::{ConnState, ConsoleChange, OpenRequest, OpenTarget, RuntimeId, SessionEvent};
use crate::storage::Db;

/// Where a session's output goes (Tauri channels in the app, a recorder in tests).
pub trait SessionOutput: Send + Sync + 'static {
    /// Raw device bytes, batched. Must not block for long.
    fn data(&self, bytes: Vec<u8>);
    fn event(&self, event: SessionEvent);
}

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
type Spawner = Arc<dyn Fn(BoxFuture) + Send + Sync>;

struct Handle {
    cmds: UnboundedSender<Command>,
    shared: Arc<Shared>,
    /// Output switch used to move the session to another window.
    router: Arc<Router>,
    /// Label of the window that shows this session (closed with that window).
    owner: String,
}

type Registry = Mutex<HashMap<RuntimeId, Handle>>;

/// Health timings (shortened in tests).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// After hotplug reports the cable gone, how long the reader gets to report the
    /// failure itself before the session is marked disconnected anyway.
    pub detach_grace: Duration,
    /// Delay before an automatic reconnect attempt (the OS may still be creating the
    /// port when the hotplug event arrives).
    pub reconnect_settle: Duration,
    /// Waits before each automatic network reconnect attempt (backoff).
    pub net_retry: [Duration; 4],
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            detach_grace: Duration::from_millis(1500),
            reconnect_settle: Duration::from_millis(500),
            net_retry: [
                Duration::from_secs(2),
                Duration::from_secs(5),
                Duration::from_secs(10),
                Duration::from_secs(30),
            ],
        }
    }
}

pub struct SessionManager {
    backend: Arc<dyn Backend>,
    spawn: Spawner,
    timing: Timing,
    next_id: AtomicU32,
    sessions: Arc<Registry>,
}

impl SessionManager {
    pub fn new(
        db: Arc<Db>,
        consoles: Arc<ConsoleRegistry>,
        secrets: Arc<dyn crate::secrets::SecretStore>,
    ) -> Self {
        Self::with_backend(
            Arc::new(RealBackend {
                db,
                consoles,
                secrets,
            }),
            Arc::new(|f| {
                tauri::async_runtime::spawn(f);
            }),
            Timing::default(),
        )
    }

    fn with_backend(backend: Arc<dyn Backend>, spawn: Spawner, timing: Timing) -> Self {
        Self {
            backend,
            spawn,
            timing,
            next_id: AtomicU32::new(1),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Test constructor: fake backend, tasks on the current tokio runtime.
    #[cfg(test)]
    pub(crate) fn for_tests(backend: Arc<dyn Backend>, timing: Timing) -> Self {
        Self::with_backend(
            backend,
            Arc::new(|f| {
                tokio::spawn(f);
            }),
            timing,
        )
    }

    fn registry(&self) -> MutexGuard<'_, HashMap<RuntimeId, Handle>> {
        // A panic while holding this lock cannot leave the map inconsistent
        // (single insert/remove calls), so recover from poisoning.
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Starts connecting in the background and returns immediately.
    pub fn open(&self, req: OpenRequest, out: Arc<dyn SessionOutput>) -> Result<RuntimeId> {
        let path = match &req.target {
            OpenTarget::Serial { settings } => {
                if settings.port.trim().is_empty() {
                    return Err(AppError::invalid("no serial port selected"));
                }
                if settings.baud == Some(0) {
                    return Err(AppError::invalid("baud rate must be > 0"));
                }
                settings.port.clone()
            }
            OpenTarget::Network {
                protocol,
                host,
                port,
                ..
            } => {
                let host = host.trim();
                if host.is_empty() || host.chars().any(char::is_whitespace) {
                    return Err(AppError::invalid("host required"));
                }
                if *protocol == crate::model::Protocol::Serial {
                    return Err(AppError::invalid("serial needs serial settings"));
                }
                let port = port
                    .or(protocol.default_port())
                    .filter(|p| *p > 0)
                    .ok_or_else(|| AppError::invalid("port required"))?;
                crate::transport::net::label(host, port)
            }
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new(path.clone()));
        let router = Arc::new(Router::new(out));
        self.registry().insert(
            id,
            Handle {
                cmds: tx,
                shared: Arc::clone(&shared),
                router: Arc::clone(&router),
                owner: String::new(),
            },
        );
        tracing::debug!(session = id, target = %path, "session open");
        let task = SessionTask::new(
            id,
            req.session_id,
            req.target,
            router,
            Arc::clone(&self.backend),
            shared,
            self.timing,
            (req.cols.max(1), req.rows.max(1)),
            rx,
            Arc::downgrade(&self.sessions),
        );
        (self.spawn)(Box::pin(task.run()));
        Ok(id)
    }

    /// Sends a command to a live session; `NotFound` if the id is unknown.
    fn send(&self, id: RuntimeId, cmd: Command) -> Result<()> {
        let map = self.registry();
        let h = map.get(&id).ok_or_else(|| not_found(id))?;
        h.cmds.send(cmd).map_err(|_| not_found(id))
    }

    /// Ensures the session is connected; otherwise `Invalid` with detail
    /// `disconnected` (UI offers reconnect) or `connecting`.
    fn require_connected(&self, id: RuntimeId) -> Result<()> {
        let map = self.registry();
        let h = map.get(&id).ok_or_else(|| not_found(id))?;
        match h.shared.state() {
            ConnState::Connected => Ok(()),
            ConnState::Disconnected => Err(AppError::invalid("disconnected")),
            ConnState::Connecting | ConnState::Detecting => Err(AppError::invalid("connecting")),
        }
    }

    pub fn write(&self, id: RuntimeId, data: &[u8]) -> Result<()> {
        self.require_connected(id)?;
        if data.is_empty() {
            return Ok(());
        }
        self.send(id, Command::Write(data.to_vec()))
    }

    /// Runs a read-only command hidden from the terminal and the log, at the device's next
    /// quiet prompt, and returns its output (P4-T06).
    pub async fn capture(
        &self,
        id: RuntimeId,
        command: &str,
        timeout: std::time::Duration,
    ) -> Result<String> {
        self.require_connected(id)?;
        if !crate::device::capture::read_only(command) {
            return Err(AppError::invalid("only read-only show/display commands"));
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(
            id,
            Command::Capture(task::CaptureRequest {
                command: command.trim().to_owned(),
                timeout,
                reply: tx,
            }),
        )?;
        rx.await.map_err(|_| {
            AppError::with(crate::error::ErrorCode::ConnectionClosed, "session closed")
        })?
    }

    /// The authenticated SSH connection of a connected SSH session (SFTP / SCP channels).
    pub fn ssh_handle(
        &self,
        id: RuntimeId,
    ) -> Result<Arc<russh::client::Handle<crate::transport::ssh::Client>>> {
        let map = self.registry();
        let h = map.get(&id).ok_or_else(|| not_found(id))?;
        h.shared
            .ssh
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .ok_or_else(|| AppError::invalid("not a connected SSH session"))
    }

    /// Starts recording the session's output into `dir` (P5-T06); returns the file.
    pub fn record_start(
        &self,
        id: RuntimeId,
        dir: &std::path::Path,
        name: &str,
        cols: u16,
        rows: u16,
    ) -> Result<String> {
        let map = self.registry();
        let h = map.get(&id).ok_or_else(|| not_found(id))?;
        let rec = crate::logging::record::Recording::open(dir, name, cols, rows)?;
        let path = rec.path().display().to_string();
        let mut slot = h
            .shared
            .recording
            .lock()
            .map_err(|_| AppError::internal("recording state"))?;
        // Replacing an older recording closes (flushes) it.
        *slot = Some(rec);
        Ok(path)
    }

    /// Stops recording; the file is complete when this returns.
    pub fn record_stop(&self, id: RuntimeId) -> Result<Option<String>> {
        // Taken out under the registry lock, dropped (flushed, thread joined) after it.
        let rec = {
            let map = self.registry();
            let h = map.get(&id).ok_or_else(|| not_found(id))?;
            h.shared.recording.lock().ok().and_then(|mut r| r.take())
        };
        Ok(rec.map(|r| r.path().display().to_string()))
    }

    pub fn resize(&self, id: RuntimeId, cols: u16, rows: u16) -> Result<()> {
        if let Some(h) = self.registry().get(&id)
            && let Ok(r) = h.shared.recording.lock()
            && let Some(r) = r.as_ref()
        {
            r.resize(cols, rows);
        }
        self.send(id, Command::Resize(cols, rows))
    }

    pub fn send_break(&self, id: RuntimeId) -> Result<()> {
        self.require_connected(id)?;
        self.send(id, Command::Break)
    }

    /// Closes the transport and forgets the session.
    pub fn close(&self, id: RuntimeId) -> Result<()> {
        let h = self.registry().remove(&id).ok_or_else(|| not_found(id))?;
        // The task may already be gone (manager shutting down); nothing left to do then.
        let _ = h.cmds.send(Command::Close);
        tracing::debug!(session = id, "session close requested");
        Ok(())
    }

    /// Answers the pending login prompt (username / password / keyboard-interactive).
    pub fn answer(&self, id: RuntimeId, answers: Vec<String>) -> Result<()> {
        self.send(id, Command::Answer(answers))
    }

    /// Re-runs the open flow for a disconnected session (same output).
    /// A no-op while the session is connected or connecting.
    pub fn reconnect(&self, id: RuntimeId) -> Result<()> {
        self.send(id, Command::Reconnect)
    }

    /// Records which window shows the session (see [`Self::close_owned_by`]).
    pub fn set_owner(&self, id: RuntimeId, owner: &str) -> Result<()> {
        let mut map = self.registry();
        let h = map.get_mut(&id).ok_or_else(|| not_found(id))?;
        owner.clone_into(&mut h.owner);
        Ok(())
    }

    /// Holds the session's output while its tab moves to another window; the current
    /// window receives end markers first.
    pub fn park(&self, id: RuntimeId) -> Result<()> {
        let router = Arc::clone(
            &self
                .registry()
                .get(&id)
                .ok_or_else(|| not_found(id))?
                .router,
        );
        router.park();
        Ok(())
    }

    /// Sends the session's output (held output first) to `out`, owned by window `owner`.
    pub fn attach(&self, id: RuntimeId, out: Arc<dyn SessionOutput>, owner: &str) -> Result<()> {
        let router = {
            let mut map = self.registry();
            let h = map.get_mut(&id).ok_or_else(|| not_found(id))?;
            owner.clone_into(&mut h.owner);
            Arc::clone(&h.router)
        };
        router.attach(out);
        Ok(())
    }

    /// Closes every session shown by window `owner` (that window is closing). Their output
    /// is dropped right here (see [`Router::disconnect`]); call it on the main thread.
    pub fn close_owned_by(&self, owner: &str) -> usize {
        let owned: Vec<(RuntimeId, Arc<Router>)> = self
            .registry()
            .iter()
            .filter(|(_, h)| h.owner == owner)
            .map(|(id, h)| (*id, Arc::clone(&h.router)))
            .collect();
        for (id, router) in &owned {
            router.disconnect();
            let _ = self.close(*id);
        }
        owned.len()
    }

    /// Called on every console hotplug change: sessions whose port vanished get
    /// `Health{CableRemoved}` (+ Disconnected); when it returns → `CableReturned`
    /// and auto-reconnect if enabled.
    pub fn on_console_change(&self, change: &ConsoleChange) {
        // Paths of the attached ports: a session opened while its cable was absent may
        // have cached a path-based key, while the returning port reports its USB serial.
        let attached_paths: Vec<&str> = change
            .snapshot
            .ports
            .iter()
            .filter(|p| change.attached.contains(&p.key))
            .flat_map(|p| std::iter::once(p.path.as_str()).chain(p.alt_path.as_deref()))
            .collect();
        let map = self.registry();
        for (id, h) in map.iter() {
            let key = h.shared.port_key();
            let has = |keys: &[String]| key.as_ref().is_some_and(|k| keys.contains(k));
            let cmd = match h.shared.state() {
                ConnState::Disconnected
                    if has(&change.attached)
                        || attached_paths.contains(&h.shared.path.as_str()) =>
                {
                    Command::PortAttached
                }
                ConnState::Connected if has(&change.detached) => Command::PortDetached,
                _ => continue,
            };
            tracing::debug!(session = id, "console hotplug affects session");
            let _ = h.cmds.send(cmd);
        }
    }
}

fn not_found(id: RuntimeId) -> AppError {
    AppError::not_found(format!("session {id}"))
}
