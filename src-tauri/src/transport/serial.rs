//! Serial transport (serialport crate) — P1-T08.
//!
//! * A dedicated reader **thread** does blocking reads with a long timeout
//!   ([`IO_TIMEOUT`]), so an idle console costs ~0 CPU (no busy loop). After every
//!   successful read it drains whatever the driver already buffered (up to
//!   [`MAX_BATCH`]) and pushes one [`TransportEvent::Data`] into the sink.
//! * Writes and BREAK run on a cloned handle inside `spawn_blocking`, never on the
//!   async executor.
//! * DTR is applied at open time (no reset pulse on devices that reset on DTR);
//!   RTS right after open.
//! * Read failures are classified: a vanished USB adapter becomes
//!   [`ErrorCode::DeviceGone`] (the session layer turns that into "cable removed").

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serialport::SerialPort;
use tokio::sync::mpsc::error::TrySendError;

use super::{Transport, TransportEvent, TransportSink};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{FlowControl, Parity, SerialSettings};

/// Read (and per-call write) timeout. Bounds how long `disconnect` waits for the
/// reader thread to notice the stop flag. Long enough that an idle port wakes up
/// only a few times per second.
pub const IO_TIMEOUT: Duration = Duration::from_millis(300);

/// Upper bound for one `Data` event.
pub const MAX_BATCH: usize = 64 * 1024;

/// How long the line is held in the BREAK condition (Cisco ROMMON needs ≥ 250 ms).
pub const BREAK_DURATION: Duration = Duration::from_millis(300);

/// A single write may stall this long in total (hardware/software flow control,
/// slow baud) before it is reported as failed.
pub const WRITE_DEADLINE: Duration = Duration::from_secs(5);

/// Poll interval while the sink is full (UI backpressure). Only used in that case.
const BACKPRESSURE_WAIT: Duration = Duration::from_millis(5);

type SharedPort = Arc<Mutex<Option<Box<dyn SerialPort>>>>;

pub struct SerialTransport {
    settings: SerialSettings,
    baud: u32,
    sink: TransportSink,
    /// Stop flag of the current reader thread (fresh per connect).
    stop: Arc<AtomicBool>,
    /// Write handle (a `try_clone` of the reader's port).
    writer: SharedPort,
    reader: Option<JoinHandle<()>>,
}

impl SerialTransport {
    /// `baud` is the resolved rate (autobaud already done by the session layer).
    pub fn new(settings: SerialSettings, baud: u32, sink: TransportSink) -> Self {
        Self {
            settings,
            baud,
            sink,
            stop: Arc::new(AtomicBool::new(false)),
            writer: Arc::new(Mutex::new(None)),
            reader: None,
        }
    }

    fn builder(&self) -> Result<serialport::SerialPortBuilder> {
        let s = &self.settings;
        if s.port.trim().is_empty() {
            return Err(AppError::invalid("no serial port selected"));
        }
        if self.baud == 0 {
            return Err(AppError::invalid("baud rate must be > 0"));
        }
        let data_bits = match s.data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            8 => serialport::DataBits::Eight,
            n => return Err(AppError::invalid(format!("unsupported data bits: {n}"))),
        };
        let stop_bits = match s.stop_bits {
            1 => serialport::StopBits::One,
            2 => serialport::StopBits::Two,
            n => return Err(AppError::invalid(format!("unsupported stop bits: {n}"))),
        };
        let parity = match s.parity {
            Parity::None => serialport::Parity::None,
            Parity::Odd => serialport::Parity::Odd,
            Parity::Even => serialport::Parity::Even,
        };
        let flow = match s.flow_control {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::RtsCts => serialport::FlowControl::Hardware,
            FlowControl::XonXoff => serialport::FlowControl::Software,
        };
        let mut b = serialport::new(s.port.clone(), self.baud)
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .parity(parity)
            .flow_control(flow)
            .timeout(IO_TIMEOUT);
        if let Some(dtr) = s.dtr {
            b = b.dtr_on_open(dtr);
        }
        Ok(b)
    }

    /// Starts the reader thread on an already opened port. Used by `connect` and by
    /// tests (pseudo-terminal pairs).
    fn start(&mut self, mut port: Box<dyn SerialPort>) -> Result<()> {
        let path = port.name().unwrap_or_else(|| self.settings.port.clone());
        port.set_timeout(IO_TIMEOUT)
            .map_err(|e| AppError::with(ErrorCode::Io, format!("{path}: {}", e.description)))?;
        let writer = port
            .try_clone()
            .map_err(|e| AppError::with(ErrorCode::Io, format!("{path}: {}", e.description)))?;

        let stop = Arc::new(AtomicBool::new(false));
        *lock(&self.writer)? = Some(writer);
        let reader = {
            let stop = Arc::clone(&stop);
            let sink = self.sink.clone();
            std::thread::Builder::new()
                .name("serial-reader".into())
                .spawn(move || reader_loop(port, path, stop, sink))
                .map_err(|e| AppError::internal(format!("spawn reader: {e}")))?
        };
        self.stop = stop;
        self.reader = Some(reader);
        Ok(())
    }

    fn port_label(&self) -> String {
        self.settings.port.clone()
    }
}

