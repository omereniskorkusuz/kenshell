//! Background identity command (P3-T04): once per connection, when the device sits quietly
//! at a user/privileged prompt, the session sends the vendor's read-only identity command
//! (`show version`), keeps its output out of the terminal and the log, and parses it.
//!
//! Pure state machine (time is passed in); the session task wires it to its timer, the
//! transport and held user input.

use std::time::{Duration, Instant};

use super::lines::LineAssembler;
use super::modes::{CliMode, Profile};

/// The prompt must be quiet this long before the command is sent.
pub const QUIET: Duration = Duration::from_millis(700);
/// Give up after this long; the captured output is then shown after all.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Captured output cap (a `show version` is a few KB).
const MAX_CAPTURE: usize = 256 * 1024;
/// At most this many `--More--` pages are answered.
const MAX_PAGES: u32 = 50;

#[derive(Debug)]
enum State {
    Off,
    /// Waiting for a quiet prompt.
    Pending,
    Scheduled(Instant),
    Capturing(Box<Capture>),
    Done,
}

#[derive(Debug)]
struct Capture {
    started: Instant,
    raw: Vec<u8>,
    lines: LineAssembler,
    text: String,
    completed: usize,
    pages: u32,
}

/// What the session does with output that arrived during a capture.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Keep capturing (nothing to send).
    Continue,
    /// A `--More--` page: send a space.
    NextPage,
    /// The prompt is back: here is the plain-text output (prompt line excluded).
    Finished(String),
}

#[derive(Debug)]
pub struct Prober {
    state: State,
}

impl Prober {
    pub fn new(enabled: bool) -> Self {
        Self {
            state: if enabled { State::Pending } else { State::Off },
        }
    }

    /// When the session's timer must wake this prober.
    pub fn deadline(&self) -> Option<Instant> {
        match &self.state {
            State::Scheduled(at) => Some(*at),
            State::Capturing(c) => Some(c.started + TIMEOUT),
            _ => None,
        }
    }

    pub fn capturing(&self) -> bool {
        matches!(self.state, State::Capturing(_))
    }

    /// After normal output: (re)schedule when the line is a user/privileged prompt.
    pub fn after_output(&mut self, at_prompt: Option<CliMode>, now: Instant) {
        if matches!(self.state, State::Pending | State::Scheduled(_)) {
            self.state = match at_prompt {
                Some(CliMode::User | CliMode::Privileged) => State::Scheduled(now + QUIET),
                _ => State::Pending,
            };
        }
    }

    /// The user typed: wait for the next quiet prompt.
    pub fn user_input(&mut self) {
        if matches!(self.state, State::Scheduled(_)) {
            self.state = State::Pending;
        }
    }

    /// The scheduled time came: start capturing if still at a prompt. Returns the command
    /// line to send (with CR).
    pub fn fire(
        &mut self,
        at_prompt: Option<CliMode>,
        profile: &Profile,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let State::Scheduled(at) = self.state else {
            return None;
        };
        if now < at {
            return None;
        }
        if !matches!(at_prompt, Some(CliMode::User | CliMode::Privileged)) {
            self.state = State::Pending;
            return None;
        }
        self.state = State::Capturing(Box::new(Capture {
            started: now,
            raw: Vec::new(),
            lines: LineAssembler::new(),
            text: String::new(),
            completed: 0,
            pages: 0,
        }));
        Some(format!("{}\r", profile.identity_command).into_bytes())
    }

