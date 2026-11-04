//! Automatic baud detection — P1-T12. `score` is a PURE function (unit-tested with
//! sample byte streams); `detect` runs the candidate loop over a [`Probe`].
//!
//! Algorithm (spec 7.2): candidates = remembered → 9600, 115200, 38400, 57600, 19200,
//! 4800, 2400. Per candidate: set baud, flush, send `\r`, read ~600 ms; if nothing, try
//! `\r\n` once. Score = printable ASCII + CR/LF/TAB ratio, + bonus if a known prompt
//! pattern matches. Accept when score ≥ 0.85 AND prompt matched; otherwise suggest best.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::error::{AppError, ErrorCode, Result};
use crate::model::{BaudTrial, FlowControl, Parity, SerialSettings};

pub const DEFAULT_CANDIDATES: [u32; 7] = [9600, 115200, 38400, 57600, 19200, 4800, 2400];
pub const ACCEPT_SCORE: f32 = 0.85;

/// Bonus added to the printable ratio when a prompt pattern matched.
const PROMPT_BONUS: f32 = 0.15;
/// Inputs shorter than this cannot be judged: value is capped at [`SHORT_INPUT_CAP`].
const MIN_BYTES: usize = 2;
const SHORT_INPUT_CAP: f32 = 0.3;
/// NUL / 0xFF share above which the stream is treated as a framing-error flood.
const FLOOD_SHARE: f32 = 0.25;
/// Longest line still considered a prompt.
const MAX_PROMPT_LEN: usize = 80;

/// Case-insensitive keywords that identify a login / boot prompt.
const PROMPT_KEYWORDS: &[&str] = &[
    "login:",
    "username:",
    "password:",
    "rommon",
    "loader>",
    "ruijie",
    "bootloader",
    "press return",
    "user access verification",
    "switch:",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub value: f32,
    pub printable_ratio: f32,
    pub prompt_match: bool,
    pub bytes: usize,
}