impl Drop for SerialTransport {
    fn drop(&mut self) {
        // The reader thread owns its handle and exits within IO_TIMEOUT.
        self.stop.store(true, Ordering::Release);
    }
}

#[async_trait]
impl Transport for SerialTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.reader.is_some() {
            return Err(AppError::invalid("already connected"));
        }
        let builder = self.builder()?;
        let path = self.settings.port.clone();
        let rts = self.settings.rts;
        let port = blocking(move || {
            let mut port = builder
                .open()
                .map_err(|e| crate::platform::classify_open_error(&path, &e))?;
            if let Some(level) = rts
                && let Err(e) = port.write_request_to_send(level)
            {
                // Some adapters cannot drive RTS; the console still works without it.
                tracing::warn!("{path}: cannot set RTS: {}", e.description);
            }
            Ok(port)
        })
        .await?;
        self.start(port)?;
        tracing::debug!(port = %self.settings.port, baud = self.baud, "serial port open");
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let writer = Arc::clone(&self.writer);
        let data = data.to_vec();
        let path = self.port_label();
        blocking(move || {
            let mut guard = lock(&writer)?;
            let port = guard
                .as_mut()
                .ok_or_else(|| AppError::invalid("not connected"))?;
            write_fully(port.as_mut(), &data, WRITE_DEADLINE)
                .and_then(|()| port.flush())
                .map_err(|e| classify_write_error(&path, &e))
        })
        .await
    }

    async fn resize(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        Ok(()) // serial lines have no window size
    }

    async fn send_break(&mut self) -> Result<()> {
        let writer = Arc::clone(&self.writer);
        let path = self.port_label();
        blocking(move || {
            // Holding the lock keeps writes from interleaving with the BREAK.
            let guard = lock(&writer)?;
            let port = guard
                .as_ref()
                .ok_or_else(|| AppError::invalid("not connected"))?;
            let map = |e: serialport::Error| {
                AppError::with(ErrorCode::Io, format!("{path}: break: {}", e.description))
            };
            port.set_break().map_err(map)?;
            std::thread::sleep(BREAK_DURATION);
            port.clear_break().map_err(map)
        })
        .await
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        let writer = Arc::clone(&self.writer);
        let reader = self.reader.take();
        blocking(move || {
            // Drop the write handle (waits for an in-flight write to finish).
            if let Ok(mut w) = writer.lock() {
                w.take();
            }
            // The reader notices the stop flag within IO_TIMEOUT and drops its handle,
            // which releases the exclusive lock so the port can be reopened at once.
            if let Some(h) = reader
                && h.join().is_err()
            {
                tracing::warn!("serial reader thread panicked");
            }
            Ok(())
        })
        .await
    }
}

/// Runs a blocking closure on tokio's blocking pool.
async fn blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(format!("blocking task: {e}")))?
}

fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| AppError::internal("serial handle lock poisoned"))
}

// ---------------------------------------------------------------- reader thread

