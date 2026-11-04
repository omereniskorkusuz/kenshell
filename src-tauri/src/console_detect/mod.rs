//! Smart console detection ("plug and connect") — P1-T09..T13.
//!
//! * `ports`       — enumerate serial ports, merge macOS tty/cu twins, stable keys
//! * `usb_ids`     — VID:PID → chip table (FTDI, Prolific, CP210x, CH340, Cisco …)
//! * `hotplug`     — event-driven USB watch (nusb: IOKit / Windows CM notifications);
//!   ≥ 2 s change-comparing polling only as fallback
//! * `driver_hint` — known chip on USB but no serial port → driver missing
//! * `autobaud`    — pure scoring + candidate loop
//!
//! `ConsoleRegistry` keeps the last snapshot and computes diffs.
#![allow(dead_code, unused_variables)] // TODO(P1-T09): remove once implemented

pub mod autobaud;
pub mod driver_hint;
pub mod hotplug;
pub mod ports;
pub mod usb_ids;

use std::sync::Arc;

use crate::error::Result;
use crate::model::{ChipInfo, ConsoleChange, ConsoleSnapshot};
use crate::storage::Db;

pub struct ConsoleRegistry {
    db: Arc<Db>,
}

impl ConsoleRegistry {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// Current snapshot (scans on first call).
    pub fn snapshot(&self) -> ConsoleSnapshot {
        todo!("P1-T09")
    }

    /// Re-enumerates; returns a change if anything differs from the last snapshot.
    pub fn rescan(&self) -> Option<ConsoleChange> {
        todo!("P1-T09")
    }

    /// Starts the background watcher; `on_change` is called for every change.
    pub fn start_watch(self: &Arc<Self>, on_change: Box<dyn Fn(ConsoleChange) + Send + Sync>) {
        todo!("P1-T10")
    }

    /// Stable key for a port path (USB serial number if known, else the path).
    pub fn port_key(&self, path: &str) -> String {
        todo!("P1-T13")
    }

    /// Chip of the port at `path`, if it is a known USB console.
    pub fn chip_for(&self, path: &str) -> Option<ChipInfo> {
        todo!("P1-T09")
    }

    /// Short label (`COM4`, `usbserial-A10K3XYZ`).
    pub fn display_name(&self, path: &str) -> String {
        todo!("P1-T09")
    }

    pub fn remembered_baud(&self, path: &str) -> Option<u32> {
        todo!("P1-T13")
    }

    /// Persists the working baud (and optional device name) for this cable.
    pub fn remember(&self, path: &str, baud: u32, last_device: Option<&str>) -> Result<()> {
        todo!("P1-T13")
    }
}