/// Remembered first (deduplicated), then the defaults.
pub fn candidate_order(remembered: Option<u32>) -> Vec<u32> {
    let mut out = Vec::with_capacity(DEFAULT_CANDIDATES.len() + 1);
    if let Some(b) = remembered.filter(|&b| b > 0) {
        out.push(b);
    }
    for b in DEFAULT_CANDIDATES {
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

fn is_printable(b: u8) -> bool {
    matches!(b, 0x20..=0x7E | b'\r' | b'\n' | b'\t')
}

/// A trimmed line that looks like a CLI prompt: `Switch>`, `access-sw-02#`,
/// `[~HUAWEI]`, `<HUAWEI>`, `rommon 1 >`, `user@mx> `, `Router(config-if)#`.
fn is_prompt_line(line: &str) -> bool {
    let line = line.trim();
    let len = line.len();
    if !(2..=MAX_PROMPT_LEN).contains(&len) || !line.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return false;
    }
    let Some(first) = line.chars().next() else {
        return false;
    };
    let ends_ok = line.ends_with(['>', '#', ']']);
    let starts_ok = first.is_ascii_alphanumeric() || matches!(first, '<' | '[' | '(' | '_');
    ends_ok && starts_ok && line.chars().any(|c| c.is_ascii_alphanumeric())
}

fn prompt_matches(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if PROMPT_KEYWORDS.iter().any(|k| text.contains(k)) {
        return true;
    }
    let lines: Vec<&str> = text
        .split(['\r', '\n'])
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines.iter().rev().take(3).any(|l| is_prompt_line(l))
}

/// Scores what the device answered at one baud rate. PURE.
pub fn score(bytes: &[u8]) -> Score {
    let n = bytes.len();
    if n == 0 {
        return Score {
            value: 0.0,
            printable_ratio: 0.0,
            prompt_match: false,
            bytes: 0,
        };
    }
    let printable = bytes.iter().filter(|&&b| is_printable(b)).count();
    let flood = bytes.iter().filter(|&&b| b == 0x00 || b == 0xFF).count();
    let printable_ratio = printable as f32 / n as f32;
    let flood_share = flood as f32 / n as f32;

    let prompt_match = n >= MIN_BYTES && prompt_matches(bytes);
    let mut value = printable_ratio + if prompt_match { PROMPT_BONUS } else { 0.0 };
    // Wrong baud rates produce framing errors that read as NUL/0xFF runs: they already
    // lower the printable ratio; a flood is penalized on top so a stray prompt-looking
    // fragment inside it can never be accepted.
    if flood_share >= FLOOD_SHARE {
        value -= flood_share;
    }
    if n < MIN_BYTES {
        value = value.min(SHORT_INPUT_CAP);
    }
    Score {
        value: value.clamp(0.0, 1.0),
        printable_ratio,
        prompt_match,
        bytes: n,
    }
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

/// Errors that make further probing pointless (the port itself is unusable).
fn is_fatal(e: &AppError) -> bool {
    matches!(
        e.code,
        ErrorCode::PortBusy
            | ErrorCode::PortNotFound
            | ErrorCode::PermissionDenied
            | ErrorCode::DeviceGone
    )
}

/// Stops at the first accepted candidate. `on_trial` is called after each probe.
pub fn detect(
    probe: &mut dyn Probe,
    remembered: Option<u32>,
    on_trial: &mut dyn FnMut(&BaudTrial),
) -> Result<Outcome> {
    let mut trials: Vec<BaudTrial> = Vec::new();
    let mut accepted = None;
    for baud in candidate_order(remembered) {
        let s = match probe.probe(baud) {
            Ok(bytes) => score(&bytes),
            Err(e) if is_fatal(&e) => return Err(e),
            Err(e) => {
                tracing::debug!("autobaud probe at {baud} failed: {e}");
                score(&[])
            }
        };
        let trial = BaudTrial {
            baud,
            score: s.value,
            prompt_match: s.prompt_match,
            bytes: s.bytes,
        };
        on_trial(&trial);
        trials.push(trial);
        if is_accepted(&s) {
            accepted = Some(baud);
            break;
        }
    }

    let suggestion = if accepted.is_some() {
        None
    } else {
        // Highest score wins; on ties the earlier candidate (remembered / more common).
        trials
            .iter()
            .filter(|t| t.bytes > 0)
            .fold(None::<&BaudTrial>, |best, t| match best {
                Some(b) if b.score >= t.score => Some(b),
                _ => Some(t),
            })
            .map(|t| t.baud)
    };
    let no_data = trials.iter().all(|t| t.bytes == 0);
    Ok(Outcome {
        accepted,
        suggestion,
        trials,
        no_data,
    })
}

// ------------------------------------------------------------------ real serial probe

/// Per-read timeout: bounds every blocking read (no busy loop, quick early stop).
const READ_TIMEOUT: Duration = Duration::from_millis(50);
/// Listening window per wake-up string.
const LISTEN: Duration = Duration::from_millis(600);
/// Silence after a prompt that ends a listening window early.
const PROMPT_SILENCE: Duration = Duration::from_millis(120);
/// Lets bytes still in flight at the old rate arrive before the buffers are flushed.
const SETTLE: Duration = Duration::from_millis(20);
const MAX_PROBE_BYTES: usize = 4096;

/// Real serial-port probe: opens the port once and changes the baud per candidate.
pub struct SerialProbe {
    path: String,
    port: Box<dyn serialport::SerialPort>,
}

impl SerialProbe {
    pub fn open(settings: &SerialSettings) -> Result<Self> {
        let path = settings.port.trim();
        if path.is_empty() {
            return Err(AppError::invalid("empty port path"));
        }
        let data_bits = match settings.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            8 => serialport::DataBits::Eight,
            other => return Err(AppError::invalid(format!("data bits {other}"))),
        };
        let stop_bits = match settings.stop_bits {
            1 => serialport::StopBits::One,
            2 => serialport::StopBits::Two,
            other => return Err(AppError::invalid(format!("stop bits {other}"))),
        };
        let parity = match settings.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        };
        let flow = match settings.flow_control {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::RtsCts => serialport::FlowControl::Hardware,
            FlowControl::XonXoff => serialport::FlowControl::Software,
        };
        let first = settings.baud.unwrap_or(DEFAULT_CANDIDATES[0]);
        let mut port = serialport::new(path, first)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .parity(parity)
            .flow_control(flow)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(|e| crate::platform::classify_open_error(path, &e))?;
        // Line states are best effort: some drivers reject them and consoles do not care.
        if let Some(dtr) = settings.dtr
            && let Err(e) = port.write_data_terminal_ready(dtr)
        {
            tracing::debug!("set DTR on {path}: {e}");
        }
        if let Some(rts) = settings.rts
            && let Err(e) = port.write_request_to_send(rts)
        {
            tracing::debug!("set RTS on {path}: {e}");
        }
        Ok(Self {
            path: path.to_owned(),
            port,
        })
    }

    /// Maps an I/O failure during probing: a vanished port is `DeviceGone`.
    fn io_error(&self, what: &str, detail: impl std::fmt::Display) -> AppError {
        let code = if crate::platform::port_exists(&self.path) {
            ErrorCode::Io
        } else {
            ErrorCode::DeviceGone
        };
        AppError::with(code, format!("{}: {what}: {detail}", self.path))
    }

    /// Sends `wake` and collects the answer for up to [`LISTEN`].
    fn exchange(&mut self, wake: &[u8], buf: &mut Vec<u8>) -> Result<()> {
        self.port
            .write_all(wake)
            .and_then(|()| self.port.flush())
            .map_err(|e| self.io_error("write", e))?;
        let deadline = Instant::now() + LISTEN;
        let mut last_rx: Option<Instant> = None;
        let mut chunk = [0u8; 512];
        loop {
            let now = Instant::now();
            if now >= deadline || buf.len() >= MAX_PROBE_BYTES {
                break;
            }
            match self.port.read(&mut chunk) {
                Ok(0) => {}
                Ok(n) => {
                    let room = MAX_PROBE_BYTES - buf.len();
                    buf.extend_from_slice(&chunk[..n.min(room)]);
                    last_rx = Some(Instant::now());
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(self.io_error("read", e)),
            }
            if let Some(t) = last_rx
                && t.elapsed() >= PROMPT_SILENCE
                && prompt_matches(buf)
            {
                break;
            }
        }
        Ok(())
    }
}

impl Probe for SerialProbe {
    fn probe(&mut self, baud: u32) -> Result<Vec<u8>> {
        self.port.set_baud_rate(baud).map_err(|e| {
            AppError::with(ErrorCode::Io, format!("{}: baud {baud}: {e}", self.path))
        })?;
        std::thread::sleep(SETTLE);
        self.port
            .clear(serialport::ClearBuffer::All)
            .map_err(|e| self.io_error("flush", e))?;
        let mut buf = Vec::new();
        self.exchange(b"\r", &mut buf)?;
        if buf.is_empty() {
            self.exchange(b"\r\n", &mut buf)?;
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(bytes: &[u8]) -> bool {
        is_accepted(&score(bytes))
    }

    #[test]
    fn candidate_order_puts_remembered_first() {
        assert_eq!(candidate_order(None), DEFAULT_CANDIDATES.to_vec());
        assert_eq!(
            candidate_order(Some(115200)),
            [115200, 9600, 38400, 57600, 19200, 4800, 2400]
        );
        assert_eq!(
            candidate_order(Some(1200)),
            [1200, 9600, 115200, 38400, 57600, 19200, 4800, 2400]
        );
        assert_eq!(candidate_order(Some(0)), DEFAULT_CANDIDATES.to_vec());
    }

    #[test]
    fn cisco_user_prompt_is_accepted() {
        let s = score(b"\r\nSwitch>");
        assert!(s.prompt_match);
        assert_eq!(s.printable_ratio, 1.0);
        assert_eq!(s.value, 1.0);
        assert!(is_accepted(&s));
        assert!(accepted(b"\r\n\r\nRouter(config-if)#"));
        assert!(accepted(
            b"\r\n\r\nPress RETURN to get started!\r\n\r\n\r\n"
        ));
        assert!(accepted(
            b"\r\n\r\nUser Access Verification\r\n\r\nPassword: "
        ));
    }

    #[test]
    fn ruijie_prompts_are_accepted() {
        assert!(accepted(b"\r\nRuijie>"));
        assert!(accepted(b"\r\naccess-sw-02#"));
        assert!(accepted(b"\r\n\r\naccess-sw-02(config)#"));
    }

    #[test]
    fn login_prompts_are_accepted() {
        assert!(accepted(b"\r\nUsername: "));
        assert!(accepted(b"\r\n\r\nPassword:"));
        assert!(accepted(b"\r\n\r\nmx204-lab (ttyu0)\r\n\r\nlogin: "));
        assert!(accepted(b"\r\nadmin@mx204-lab> "));
    }

    #[test]
    fn boot_prompts_are_accepted() {
        assert!(accepted(b"\r\nrommon 1 > "));
        assert!(accepted(b"\r\nswitch: "));
        assert!(accepted(b"\r\nloader> "));
        assert!(accepted(b"\r\nBootloader version 1.2\r\n"));
    }

    #[test]
    fn huawei_prompts_are_accepted() {
        assert!(accepted(b"\r\n<HUAWEI>"));
        assert!(accepted(b"\r\n[~HUAWEI]"));
        assert!(accepted(b"\r\n[HUAWEI-GigabitEthernet0/0/1]"));
    }

    #[test]
    fn wrong_baud_garbage_is_rejected() {
        let garbage = [
            0x00, 0xF8, 0x80, 0xFE, 0x1C, 0xE0, 0x00, 0x78, 0xF8, 0x00, 0x80, 0xFE, 0xE6, 0x00,
            0x1E, 0x98,
        ];
        let s = score(&garbage);
        assert!(!s.prompt_match);
        assert!(s.value < 0.3, "{s:?}");
        assert!(!is_accepted(&s));
    }

    #[test]
    fn nul_flood_with_prompt_like_tail_is_rejected() {
        let mut bytes = vec![0x00; 40];
        bytes.extend_from_slice(b"\r\nab#");
        let s = score(&bytes);
        assert!(s.prompt_match);
        assert!(!is_accepted(&s), "{s:?}");
        let mut ff = vec![0xFF; 30];
        ff.extend_from_slice(b"\r\nSwitch>");
        assert!(!accepted(&ff));
    }

    #[test]
    fn printable_noise_without_prompt_is_not_accepted() {
        // Correct baud but a boot log mid-way: high ratio, no prompt → suggestion only.
        let s = score(b"Loading flash:/c2960-lanbasek9-mz.150-2.SE11.bin...\r\n######");
        assert!(s.value >= 0.95);
        assert!(!s.prompt_match);
        assert!(!is_accepted(&s));
    }

    #[test]
    fn mixed_noise_is_scored_in_between() {
        // Half garbage, half text with a prompt: prompt matches but ratio too low.
        let mut bytes = vec![0x80, 0xF8, 0x9C, 0xE0, 0x86, 0xFC, 0x1C, 0x8E, 0x92, 0xC0];
        bytes.extend_from_slice(b"\r\nSw>");
        let s = score(&bytes);
        assert!(s.value > 0.2 && s.value < ACCEPT_SCORE, "{s:?}");
        assert!(!is_accepted(&s));
    }

    #[test]
    fn prompt_symbol_inside_garbage_is_not_a_prompt() {
        // A '#' or '>' alone on a line must not count.
        let s = score(b"\x8c\x9e\r\n#\r\n>\r\n");
        assert!(!s.prompt_match, "{s:?}");
        // Prompts only count in the last three lines.
        let s = score(b"Switch>\r\nfoo\r\nbar\r\nbaz\r\n");
        assert!(!s.prompt_match);
    }

    #[test]
    fn empty_and_tiny_inputs() {
        let s = score(&[]);
        assert_eq!(
            s,
            Score {
                value: 0.0,
                printable_ratio: 0.0,
                prompt_match: false,
                bytes: 0
            }
        );
        let s = score(b"\n");
        assert!(s.value <= SHORT_INPUT_CAP);
        assert!(!s.prompt_match);
        assert_eq!(s.bytes, 1);
        assert!(!accepted(b"#"));
    }

    // ----------------------------------------------------------- detect()

    /// Fake device answering at `real` baud only.
    struct FakeProbe {
        answers: Vec<(u32, Result<Vec<u8>>)>,
        default: Vec<u8>,
        calls: Vec<u32>,
    }

    impl FakeProbe {
        fn new(default: &[u8]) -> Self {
            Self {
                answers: Vec::new(),
                default: default.to_vec(),
                calls: Vec::new(),
            }
        }
        fn at(mut self, baud: u32, answer: Result<Vec<u8>>) -> Self {
            self.answers.push((baud, answer));
            self
        }
    }

    impl Probe for FakeProbe {
        fn probe(&mut self, baud: u32) -> Result<Vec<u8>> {
            self.calls.push(baud);
            self.answers
                .iter()
                .find(|(b, _)| *b == baud)
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| Ok(self.default.clone()))
        }
    }

    const GARBAGE: &[u8] = &[0x00, 0xF8, 0x80, 0xFE, 0x1C, 0xE0, 0x00, 0x78];

    #[test]
    fn detect_tries_remembered_first_and_stops_early() {
        let mut p = FakeProbe::new(GARBAGE).at(57600, Ok(b"\r\nSwitch>".to_vec()));
        let mut seen = Vec::new();
        let out = detect(&mut p, Some(57600), &mut |t| seen.push(t.baud)).unwrap();
        assert_eq!(p.calls, [57600]);
        assert_eq!(seen, [57600]);
        assert_eq!(out.accepted, Some(57600));
        assert_eq!(out.suggestion, None);
        assert!(!out.no_data);
        assert!(out.trials[0].prompt_match);
    }

    #[test]
    fn detect_walks_defaults_until_accepted() {
        let mut p = FakeProbe::new(GARBAGE).at(38400, Ok(b"\r\nRuijie#".to_vec()));
        let mut n = 0;
        let out = detect(&mut p, None, &mut |_| n += 1).unwrap();
        assert_eq!(p.calls, [9600, 115200, 38400]);
        assert_eq!(n, 3);
        assert_eq!(out.accepted, Some(38400));
        assert_eq!(out.trials.len(), 3);
    }

    #[test]
    fn detect_suggests_best_when_nothing_accepted() {
        let mut p = FakeProbe::new(&[])
            .at(9600, Ok(GARBAGE.to_vec()))
            .at(
                115200,
                Ok(b"%SYS-5-CONFIG_I: Configured from console\r\n".to_vec()),
            )
            .at(19200, Ok(b"\x80\xF8abc".to_vec()));
        let out = detect(&mut p, None, &mut |_| {}).unwrap();
        assert_eq!(p.calls.len(), DEFAULT_CANDIDATES.len());
        assert_eq!(out.accepted, None);
        assert_eq!(out.suggestion, Some(115200));
        assert!(!out.no_data);
    }

    #[test]
    fn detect_reports_no_data() {
        let mut p = FakeProbe::new(&[]);
        let out = detect(&mut p, Some(9600), &mut |_| {}).unwrap();
        // Remembered 9600 is deduplicated against the defaults.
        assert_eq!(p.calls, DEFAULT_CANDIDATES.to_vec());
        assert!(out.no_data);
        assert_eq!(out.accepted, None);
        assert_eq!(out.suggestion, None);
        assert!(out.trials.iter().all(|t| t.score == 0.0));
    }

    #[test]
    fn detect_propagates_busy_and_skips_soft_errors() {
        let mut p = FakeProbe::new(&[]).at(9600, Err(AppError::with(ErrorCode::PortBusy, "COM4")));
        let err = detect(&mut p, None, &mut |_| {}).unwrap_err();
        assert_eq!(err.code, ErrorCode::PortBusy);
        assert_eq!(p.calls, [9600]);

        let mut p = FakeProbe::new(&[])
            .at(9600, Err(AppError::with(ErrorCode::Io, "baud rejected")))
            .at(115200, Ok(b"\r\nSwitch#".to_vec()));
        let mut trials = Vec::new();
        let out = detect(&mut p, None, &mut |t| trials.push(t.clone())).unwrap();
        assert_eq!(out.accepted, Some(115200));
        assert_eq!(trials[0].bytes, 0);
        assert_eq!(trials[0].score, 0.0);

        let mut p = FakeProbe::new(&[]).at(
            115200,
            Err(AppError::with(ErrorCode::DeviceGone, "unplugged")),
        );
        assert_eq!(
            detect(&mut p, None, &mut |_| {}).unwrap_err().code,
            ErrorCode::DeviceGone
        );
    }

    #[test]
    fn serial_probe_rejects_bad_settings_and_missing_port() {
        let mut s = SerialSettings {
            port: "  ".into(),
            ..Default::default()
        };
        assert_eq!(
            SerialProbe::open(&s).err().unwrap().code,
            ErrorCode::Invalid
        );
        s.port = if cfg!(windows) {
            "COM250".into()
        } else {
            "/dev/cu.kenshell-does-not-exist".into()
        };
        s.data_bits = 9;
        assert_eq!(
            SerialProbe::open(&s).err().unwrap().code,
            ErrorCode::Invalid
        );
        s.data_bits = 8;
        let code = SerialProbe::open(&s).err().unwrap().code;
        assert!(
            matches!(code, ErrorCode::PortNotFound | ErrorCode::Io),
            "{code:?}"
        );
    }
}