fn reader_loop(
    mut port: Box<dyn SerialPort>,
    path: String,
    stop: Arc<AtomicBool>,
    sink: TransportSink,
) {
    let mut scratch = vec![0u8; MAX_BATCH];
    let closed = loop {
        if stop.load(Ordering::Acquire) {
            break None;
        }
        match port.read(&mut scratch) {
            // A tty read only returns 0 after poll() said "readable": hang-up.
            Ok(0) => {
                let e = io::Error::new(io::ErrorKind::UnexpectedEof, "end of stream");
                break Some(e);
            }
            Ok(mut n) => {
                n += drain_available(port.as_mut(), &mut scratch[n..]);
                if !deliver(&sink, &stop, TransportEvent::Data(scratch[..n].to_vec())) {
                    // Receiver gone or stopping while the UI is backlogged.
                    let _ = sink.try_send(TransportEvent::Closed(None));
                    return;
                }
            }
            Err(e) if is_transient(&e) => continue,
            Err(e) => break Some(e),
        }
    };
    drop(scratch);
    let event = match closed {
        // Errors caused by our own shutdown are a clean close.
        _ if stop.load(Ordering::Acquire) => TransportEvent::Closed(None),
        None => TransportEvent::Closed(None),
        Some(e) => {
            let err = classify_read_error(&path, &e, crate::platform::port_exists(&path));
            tracing::debug!(port = %path, code = ?err.code, "serial reader ended: {e}");
            TransportEvent::Closed(Some(err))
        }
    };
    // Close the port before reporting, so a reconnect can reopen it immediately.
    drop(port);
    match event {
        TransportEvent::Closed(None) => {
            let _ = sink.try_send(event);
        }
        ev => {
            deliver(&sink, &stop, ev);
        }
    }
}

/// Reads what the driver already has buffered into `buf` (non-blocking in practice:
/// only called while `bytes_to_read() > 0`). Errors are left for the next main read.
fn drain_available(port: &mut dyn SerialPort, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        let avail = match port.bytes_to_read() {
            Ok(k) if k > 0 => k as usize,
            _ => break,
        };
        let want = avail.min(buf.len() - n);
        match port.read(&mut buf[n..n + want]) {
            Ok(0) | Err(_) => break,
            Ok(m) => n += m,
        }
    }
    n
}

/// Sends into the bounded sink. Under UI backpressure it waits (in short steps, so a
/// `disconnect` is noticed) instead of `blocking_send`, which could deadlock against
/// a session loop that is itself waiting for this thread to exit.
fn deliver(sink: &TransportSink, stop: &AtomicBool, mut ev: TransportEvent) -> bool {
    loop {
        match sink.try_send(ev) {
            Ok(()) => return true,
            Err(TrySendError::Closed(_)) => return false,
            Err(TrySendError::Full(back)) => {
                if stop.load(Ordering::Acquire) {
                    return false;
                }
                ev = back;
                std::thread::sleep(BACKPRESSURE_WAIT);
            }
        }
    }
}

fn is_transient(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

/// Writes all of `data`, tolerating per-call timeouts (flow control) until `deadline`.
fn write_fully(port: &mut dyn SerialPort, mut data: &[u8], deadline: Duration) -> io::Result<()> {
    let start = Instant::now();
    while !data.is_empty() {
        match port.write(data) {
            Ok(n) if n > 0 => data = &data[n..],
            Ok(_) => {
                // Windows reports a write timeout as 0 bytes written.
                if start.elapsed() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "write timed out"));
                }
            }
            Err(e) if is_transient(&e) => {
                if start.elapsed() >= deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "write timed out"));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- error classification

/// Windows error codes seen when a USB serial adapter is unplugged under an open handle.
#[cfg(windows)]
const GONE_OS_ERRORS: &[i32] = &[
    2,    // ERROR_FILE_NOT_FOUND
    5,    // ERROR_ACCESS_DENIED (after open = device removed)
    22,   // ERROR_BAD_COMMAND
    31,   // ERROR_GEN_FAILURE
    433,  // ERROR_NO_SUCH_DEVICE
    995,  // ERROR_OPERATION_ABORTED
    1167, // ERROR_DEVICE_NOT_CONNECTED
];

/// errno values for a vanished tty (EIO, ENXIO, ENODEV).
#[cfg(not(windows))]
const GONE_OS_ERRORS: &[i32] = &[5, 6, 19];

/// serialport drops the raw errno on unix and keeps only the message; match those.
const GONE_MESSAGES: &[&str] = &[
    "i/o error",
    "input/output error",
    "no such device",
    "device not configured",
    "device not connected",
    "device does not recognize the command",
];

