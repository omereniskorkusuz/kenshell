//! Every external effect of a session (port memory, autobaud, transports, DB) goes
//! through [`Backend`], so the session state machine is testable with fakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::console_detect::ConsoleRegistry;
use crate::console_detect::autobaud::{self, Outcome, Probe, SerialProbe};
use crate::error::{AppError, Result};
use crate::model::{BaudTrial, KnownHost, OpenTarget, Protocol, SerialSettings};
use crate::storage::Db;
use crate::transport::net::TcpTransport;
use crate::transport::raw::RawCodec;
use crate::transport::rlogin::{RloginCodec, local_user_name};
use crate::transport::serial::SerialTransport;
use crate::transport::ssh::SshTransport;
use crate::transport::telnet::TelnetCodec;
use crate::transport::{HostKeyStore, Prompter, Transport, TransportSink};

/// Setting key; JSON bool, default `true`.
pub const AUTO_RECONNECT_SETTING: &str = "console.autoReconnect";
/// Setting key; JSON bool, default `false`.
pub const NET_AUTO_RECONNECT_SETTING: &str = "connection.autoReconnect";
/// Background identity command after login (P3-T04), default on.
pub const IDENTIFY_SETTING: &str = "device.identify";
pub const AUTO_MORE_SETTING: &str = "terminal.autoMore";

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
    /// ssh / telnet / rlogin / raw transport for a `OpenTarget::Network`.
    fn make_network_transport(
        &self,
        target: &OpenTarget,
        size: (u16, u16),
        sink: TransportSink,
        prompter: Arc<dyn Prompter>,
        creds: Option<crate::secrets::Credentials>,
    ) -> Result<Box<dyn Transport>>;
    fn touch_session(&self, id: i64) -> Result<()>;
    fn auto_reconnect_enabled(&self) -> bool;
    /// `connection.autoReconnect` (default false): reconnect dropped network sessions.
    fn net_auto_reconnect_enabled(&self) -> bool;
    /// Session logging config when `log.enabled` is on (default off).
    fn log_config(&self) -> Option<crate::logging::LogConfig>;
    /// Saved login of a saved session: its credential profile, else its own user name
    /// and keychain password. Blocking (keychain).
    fn credentials(&self, session_id: i64) -> Option<crate::secrets::Credentials>;
    /// Run the background identity command: setting `device.identify` (default on) and,
    /// for saved sessions, the session's own switch.
    fn identify_enabled(&self, session_id: Option<i64>) -> bool;
    /// `terminal.autoMore` (default off): answer the device's pager (`--More--`) with a
    /// space so long output runs through (P3-T14).
    fn auto_more_enabled(&self) -> bool {
        false
    }
    /// Stores the device the identity command described; its id (errors are logged).
    fn record_device(
        &self,
        session_id: Option<i64>,
        vendor: &str,
        facts: &crate::device::modes::DeviceFacts,
    ) -> Option<i64>;
}

/// SQLite-backed SSH host key store.
pub(crate) struct DbHostKeys(pub Arc<Db>);

impl HostKeyStore for DbHostKeys {
    fn get(&self, host: &str, port: u16) -> Result<Option<KnownHost>> {
        self.0.get_known_host(host, port)
    }
    fn save(&self, key: &KnownHost) -> Result<()> {
        self.0.save_known_host(key)
    }
}

/// Builds the transport for a network target (shared by the real backend and tests
/// that talk to the local device simulator).
pub(crate) fn network_transport(
    target: &OpenTarget,
    size: (u16, u16),
    sink: TransportSink,
    prompter: Arc<dyn Prompter>,
    keys: Arc<dyn HostKeyStore>,
    creds: Option<crate::secrets::Credentials>,
) -> Result<Box<dyn Transport>> {
    let OpenTarget::Network {
        protocol,
        host,
        port,
        username,
        ssh,
    } = target
    else {
        return Err(AppError::invalid("not a network target"));
    };
    let port = port
        .or(protocol.default_port())
        .ok_or_else(|| AppError::invalid("port required"))?;
    let (cols, rows) = size;
    let host = host.trim().to_owned();
    Ok(match protocol {
        Protocol::Ssh => Box::new(SshTransport::new(
            host,
            port,
            username.clone(),
            ssh.clone(),
            sink,
            prompter,
            keys,
            cols,
            rows,
            creds,
        )),
        Protocol::Telnet => Box::new(TcpTransport::new(
            *protocol,
            host,
            port,
            username.clone(),
            TelnetCodec::new(),
            sink,
            cols,
            rows,
        )),
        Protocol::Raw => Box::new(TcpTransport::new(
            *protocol, host, port, None, RawCodec, sink, cols, rows,
        )),
        Protocol::Rlogin => {
            let remote = username
                .clone()
                .filter(|u| !u.trim().is_empty())
                .unwrap_or_else(|| local_user_name("root"));
            let local = local_user_name(&remote);
            Box::new(TcpTransport::new(
                *protocol,
                host,
                port,
                Some(remote.clone()),
                RloginCodec::new(local, remote),
                sink,
                cols,
                rows,
            ))
        }
        Protocol::Serial => return Err(AppError::invalid("serial is not a network protocol")),
    })
}

