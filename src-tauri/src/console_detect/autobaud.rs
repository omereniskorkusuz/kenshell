//! Automatic baud detection — P1-T12. `score` is a PURE function (unit-tested with
//! sample byte streams); `detect` runs the candidate loop over a [`Probe`].
//!
//! Algorithm (spec 7.2): candidates = remembered → 9600, 115200, 38400, 57600, 19200,
//! 4800, 2400. Per candidate: set baud, flush, send `\r`, read ~600 ms; if nothing, try
//! `\r\n` once. Score = printable ASCII + CR/LF/TAB ratio, + bonus if a known prompt
//! pattern matches. Accept when score ≥ 0.85 AND prompt matched; otherwise suggest best.
#![allow(dead_code, unused_variables)] // TODO(P1-T12): remove once implemented

use crate::error::Result;
use crate::model::BaudTrial;

pub const DEFAULT_CANDIDATES: [u32; 7] = [9600, 115200, 38400, 57600, 19200, 4800, 2400];
pub const ACCEPT_SCORE: f32 = 0.85;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub value: f32,
    pub printable_ratio: f32,
    pub prompt_match: bool,
    pub bytes: usize,
}

/// Remembered first (deduplicated), then the defaults.
pub fn candidate_order(remembered: Option<u32>) -> Vec<u32> {
    todo!("P1-T12")
}

pub fn score(bytes: &[u8]) -> Score {
    todo!("P1-T12")
}

pub fn is_accepted(s: &Score) -> bool {
    s.value >= ACCEPT_SCORE && s.prompt_match
}

/// One probe at one baud rate; returns what the device answered.
pub trait Probe {
    fn probe(&mut self, baud: u32) -> Result<Vec<u8>>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Accepted rate (score ≥ threshold + prompt).
    pub accepted: Option<u32>,
    /// Highest scoring rate with data, when nothing was accepted.
    pub suggestion: Option<u32>,
    pub trials: Vec<BaudTrial>,
    /// No candidate produced a single byte.
    pub no_data: bool,
}

/// Stops at the first accepted candidate. `on_trial` is called after each probe.
pub fn detect(
    probe: &mut dyn Probe,
    remembered: Option<u32>,
    on_trial: &mut dyn FnMut(&BaudTrial),
) -> Result<Outcome> {
    todo!("P1-T12")
}

/// Real serial-port probe: opens the port once and changes the baud per candidate.
pub struct SerialProbe {
    // TODO(P1-T12)
}

impl SerialProbe {
    pub fn open(settings: &crate::model::SerialSettings) -> Result<Self> {
        todo!("P1-T12")
    }
}

impl Probe for SerialProbe {
    fn probe(&mut self, baud: u32) -> Result<Vec<u8>> {
        todo!("P1-T12")
    }
}
