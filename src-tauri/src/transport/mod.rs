//! Transports: byte pipes to a device. Owner: P1-T06 (trait), P1-T08 (serial).
//! Phase 2 adds ssh, telnet, rlogin, raw.
//!
//! A transport pushes everything it receives into its [`TransportSink`] (given at
//! construction) from its own reader thread/task, so the session loop never polls.

pub mod serial;

use async_trait::async_trait;

use crate::error::{AppError, Result};

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
}