    /// Output during a capture (never shown).
    pub fn capture(&mut self, bytes: &[u8], profile: &Profile) -> Step {
        let State::Capturing(c) = &mut self.state else {
            return Step::Continue;
        };
        if c.raw.len() + bytes.len() <= MAX_CAPTURE {
            c.raw.extend_from_slice(bytes);
        }
        let (text, completed) = (&mut c.text, &mut c.completed);
        c.lines.feed(bytes, |line| {
            *completed += 1;
            if text.len() < MAX_CAPTURE {
                text.push_str(&line);
                text.push('\n');
            }
        });
        let current = c.lines.current().trim_end();
        // The first completed line is the echo of the command; the prompt after the output
        // ends the capture.
        if c.completed >= 1 && profile.match_prompt(current).is_some() {
            let text = std::mem::take(&mut c.text);
            self.state = State::Done;
            return Step::Finished(text);
        }
        if current.contains(profile.more.as_str()) && c.pages < MAX_PAGES {
            c.pages += 1;
            return Step::NextPage;
        }
        Step::Continue
    }

    /// The capture took too long: give back what arrived so the terminal can show it.
    pub fn time_out(&mut self, now: Instant) -> Option<Vec<u8>> {
        match &mut self.state {
            State::Capturing(c) if now >= c.started + TIMEOUT => {
                let raw = std::mem::take(&mut c.raw);
                self.state = State::Done;
                Some(raw)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::modes;

    fn cisco() -> &'static Profile {
        modes::profile("cisco-like").unwrap()
    }

    #[test]
    fn waits_for_a_quiet_prompt_then_captures_until_the_next_prompt() {
        let t0 = Instant::now();
        let mut p = Prober::new(true);
        p.after_output(None, t0); // "Username:" etc.
        assert_eq!(p.deadline(), None);
        p.after_output(Some(CliMode::Privileged), t0);
        assert_eq!(p.deadline(), Some(t0 + QUIET));
        // Typing postpones.
        p.user_input();
        assert_eq!(p.deadline(), None);
        p.after_output(Some(CliMode::Privileged), t0);
        assert_eq!(
            p.fire(Some(CliMode::Privileged), cisco(), t0),
            None,
            "too early"
        );
        let cmd = p
            .fire(Some(CliMode::Privileged), cisco(), t0 + QUIET)
            .unwrap();
        assert_eq!(cmd, b"show version\r");
        assert!(p.capturing());
        assert_eq!(
            p.capture(b"show version\r\nCisco IOS Software, ", cisco()),
            Step::Continue
        );
        assert_eq!(
            p.capture(b"Version 15.2(7)E8\r\n --More-- ", cisco()),
            Step::NextPage
        );
        assert_eq!(
            p.capture(b"\r          \rModel number : WS-C2960X\r\nsw1#", cisco()),
            Step::Finished(
                "show version\nCisco IOS Software, Version 15.2(7)E8\nModel number : WS-C2960X\n"
                    .into()
            )
        );
        assert!(!p.capturing());
        assert_eq!(p.deadline(), None, "once per connection");
    }

    #[test]
    fn config_mode_and_disabled_never_send() {
        let t0 = Instant::now();
        let mut p = Prober::new(true);
        p.after_output(Some(CliMode::Config), t0);
        assert_eq!(p.deadline(), None);
        p.after_output(Some(CliMode::User), t0);
        // Prompt gone by the time the timer fires (e.g. the user started a command).
        assert_eq!(p.fire(None, cisco(), t0 + QUIET), None);
        let mut off = Prober::new(false);
        off.after_output(Some(CliMode::User), t0);
        assert_eq!(off.deadline(), None);
        assert_eq!(off.fire(Some(CliMode::User), cisco(), t0 + QUIET), None);
    }

    #[test]
    fn time_out_gives_the_output_back() {
        let t0 = Instant::now();
        let mut p = Prober::new(true);
        p.after_output(Some(CliMode::User), t0);
        p.fire(Some(CliMode::User), cisco(), t0 + QUIET).unwrap();
        assert_eq!(
            p.capture(b"show version\r\nslow device...", cisco()),
            Step::Continue
        );
        assert_eq!(p.time_out(t0 + QUIET + Duration::from_secs(1)), None);
        let raw = p.time_out(t0 + QUIET + TIMEOUT).unwrap();
        assert_eq!(raw, b"show version\r\nslow device...");
        assert!(!p.capturing());
    }
}