/// Production backend: console registry + autobaud + serial transport + SQLite.
pub(crate) struct RealBackend {
    pub db: Arc<Db>,
    pub consoles: Arc<ConsoleRegistry>,
    pub secrets: Arc<dyn crate::secrets::SecretStore>,
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
            .map(|c| crate::console_detect::usb_ids::label(&c))
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

    fn make_network_transport(
        &self,
        target: &OpenTarget,
        size: (u16, u16),
        sink: TransportSink,
        prompter: Arc<dyn Prompter>,
        creds: Option<crate::secrets::Credentials>,
    ) -> Result<Box<dyn Transport>> {
        let keys = Arc::new(DbHostKeys(Arc::clone(&self.db)));
        network_transport(target, size, sink, prompter, keys, creds)
    }

    fn touch_session(&self, id: i64) -> Result<()> {
        self.db.touch_session(id)
    }

    fn credentials(&self, session_id: i64) -> Option<crate::secrets::Credentials> {
        use crate::secrets::{Credentials, profile_account, session_account};
        let rec = self.db.get_session(session_id).ok()?;
        let secret = |acc: String| match self.secrets.get(&acc) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("keychain read for session {session_id}: {e}");
                None
            }
        };
        let creds = match rec.credential_profile_id {
            Some(pid) => {
                let p = self.db.get_profile(pid).ok()?;
                Credentials {
                    username: p.username.or(rec.username),
                    password: secret(profile_account(pid)),
                }
            }
            None => Credentials {
                username: rec.username,
                password: secret(session_account(session_id)),
            },
        };
        (creds.username.is_some() || creds.password.is_some()).then_some(creds)
    }

    fn log_config(&self) -> Option<crate::logging::LogConfig> {
        use crate::logging::{DEFAULT_TEMPLATE, LogConfig, LogMode, default_dir};
        let get = |k: &str| self.db.get_setting(k).ok().flatten();
        if !get("log.enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let text = |k: &str| {
            get(k)
                .and_then(|v| v.as_str().map(str::to_owned))
                .filter(|s| !s.trim().is_empty())
        };
        Some(LogConfig {
            dir: text("log.dir").map(Into::into).unwrap_or_else(default_dir),
            template: text("log.template").unwrap_or_else(|| DEFAULT_TEMPLATE.into()),
            mode: get("log.mode")
                .and_then(|v| serde_json::from_value::<LogMode>(v).ok())
                .unwrap_or_default(),
            timestamps: get("log.timestamps")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    fn net_auto_reconnect_enabled(&self) -> bool {
        match self.db.get_setting(NET_AUTO_RECONNECT_SETTING) {
            Ok(Some(v)) => v.as_bool().unwrap_or(false),
            Ok(None) => false,
            Err(e) => {
                tracing::warn!("read {NET_AUTO_RECONNECT_SETTING}: {e}");
                false
            }
        }
    }

    fn auto_more_enabled(&self) -> bool {
        match self.db.get_setting(AUTO_MORE_SETTING) {
            Ok(Some(v)) => v.as_bool().unwrap_or(false),
            Ok(None) => false,
            Err(e) => {
                tracing::warn!("read {AUTO_MORE_SETTING}: {e}");
                false
            }
        }
    }

    fn identify_enabled(&self, session_id: Option<i64>) -> bool {
        let global = match self.db.get_setting(IDENTIFY_SETTING) {
            Ok(Some(v)) => v.as_bool().unwrap_or(true),
            Ok(None) => true,
            Err(e) => {
                tracing::warn!("read {IDENTIFY_SETTING}: {e}");
                true
            }
        };
        global && session_id.is_none_or(|sid| self.db.get_session(sid).map_or(true, |s| s.identify))
    }

    fn record_device(
        &self,
        session_id: Option<i64>,
        vendor: &str,
        facts: &crate::device::modes::DeviceFacts,
    ) -> Option<i64> {
        match self.db.record_device(session_id, vendor, facts) {
            Ok(d) => Some(d.id),
            Err(e) => {
                tracing::warn!("store device: {e}");
                None
            }
        }
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
