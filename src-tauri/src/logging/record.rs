//! Session recording (P5-T06) in asciicast v2 (`.cast`): a JSON header line, then one
//! `[seconds, "o", text]` line per output chunk and `[seconds, "r", "COLSxROWS"]` per
//! resize. Like the session log, only RECEIVED output is recorded, never what the user
//! types (passwords), and it streams to disk from its own thread.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::Local;

use crate::error::{AppError, ErrorCode, Result};

pub const TEMPLATE: &str = "{name}_{date}_{time}";
/// Largest recording the player loads.
pub const MAX_READ: u64 = 64 * 1024 * 1024;

enum Msg {
    Output(Instant, Vec<u8>),
    Resize(Instant, u16, u16),
}

pub struct Recording {
    tx: Option<mpsc::Sender<Msg>>,
    path: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Recording {
    pub fn open(dir: &Path, name: &str, cols: u16, rows: u16) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .map_err(|e| AppError::with(ErrorCode::Io, format!("folder {}: {e}", dir.display())))?;
        // `file_name` gives the log's `.log` ending; recordings are `.cast`.
        let base = super::file_name(TEMPLATE, name, Local::now());
        let file = format!("{}.cast", base.strip_suffix(".log").unwrap_or(&base));
        let path = super::unique_path(dir, &file);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| AppError::with(ErrorCode::Io, format!("{}: {e}", path.display())))?;
        let header = serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": Local::now().timestamp(),
            "title": name,
            "env": { "TERM": "xterm-256color" },
        });
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        let thread = std::thread::Builder::new()
            .name("session-record".into())
            .spawn(move || writer(file, &header.to_string(), start, rx))
            .map_err(|e| AppError::internal(format!("record thread: {e}")))?;
        Ok(Self {
            tx: Some(tx),
            path,
            thread: Some(thread),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn output(&self, bytes: &[u8]) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Output(Instant::now(), bytes.to_vec()));
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Resize(Instant::now(), cols, rows));
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// UTF-8 text of `pending` + `bytes`; an incomplete sequence at the end stays pending
/// for the next chunk, invalid bytes become U+FFFD.
fn decode(pending: &mut Vec<u8>, bytes: &[u8]) -> String {
    pending.extend_from_slice(bytes);
    let mut out = String::with_capacity(pending.len());
    let mut rest: &[u8] = pending;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                rest = &[];
                break;
            }
            Err(e) => {
                let (ok, after) = rest.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(ok).unwrap_or_default());
                match e.error_len() {
                    Some(n) => {
                        out.push('\u{FFFD}');
                        rest = &after[n..];
                    }
                    // Cut mid-character: keep it for the next chunk.
                    None => {
                        rest = after;
                        break;
                    }
                }
            }
        }
    }
    *pending = rest.to_vec();
    out
}

fn writer(file: File, header: &str, start: Instant, rx: mpsc::Receiver<Msg>) {
    let mut w = BufWriter::new(file);
    let _ = writeln!(w, "{header}");
    let mut pending = Vec::new();
    let mut last_flush = Instant::now();
    let secs = |at: Instant| (at.saturating_duration_since(start).as_micros() as f64) / 1e6;
    loop {
        let msg = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(m) => Some(m),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let line = match msg {
            Some(Msg::Output(at, bytes)) => {
                let text = decode(&mut pending, &bytes);
                (!text.is_empty()).then(|| serde_json::json!([secs(at), "o", text]))
            }
            Some(Msg::Resize(at, c, r)) => {
                Some(serde_json::json!([secs(at), "r", format!("{c}x{r}")]))
            }
            None => None,
        };
        if let Some(l) = line
            && writeln!(w, "{l}").is_err()
        {
            break;
        }
        if last_flush.elapsed() >= Duration::from_secs(1) {
            let _ = w.flush();
            last_flush = Instant::now();
        }
    }
    let _ = w.flush();
}

/// A recording's text for the player (bounded).
pub fn read(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).map_err(AppError::from)?;
    if meta.len() > MAX_READ {
        return Err(AppError::invalid("recording is larger than 64 MB"));
    }
    let text = std::fs::read_to_string(path).map_err(AppError::from)?;
    let first = text.lines().next().unwrap_or("");
    let ok = serde_json::from_str::<serde_json::Value>(first)
        .ok()
        .and_then(|h| h.get("version").and_then(serde_json::Value::as_u64))
        == Some(2);
    if ok {
        Ok(text)
    } else {
        Err(AppError::invalid("not an asciicast v2 recording"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_split_across_chunks() {
        let mut p = Vec::new();
        let s = "şğü→€".as_bytes();
        let a = decode(&mut p, &s[..3]);
        let b = decode(&mut p, &s[3..]);
        assert_eq!(format!("{a}{b}"), "şğü→€");
        assert!(p.is_empty());
        assert_eq!(decode(&mut p, b"a\xffb"), "a\u{FFFD}b");
    }

    #[test]
    fn writes_a_playable_cast() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recording::open(dir.path(), "core-sw1", 120, 40).unwrap();
        let path = rec.path().to_owned();
        assert!(path.extension().is_some_and(|e| e == "cast"));
        rec.output(b"core-sw1#show ver\r\n\x1b[1mCisco\x1b[0m \xc5");
        rec.output(b"\x9f\r\n");
        rec.resize(100, 30);
        drop(rec);
        let text = read(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["width"], 120);
        assert_eq!(lines[0]["title"], "core-sw1");
        let out: String = lines[1..]
            .iter()
            .filter(|e| e[1] == "o")
            .map(|e| e[2].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(out, "core-sw1#show ver\r\n\x1b[1mCisco\x1b[0m ş\r\n");
        assert_eq!(lines.last().unwrap()[1], "r");
        assert_eq!(lines.last().unwrap()[2], "100x30");
        let t: Vec<f64> = lines[1..].iter().map(|e| e[0].as_f64().unwrap()).collect();
        assert!(t.windows(2).all(|w| w[0] <= w[1]));

        // A second recording of the same name never overwrites the first.
        let again = Recording::open(dir.path(), "core-sw1", 80, 24).unwrap();
        assert_ne!(again.path(), path);

        std::fs::write(dir.path().join("x.cast"), "not json\n").unwrap();
        assert!(read(&dir.path().join("x.cast")).is_err());
    }
}
