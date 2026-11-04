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
//! emit `Link` → `Connected` → remember baud via `ConsoleRegistry::remember`.
#![allow(dead_code, unused_variables)] // TODO(P1-T06): remove once implemented

use std::sync::Arc;

use crate::console_detect::ConsoleRegistry;
use crate::error::Result;
use crate::model::{ConsoleChange, OpenRequest, RuntimeId, SessionEvent};
use crate::storage::Db;

/// Where a session's output goes (Tauri channels in the app, a recorder in tests).
pub trait SessionOutput: Send + Sync + 'static {
    /// Raw device bytes, batched. Must not block for long.
    fn data(&self, bytes: Vec<u8>);
    fn event(&self, event: SessionEvent);
}

pub struct SessionManager {
    db: Arc<Db>,
    consoles: Arc<ConsoleRegistry>,
}

impl SessionManager {
    pub fn new(db: Arc<Db>, consoles: Arc<ConsoleRegistry>) -> Self {
        Self { db, consoles }
    }

    /// Starts connecting in the background and returns immediately.
    pub fn open(&self, req: OpenRequest, out: Arc<dyn SessionOutput>) -> Result<RuntimeId> {
        todo!("P1-T06")
    }

    pub fn write(&self, id: RuntimeId, data: &[u8]) -> Result<()> {
        todo!("P1-T06")
    }

    pub fn resize(&self, id: RuntimeId, cols: u16, rows: u16) -> Result<()> {
        todo!("P1-T06")
    }

    pub fn send_break(&self, id: RuntimeId) -> Result<()> {
        todo!("P1-T08")
    }

    /// Closes the transport and forgets the session.
    pub fn close(&self, id: RuntimeId) -> Result<()> {
        todo!("P1-T06")
    }

    /// Re-runs the open flow for a disconnected session (same output).
    pub fn reconnect(&self, id: RuntimeId) -> Result<()> {
        todo!("P1-T06")
    }

    /// Called on every console hotplug change: sessions whose port vanished get
    /// `Health{CableRemoved}` (+ Disconnected); when it returns → `CableReturned`
    /// and auto-reconnect if enabled.
    pub fn on_console_change(&self, change: &ConsoleChange) {
        todo!("P1-T15")
    }
}
