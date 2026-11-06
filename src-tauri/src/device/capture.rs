//! Hidden command capture (P3-T04 identity command, generalised in P4-T06): the session
//! sends a read-only command at a quiet prompt, keeps its output out of the terminal and
//! the log, answers the pager and hands back plain text when the prompt returns.
//!
//! Only read-only commands may be captured ([`read_only`]): KenShell never sends a
//! configuration command on its own.

use std::time::Instant;

use super::lines::LineAssembler;
use super::modes::Profile;

/// What the session does with output that arrived during a capture.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Keep capturing (nothing to send).
    Continue,
    /// A `--More--` page: send a space.
    NextPage,
    /// The prompt is back: here is the plain-text output (prompt line excluded; the first
    /// line is the echoed command).
    Finished(String),
}

#[derive(Debug)]
pub struct Capture {
    started: Instant,
    max: usize,
    max_pages: u32,
    raw: Vec<u8>,
    lines: LineAssembler,
    text: String,
    completed: usize,
    pages: u32,
}

impl Capture {
    /// `max` bytes of output are kept (raw and text each); at most `max_pages` pages.
    pub fn new(now: Instant, max: usize, max_pages: u32) -> Self {
        Self {
            started: now,
            max,
            max_pages,
            raw: Vec::new(),
            lines: LineAssembler::new(),
            text: String::new(),
            completed: 0,
            pages: 0,
        }
    }

    pub fn started(&self) -> Instant {
        self.started
    }

    pub fn feed(&mut self, bytes: &[u8], profile: &Profile) -> Step {
        if self.raw.len() + bytes.len() <= self.max {
            self.raw.extend_from_slice(bytes);
        }
        let (text, completed, max) = (&mut self.text, &mut self.completed, self.max);
        self.lines.feed(bytes, |line| {
            *completed += 1;
            if text.len() < max {
                text.push_str(&line);
                text.push('\n');
            }
        });
        let current = self.lines.current().trim_end();
        // The first completed line is the echo of the command; the prompt after the output
        // ends the capture.
        if self.completed >= 1 && profile.match_prompt(current).is_some() {
            return Step::Finished(std::mem::take(&mut self.text));
        }
        if current.contains(profile.more.as_str()) && self.pages < self.max_pages {
            self.pages += 1;
            return Step::NextPage;
        }
        Step::Continue
    }

    /// What arrived so far, for showing it after all (time-out).
    pub fn into_raw(self) -> Vec<u8> {
        self.raw
    }
}

/// Pipe filters that only narrow the output (`| redirect`, `| tee`, `| append` write files).
const SAFE_PIPES: &[&str] = &[
    "include", "exclude", "begin", "section", "count", "i", "e", "b", "s", "inc", "exc", "beg",
    "sec", "no-more", "match", "except", "find",
];

/// A single-line `show …` / `display …` command (abbreviations allowed), optionally
/// filtered with a read-only pipe. Everything else is refused.
pub fn read_only(command: &str) -> bool {
    let c = command.trim();
    if c.is_empty() || c.len() > 300 || c.chars().any(char::is_control) || c.contains(';') {
        return false;
    }
    let mut parts = c.split('|');
    let head = parts.next().unwrap_or("");
    let first = head
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let verb_ok = first.len() >= 2 && ("show".starts_with(&first) || "display".starts_with(&first));
    verb_ok
        && parts.all(|p| {
            let w = p
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            SAFE_PIPES.contains(&w.as_str())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::modes;

    #[test]
    fn only_read_only_commands() {
        for ok in [
            "show version",
            "sh ip int br",
            "display interface brief",
            "show logging | include %LINK",
            "show run | sec interface",
        ] {
            assert!(read_only(ok), "{ok}");
        }
        for bad in [
            "conf t",
            "configure terminal",
            "reload",
            "write memory",
            "copy run start",
            "s version",
            "show run | redirect flash:x",
            "show run | tee flash:x",
            "show ver; reload",
            "show ver\rreload",
            "",
            "debug all",
            "clear counters",
        ] {
            assert!(!read_only(bad), "{bad}");
        }
    }

    #[test]
    fn captures_until_the_prompt_answering_the_pager() {
        let p = modes::profile("cisco-like").unwrap();
        let mut c = Capture::new(Instant::now(), 1024, 5);
        assert_eq!(c.feed(b"show clock\r\n*10:00:01 UTC", p), Step::Continue);
        assert_eq!(c.feed(b"\r\n --More-- ", p), Step::NextPage);
        assert_eq!(
            c.feed(b"\r        \rWed\r\nsw1#", p),
            Step::Finished("show clock\n*10:00:01 UTC\nWed\n".into())
        );
    }
}
