//! Serial transport (serialport crate) — P1-T08.
//! Blocking reader thread with a long read timeout (no busy loop), batching reads,
//! writes via a cloned handle, BREAK for ROMMON / password recovery, DTR/RTS control.
#![allow(dead_code, unused_variables)] // TODO(P1-T08): remove once implemented

use async_trait::async_trait;

use super::{Transport, TransportSink};
use crate::error::Result;
use crate::model::SerialSettings;

pub struct SerialTransport {
    settings: SerialSettings,
    baud: u32,
    sink: TransportSink,
}

impl SerialTransport {
    /// `baud` is the resolved rate (autobaud already done by the session layer).
    pub fn new(settings: SerialSettings, baud: u32, sink: TransportSink) -> Self {
        Self {
            settings,
            baud,
            sink,
        }
    }
}

#[async_trait]
impl Transport for SerialTransport {
    async fn connect(&mut self) -> Result<()> {
        todo!("P1-T08")
    }
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        todo!("P1-T08")
    }
    async fn resize(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Ok(()) // serial lines have no window size
    }
    async fn send_break(&mut self) -> Result<()> {
        todo!("P1-T08")
    }
    async fn disconnect(&mut self) -> Result<()> {
        todo!("P1-T08")
    }
}
