//! Transports: byte pipes to a device. Owner: P1-T06 (trait), P1-T08 (serial).
//! Phase 2 adds ssh, telnet, rlogin, raw.
//!
//! A transport pushes everything it receives into its [`TransportSink`] (given at
//! construction) from its own reader thread/task, so the session loop never polls.

pub mod net;
pub mod raw;
pub mod rlogin;
pub mod serial;
pub mod ssh;
pub mod telnet;
pub mod tunnel;

use async_trait::async_trait;

use crate::error::{AppError, Result};
use crate::model::{KnownHost, NetLink, PromptRequest};

#[derive(Debug)]
pub enum TransportEvent {
    /// Bytes received from the device (already batched by the reader).
    Data(Vec<u8>),
    /// Reader ended. `None` = clean close; `Some(err)` = failure (e.g. `DeviceGone`).
    Closed(Option<AppError>),
}

/// Bounded channel: backpressure if the UI cannot keep up.
pub type TransportSink = tokio::sync::mpsc::Sender<TransportEvent>;

#[async_trait]
pub trait Transport: Send {
    async fn connect(&mut self) -> Result<()>;
    async fn write(&mut self, data: &[u8]) -> Result<()>;
    async fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    async fn send_break(&mut self) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;

    /// Network connection facts once connected (identity bar). Serial: `None`.
    fn net_link(&self) -> Option<NetLink> {
        None
    }

    /// The authenticated SSH connection, for extra channels (SFTP / SCP, P5-T01).
    fn ssh_handle(&self) -> Option<std::sync::Arc<russh::client::Handle<ssh::Client>>> {
        None
    }
}

/// Asks the user something during login (username, password, keyboard-interactive,
/// key passphrase). Answered inside the terminal, PuTTY-style. `None` = cancelled
/// (tab closed). Implementations must never log the answers.
#[async_trait]
pub trait Prompter: Send + Sync {
    async fn ask(&self, request: PromptRequest) -> Option<Vec<String>>;
}

/// Stored SSH host keys (implemented by the SQLite store).
pub trait HostKeyStore: Send + Sync {
    fn get(&self, host: &str, port: u16) -> Result<Option<KnownHost>>;
    fn save(&self, key: &KnownHost) -> Result<()>;
}
