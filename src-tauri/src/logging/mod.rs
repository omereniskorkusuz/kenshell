//! Session logging (P2-T17): received device output is streamed to a file on a
//! dedicated writer thread (never accumulated in memory).
//!
//! * Only RECEIVED bytes are logged — never what the user types — so passwords typed
//!   at device prompts (which devices do not echo) cannot end up in a log.
//! * Modes: `printable` (ANSI/control sequences stripped, CRLF → LF, optional
//!   `[YYYY-MM-DD HH:MM:SS] ` line timestamps) and `all` (raw bytes).
//! * File name from a template: `{name}` (`{hostname}` alias), `{date}`, `{time}`.

pub mod record;

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

pub const DEFAULT_TEMPLATE: &str = "{name}_{date}_{time}.log";
/// The writer flushes at least this often (so a crash loses at most ~1 s of log).
const FLUSH_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogMode {
    #[default]
    Printable,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub dir: PathBuf,
    pub template: String,
    pub mode: LogMode,
    pub timestamps: bool,
}

/// `~/Documents/KenShell Logs` (falls back to the temp dir when there is no home).
pub fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    match home {
        Some(h) => h.join("Documents").join("KenShell Logs"),
        None => std::env::temp_dir().join("KenShell Logs"),
    }
}

/// Replaces characters that are invalid or awkward in file names.
fn sanitize(part: &str) -> String {
    let s: String = part
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let s = s.trim_matches(['.', ' ']).to_owned();
    if s.is_empty() { "session".into() } else { s }
}

pub fn file_name(template: &str, name: &str, now: DateTime<Local>) -> String {
    let tpl = if template.trim().is_empty() {
        DEFAULT_TEMPLATE
    } else {
        template
    };
    let name = sanitize(name);
    let out = tpl
        .replace("{name}", &name)
        .replace("{hostname}", &name)
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H%M%S").to_string());
    let out = sanitize(&out);
    if out.to_ascii_lowercase().ends_with(".log") {
        out
    } else {
        format!("{out}.log")
    }
}

/// Picks a path that does not exist yet (`x.log`, `x (2).log`, …).
fn unique_path(dir: &Path, file: &str) -> PathBuf {
    let first = dir.join(file);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = file.rsplit_once('.').unwrap_or((file, "log"));
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Esc {
    None,
    /// Saw ESC.
    Start,
    /// CSI: ESC [ … final byte 0x40–0x7E.
    Csi,
    /// OSC / DCS / etc.: until BEL or ESC \.
    Str,
    StrEnd,
}

/// Turns terminal output into readable text (pure; tested).
#[derive(Debug)]
pub struct Printable {
    timestamps: bool,
    line_start: bool,
    esc: Esc,
    pending_cr: bool,
}

impl Printable {
    pub fn new(timestamps: bool) -> Self {
        Self {
            timestamps,
            line_start: true,
            esc: Esc::None,
            pending_cr: false,
        }
    }

    pub fn feed(&mut self, input: &[u8], now: impl Fn() -> String, out: &mut Vec<u8>) {
        for &b in input {
            match self.esc {
                Esc::Start => {
                    self.esc = match b {
                        b'[' => Esc::Csi,
                        b']' | b'P' | b'_' | b'^' | b'X' => Esc::Str,
                        _ => Esc::None, // two-byte sequence (ESC c, ESC =, …)
                    };
                    continue;
                }
                Esc::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.esc = Esc::None;
                    }
                    continue;
                }
                Esc::Str => {
                    self.esc = match b {
                        0x07 => Esc::None,
                        0x1b => Esc::StrEnd,
                        _ => Esc::Str,
                    };
                    continue;
                }
                Esc::StrEnd => {
                    self.esc = if b == b'\\' { Esc::None } else { Esc::Str };
                    continue;
                }
                Esc::None => {}
            }
            match b {
                0x1b => self.esc = Esc::Start,
                b'\r' => self.pending_cr = true,
                b'\n' => {
                    self.pending_cr = false;
                    if self.timestamps && self.line_start {
                        out.extend(now().as_bytes());
                    }
                    out.push(b'\n');
                    self.line_start = true;
                }
                b'\t' | 0x20..=0xff => {
                    // A lone CR (progress spinners, --More-- erasure) does not start a
                    // new line in the log; the text just continues.
                    self.pending_cr = false;
                    if b == 0x7f {
                        continue;
                    }
                    if self.timestamps && self.line_start {
                        out.extend(now().as_bytes());
                    }
                    self.line_start = false;
                    out.push(b);
                }
                // Backspace: drop the last character of the current line if any.
                0x08 if !self.line_start && out.last().is_some_and(|c| *c != b'\n') => {
                    out.pop();
                }
                _ => {} // BEL, NUL and other C0 controls
            }
        }
    }
}

fn timestamp() -> String {
    Local::now().format("[%Y-%m-%d %H:%M:%S] ").to_string()
}

enum Msg {
    Data(Vec<u8>),
}