/// Maps a failed read (after a successful open) to an [`AppError`].
/// `port_exists` = whether the port path is still present.
pub(crate) fn classify_read_error(path: &str, err: &io::Error, port_exists: bool) -> AppError {
    use io::ErrorKind as K;
    let msg = err.to_string().to_ascii_lowercase();
    let gone = !port_exists
        || matches!(
            err.kind(),
            K::BrokenPipe
                | K::NotFound
                | K::NotConnected
                | K::ConnectionAborted
                | K::ConnectionReset
                | K::UnexpectedEof
        )
        || err
            .raw_os_error()
            .is_some_and(|c| GONE_OS_ERRORS.contains(&c))
        || GONE_MESSAGES.iter().any(|m| msg.contains(m));
    let code = if gone {
        ErrorCode::DeviceGone
    } else {
        ErrorCode::Io
    };
    AppError::with(code, format!("{path}: {err}"))
}

fn classify_write_error(path: &str, err: &io::Error) -> AppError {
    match err.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WriteZero => {
            AppError::with(ErrorCode::Io, format!("{path}: write timed out"))
        }
        _ => classify_read_error(path, err, crate::platform::port_exists(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_device_gone_vs_io() {
        let gone = |e: io::Error, exists| classify_read_error("/dev/x", &e, exists).code;
        use io::ErrorKind as K;
        assert_eq!(gone(io::Error::new(K::BrokenPipe, "x"), true), ErrorCode::DeviceGone);
        assert_eq!(gone(io::Error::new(K::Other, "x"), false), ErrorCode::DeviceGone);
        assert_eq!(
            gone(io::Error::new(K::Other, "Device not configured"), true),
            ErrorCode::DeviceGone
        );
        assert_eq!(gone(io::Error::new(K::Other, "I/O error"), true), ErrorCode::DeviceGone);
        assert_eq!(gone(io::Error::from_raw_os_error(GONE_OS_ERRORS[0]), true), ErrorCode::DeviceGone);
        assert_eq!(gone(io::Error::new(K::Other, "framing"), true), ErrorCode::Io);
        let e = classify_read_error("/dev/x", &io::Error::new(K::Other, "boom"), true);
        assert_eq!(e.detail.as_deref(), Some("/dev/x: boom"));
    }

    #[test]
    fn write_errors() {
        let e = classify_write_error("p", &io::Error::new(io::ErrorKind::WriteZero, "z"));
        assert_eq!(e.code, ErrorCode::Io);
        assert_eq!(e.detail.as_deref(), Some("p: write timed out"));
    }

    fn transport(settings: SerialSettings, baud: u32) -> (SerialTransport, tokio::sync::mpsc::Receiver<TransportEvent>) {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        (SerialTransport::new(settings, baud, tx), rx)
    }

    #[tokio::test]
    async fn invalid_settings_are_rejected_before_open() {
        let base = SerialSettings {
            port: "/dev/null-kenshell".into(),
            ..SerialSettings::default()
        };
        for s in [
            SerialSettings { data_bits: 9, ..base.clone() },
            SerialSettings { stop_bits: 3, ..base.clone() },
            SerialSettings { port: " ".into(), ..base.clone() },
        ] {
            let (mut t, _rx) = transport(s, 9600);
            assert_eq!(t.connect().await.unwrap_err().code, ErrorCode::Invalid);
        }
        let (mut t, _rx) = transport(base, 0);
        assert_eq!(t.connect().await.unwrap_err().code, ErrorCode::Invalid);
    }

    #[tokio::test]
    async fn write_and_break_without_connection_are_invalid() {
        let (mut t, _rx) = transport(SerialSettings::default(), 9600);
        assert_eq!(t.write(b"x").await.unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(t.send_break().await.unwrap_err().code, ErrorCode::Invalid);
        // disconnect is idempotent, even when never connected
        t.disconnect().await.unwrap();
        t.disconnect().await.unwrap();
    }

    #[cfg(unix)]
    mod pty {
        use super::*;
        use serialport::TTYPort;
        use tokio::sync::mpsc::Receiver;

        const WAIT: Duration = Duration::from_secs(5);

        fn started() -> (SerialTransport, Receiver<TransportEvent>, TTYPort) {
            let (master, slave) = TTYPort::pair().expect("pty pair");
            let (mut t, rx) = transport(
                SerialSettings {
                    port: slave.name().unwrap_or_default(),
                    ..SerialSettings::default()
                },
                9600,
            );
            t.start(Box::new(slave)).expect("start");
            (t, rx, master)
        }

        async fn next(rx: &mut Receiver<TransportEvent>) -> TransportEvent {
            tokio::time::timeout(WAIT, rx.recv())
                .await
                .expect("event in time")
                .expect("channel open")
        }

        #[tokio::test]
        async fn opening_missing_port_is_port_not_found() {
            let (mut t, _rx) = transport(
                SerialSettings {
                    port: "/dev/cu.kenshell-does-not-exist".into(),
                    baud: Some(9600),
                    ..SerialSettings::default()
                },
                9600,
            );
            let e = t.connect().await.unwrap_err();
            assert_eq!(e.code, ErrorCode::PortNotFound);
            assert!(e.detail.unwrap().contains("kenshell-does-not-exist"));
        }

        #[tokio::test]
        async fn reads_are_batched_and_complete() {
            let (mut t, mut rx, mut master) = started();
            let payload: Vec<u8> = (0..20_000u32).map(|i| b'a' + (i % 26) as u8).collect();
            let chunks = payload.len() / 100;
            let to_send = payload.clone();
            let writer = std::thread::spawn(move || {
                for c in to_send.chunks(100) {
                    master.write_all(c).unwrap();
                }
                master
            });
            let mut got = Vec::new();
            let mut events = 0;
            while got.len() < payload.len() {
                match next(&mut rx).await {
                    TransportEvent::Data(d) => {
                        assert!(!d.is_empty() && d.len() <= MAX_BATCH);
                        got.extend_from_slice(&d);
                        events += 1;
                    }
                    TransportEvent::Closed(e) => panic!("closed early: {e:?}"),
                }
            }
            assert_eq!(got, payload);
            assert!(events < chunks, "expected coalescing: {events} events for {chunks} writes");
            let _master = writer.join().unwrap();
            t.disconnect().await.unwrap();
        }

        #[tokio::test]
        async fn write_reaches_the_device() {
            let (mut t, _rx, mut master) = started();
            t.write(b"show version\r").await.unwrap();
            t.write(b"").await.unwrap();
            let got = tokio::task::spawn_blocking(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 64];
                let start = Instant::now();
                while buf.len() < 13 && start.elapsed() < WAIT {
                    match master.read(&mut chunk) {
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e) if is_transient(&e) => {}
                        Err(e) => panic!("{e}"),
                    }
                }
                buf
            })
            .await
            .unwrap();
            assert_eq!(got, b"show version\r");
            t.disconnect().await.unwrap();
        }

        #[tokio::test]
        async fn disconnect_closes_cleanly_and_is_idempotent() {
            let (mut t, mut rx, _master) = started();
            let start = Instant::now();
            t.disconnect().await.unwrap();
            assert!(start.elapsed() < IO_TIMEOUT * 3, "reader must stop within its timeout");
            assert!(matches!(next(&mut rx).await, TransportEvent::Closed(None)));
            t.disconnect().await.unwrap();
            assert_eq!(t.write(b"x").await.unwrap_err().code, ErrorCode::Invalid);
        }

        #[tokio::test]
        async fn device_hangup_reports_device_gone() {
            let (mut t, mut rx, master) = started();
            drop(master);
            match next(&mut rx).await {
                TransportEvent::Closed(Some(e)) => assert_eq!(e.code, ErrorCode::DeviceGone),
                other => panic!("unexpected {other:?}"),
            }
            t.disconnect().await.unwrap();
        }

        #[tokio::test]
        async fn dropping_receiver_stops_the_reader() {
            let (t, rx, mut master) = started();
            drop(rx);
            master.write_all(b"data nobody reads").unwrap();
            // The reader exits on the failed send; dropping the transport must not hang.
            drop(t);
        }

        #[tokio::test]
        async fn connect_twice_is_rejected() {
            let (mut t, _rx, _master) = started();
            assert_eq!(t.connect().await.unwrap_err().code, ErrorCode::Invalid);
            t.disconnect().await.unwrap();
        }
    }
}
