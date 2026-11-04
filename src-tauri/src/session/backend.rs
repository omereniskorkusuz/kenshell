//! Every external effect of a session (port memory, autobaud, transports, DB) goes
//! through [`Backend`], so the session state machine is testable with fakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::console_detect::ConsoleRegistry;
use crate::console_detect::autobaud::{self, Outcome, Probe, SerialProbe};
use crate::error::{AppError, Result};
use crate::model::{BaudTrial, SerialSettings};
use crate::storage::Db;
use crate::transport::serial::SerialTransport;
use crate::transport::{Transport, TransportSink};

/// Setting key; JSON bool, default `true`.
pub const AUTO_RECONNECT_SETTING: &str = "console.autoReconnect";

/// Synchronous by design: implementations may block briefly (DB, port enumeration);
/// the session task calls them from `spawn_blocking`.
pub(crate) trait Backend: Send + Sync + 'static {
    fn remembered_baud(&self, path: &str) -> Option<u32>;
    fn remember(&self, path: &str, baud: u32) -> Result<()>;
    fn display_name(&self, path: &str) -> String;
    /// e.g. `FTDI FT232R`
    fn chip_label(&self, path: &str) -> Option<String>;
    /// Stable cable identity used to match hotplug changes.
    fn port_key(&self, path: &str) -> String;
    /// Runs the autobaud loop (blocking). Must return soon after `cancel` is set and
    /// must have closed its probe port when it returns.
    fn run_autobaud(
        &self,
        settings: &SerialSettings,
        remembered: Option<u32>,
        cancel: &AtomicBool,
        on_trial: &mut dyn FnMut(&BaudTrial),
    ) -> Result<Outcome>;
    fn make_transport(
        &self,
        settings: &SerialSettings,
        baud: u32,
        sink: TransportSink,
    ) -> Box<dyn Transport>;
    fn touch_session(&self, id: i64) -> Result<()>;
    fn auto_reconnect_enabled(&self) -> bool;
}

/// Production backend: console registry + autobaud + serial transport + SQLite.
pub(crate) struct RealBackend {
    pub db: Arc<Db>,
    pub consoles: Arc<ConsoleRegistry>,
}

impl Backend for RealBackend {
    fn remembered_baud(&self, path: &str) -> Option<u32> {
        self.consoles.remembered_baud(path)
    }

    fn remember(&self, path: &str, baud: u32) -> Result<()> {
        self.consoles.remember(path, baud, None)
    }

    fn display_name(&self, path: &str) -> String {
        self.consoles.display_name(path)
    }

    fn chip_label(&self, path: &str) -> Option<String> {
        self.consoles
            .chip_for(path)
            .map(|c| format!("{} {}", c.vendor, c.model))
    }

    fn port_key(&self, path: &str) -> String {
        self.consoles.port_key(path)
    }

    fn run_autobaud(
        &self,
        settings: &SerialSettings,
        remembered: Option<u32>,
        cancel: &AtomicBool,
        on_trial: &mut dyn FnMut(&BaudTrial),
    ) -> Result<Outcome> {
        let mut probe = CancellableProbe {
            inner: SerialProbe::open(settings)?,
            cancel,
        };
        autobaud::detect(&mut probe, remembered, on_trial)
        // `probe` (and its port) is dropped here, before the transport opens.
    }

    fn make_transport(
        &self,
        settings: &SerialSettings,
        baud: u32,
        sink: TransportSink,
    ) -> Box<dyn Transport> {
        Box::new(SerialTransport::new(settings.clone(), baud, sink))
    }

    fn touch_session(&self, id: i64) -> Result<()> {
        self.db.touch_session(id)
    }

    fn auto_reconnect_enabled(&self) -> bool {
        match self.db.get_setting(AUTO_RECONNECT_SETTING) {
            Ok(Some(v)) => v.as_bool().unwrap_or(true),
            Ok(None) => true,
            Err(e) => {
                tracing::warn!("read {AUTO_RECONNECT_SETTING}: {e}");
                true
            }
        }
    }
}

/// Aborts the candidate loop at the next probe once the session was closed.
struct CancellableProbe<'a, P: Probe> {
    inner: P,
    cancel: &'a AtomicBool,
}

impl<P: Probe> Probe for CancellableProbe<'_, P> {
    fn probe(&mut self, baud: u32) -> Result<Vec<u8>> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(AppError::invalid("autobaud cancelled"));
        }
        self.inner.probe(baud)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(Vec<u32>);
    impl Probe for Echo {
        fn probe(&mut self, baud: u32) -> Result<Vec<u8>> {
            self.0.push(baud);
            Ok(b"x".to_vec())
        }
    }

    #[test]
    fn cancellable_probe_stops_after_cancel() {
        let cancel = AtomicBool::new(false);
        let mut p = CancellableProbe {
            inner: Echo(vec![]),
            cancel: &cancel,
        };
        assert!(p.probe(9600).is_ok());
        cancel.store(true, Ordering::Release);
        assert!(p.probe(115200).is_err());
        assert_eq!(p.inner.0, vec![9600]);
    }
}