/// An open session log. Dropping it flushes and closes the file.
pub struct SessionLog {
    tx: Option<mpsc::Sender<Msg>>,
    path: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SessionLog {
    pub fn open(cfg: &LogConfig, name: &str) -> Result<Self> {
        std::fs::create_dir_all(&cfg.dir).map_err(|e| {
            AppError::with(
                crate::error::ErrorCode::Io,
                format!("log folder {}: {e}", cfg.dir.display()),
            )
        })?;
        let path = unique_path(&cfg.dir, &file_name(&cfg.template, name, Local::now()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                AppError::with(
                    crate::error::ErrorCode::Io,
                    format!("{}: {e}", path.display()),
                )
            })?;
        let (tx, rx) = mpsc::channel::<Msg>();
        let mode = cfg.mode;
        let timestamps = cfg.timestamps;
        let shown = path.display().to_string();
        let thread = std::thread::Builder::new()
            .name("session-log".into())
            .spawn(move || writer(file, rx, mode, timestamps, &shown))
            .map_err(|e| AppError::internal(format!("log thread: {e}")))?;
        Ok(Self {
            tx: Some(tx),
            path,
            thread: Some(thread),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Queues received bytes (never blocks on disk).
    pub fn write(&self, bytes: &[u8]) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Data(bytes.to_vec()));
        }
    }
}

impl Drop for SessionLog {
    fn drop(&mut self) {
        self.tx.take(); // closes the channel → writer flushes and exits
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn writer(file: File, rx: mpsc::Receiver<Msg>, mode: LogMode, timestamps: bool, path: &str) {
    let mut w = BufWriter::with_capacity(64 * 1024, file);
    let mut filter = Printable::new(timestamps);
    let mut buf = Vec::new();
    loop {
        match rx.recv_timeout(FLUSH_EVERY) {
            Ok(Msg::Data(bytes)) => {
                let res = match mode {
                    LogMode::All => w.write_all(&bytes),
                    LogMode::Printable => {
                        buf.clear();
                        filter.feed(&bytes, timestamp, &mut buf);
                        w.write_all(&buf)
                    }
                };
                if let Err(e) = res {
                    tracing::warn!("session log {path}: {e}");
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = w.flush();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Err(e) = w.flush() {
        tracing::warn!("session log {path}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn printable(input: &[u8], ts: bool) -> String {
        let mut p = Printable::new(ts);
        let mut out = Vec::new();
        p.feed(input, || "[T] ".to_string(), &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn strips_ansi_and_normalizes_newlines() {
        let s = printable(
            b"\x1b[1;32mSwitch#\x1b[0m show ver\r\n\x1b]0;title\x07Line2\r\r\nx\x08y\x07\r\n",
            false,
        );
        assert_eq!(s, "Switch# show ver\nLine2\ny\n");
    }

    #[test]
    fn more_prompt_erasure_does_not_break_lines() {
        let s = printable(
            b"a\r\n --More-- \x08\x08\x08\x08\x08\x08\x08\x08\x08\x08          \rb\r\n",
            false,
        );
        assert!(s.starts_with("a\n"));
        assert!(s.ends_with("b\n"));
    }

    #[test]
    fn timestamps_at_line_starts_only_across_chunks() {
        let mut p = Printable::new(true);
        let mut out = Vec::new();
        p.feed(b"one\r\ntw", || "[T] ".into(), &mut out);
        p.feed(b"o\r\n", || "[T] ".into(), &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "[T] one\n[T] two\n");
    }

    #[test]
    fn file_names_are_safe() {
        let now = Local.with_ymd_and_hms(2026, 11, 4, 9, 5, 7).unwrap();
        assert_eq!(
            file_name(DEFAULT_TEMPLATE, "core-sw-01", now),
            "core-sw-01_2026-11-04_090507.log"
        );
        assert_eq!(
            file_name("{hostname}-{date}", "10.0.0.1:22", now),
            "10.0.0.1-22-2026-11-04.log"
        );
        assert_eq!(
            file_name("", "../../etc/x", now),
            "-..-etc-x_2026-11-04_090507.log"
        );
        assert_eq!(file_name("{name}", "  ", now), "session.log");
    }

    #[test]
    fn writes_only_what_it_receives_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LogConfig {
            dir: dir.path().to_path_buf(),
            template: "{name}".into(),
            mode: LogMode::Printable,
            timestamps: false,
        };
        let a = SessionLog::open(&cfg, "sw1").unwrap();
        let b = SessionLog::open(&cfg, "sw1").unwrap();
        assert_ne!(
            a.path(),
            b.path(),
            "second log must not overwrite the first"
        );
        // The session only ever calls write() with RECEIVED data; a password typed by
        // the user is sent to the device but never passed here.
        a.write(b"Password: \r\nSwitch>\x1b[0m");
        let path = a.path().to_path_buf();
        drop(a);
        drop(b);
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text, "Password: \nSwitch>");
    }

    #[test]
    fn raw_mode_keeps_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LogConfig {
            dir: dir.path().to_path_buf(),
            template: "raw".into(),
            mode: LogMode::All,
            timestamps: true,
        };
        let log = SessionLog::open(&cfg, "x").unwrap();
        log.write(b"\x1b[0mA\r\n");
        let path = log.path().to_path_buf();
        drop(log);
        assert_eq!(std::fs::read(path).unwrap(), b"\x1b[0mA\r\n");
    }
}
