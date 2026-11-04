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
//!
//! Locking: `scan_lock` serializes scans (and listener calls, so changes are delivered
//! in order); `state` is only held for short, non-blocking sections. Lookups
//! (`port_key`, `remember` …) never take `scan_lock` and never notify the listener, so
//! they are safe to call from inside `on_change` or while holding other locks.

// TODO(P1-T06): drop the allow once the session layer (autobaud wiring) is merged.
#[allow(dead_code)]
pub mod autobaud;
pub mod driver_hint;
pub mod hotplug;
pub mod ports;
pub mod usb_ids;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serialport::SerialPortInfo;

use self::driver_hint::UsbDevice;
use self::ports::ScannedPort;
use crate::error::{AppError, Result};
use crate::model::{ChipInfo, ConsoleChange, ConsoleSnapshot, DriverHint, PortMemory};
use crate::storage::Db;

/// A driver hint is only shown once the chip has been on USB without a serial port for
/// this long: right after plug-in the OS has not created the tty/COM node yet.
const HINT_GRACE: Duration = Duration::from_secs(2);
/// Longest remembered device name.
const MAX_DEVICE_NAME: usize = 64;

/// Port-memory persistence (implemented by [`Db`]; faked in tests).
pub(crate) trait PortMemoryStore: Send + Sync {
    fn get_port_memory(&self, key: &str) -> Result<Option<PortMemory>>;
    fn upsert_port_memory(&self, mem: &PortMemory) -> Result<()>;
}

impl PortMemoryStore for Db {
    fn get_port_memory(&self, key: &str) -> Result<Option<PortMemory>> {
        Db::get_port_memory(self, key)
    }
    fn upsert_port_memory(&self, mem: &PortMemory) -> Result<()> {
        Db::upsert_port_memory(self, mem)
    }
}

/// OS device listings (faked in tests).
pub(crate) trait DeviceSource: Send + Sync {
    /// Serial ports; `Err` = enumeration failed (keep the previous snapshot).
    fn serial_ports(&self) -> std::result::Result<Vec<SerialPortInfo>, String>;
    /// USB devices; `None` = listing failed (keep the previous driver hints).
    fn usb_devices(&self) -> Option<Vec<UsbDevice>>;
}

struct SystemSource;

impl DeviceSource for SystemSource {
    fn serial_ports(&self) -> std::result::Result<Vec<SerialPortInfo>, String> {
        ports::enumerate()
    }
    fn usb_devices(&self) -> Option<Vec<UsbDevice>> {
        driver_hint::list_usb_devices()
    }
}

type Listener = Arc<dyn Fn(ConsoleChange) + Send + Sync>;

#[derive(Default)]
struct State {
    snapshot: Option<ConsoleSnapshot>,
    /// USB identity per port key (also for chips not in the table).
    usb: HashMap<String, UsbDevice>,
    /// Per hinted chip (VID:PID): first seen without a port, and whether shown yet.
    hint_first_seen: HashMap<(u16, u16), (Instant, bool)>,
}

thread_local! {
    /// Set while the change listener runs on this thread (re-entrancy guard).
    static IN_LISTENER: Cell<bool> = const { Cell::new(false) };
}

struct ListenerGuard;

impl ListenerGuard {
    fn enter() -> Self {
        IN_LISTENER.with(|f| f.set(true));
        Self
    }
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        IN_LISTENER.with(|f| f.set(false));
    }
}

/// Locks, recovering from poisoning: the guarded data stays consistent because every
/// critical section only assigns whole values.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct ConsoleRegistry {
    store: Arc<dyn PortMemoryStore>,
    source: Arc<dyn DeviceSource>,
    hint_grace: Duration,
    state: Mutex<State>,
    scan_lock: Mutex<()>,
    listener: Mutex<Option<Listener>>,
    watching: AtomicBool,
}

impl ConsoleRegistry {
    pub fn new(db: Arc<Db>) -> Self {
        Self::with_parts(db, Arc::new(SystemSource), HINT_GRACE)
    }

    pub(crate) fn with_parts(
        store: Arc<dyn PortMemoryStore>,
        source: Arc<dyn DeviceSource>,
        hint_grace: Duration,
    ) -> Self {
        Self {
            store,
            source,
            hint_grace,
            state: Mutex::new(State::default()),
            scan_lock: Mutex::new(()),
            listener: Mutex::new(None),
            watching: AtomicBool::new(false),
        }
    }

    /// Current snapshot (scans on first call).
    pub fn snapshot(&self) -> ConsoleSnapshot {
        if let Some(s) = lock(&self.state).snapshot.clone() {
            return s;
        }
        let _scan = lock(&self.scan_lock);
        if let Some(s) = lock(&self.state).snapshot.clone() {
            return s; // another thread scanned while we waited
        }
        // Baseline scan: nothing to compare with, so nobody is notified.
        let _ = self.scan_and_store();
        lock(&self.state).snapshot.clone().unwrap_or_default()
    }

    /// Re-enumerates; returns a change if anything differs from the last snapshot.
    /// The watch listener (if any) is notified of the change as well.
    pub fn rescan(&self) -> Option<ConsoleChange> {
        if IN_LISTENER.with(Cell::get) {
            tracing::warn!("console rescan requested from inside the change listener; ignored");
            return None;
        }
        let _scan = lock(&self.scan_lock);
        let change = self.scan_and_store()?;
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            let _guard = ListenerGuard::enter();
            listener(change.clone());
        }
        Some(change)
    }

    /// Starts the background watcher; `on_change` is called for every change.
    pub fn start_watch(self: &Arc<Self>, on_change: Box<dyn Fn(ConsoleChange) + Send + Sync>) {
        if self.watching.swap(true, Ordering::SeqCst) {
            tracing::warn!("console watch already running");
            return;
        }
        *lock(&self.listener) = Some(Arc::from(on_change));

        // Baseline off the setup path (keeps cold start fast); later scans diff against it.
        let weak = Arc::downgrade(self);
        let baseline = std::thread::Builder::new()
            .name("kenshell-console-baseline".into())
            .spawn(move || {
                if let Some(reg) = weak.upgrade() {
                    let _ = reg.snapshot();
                }
            });
        if let Err(e) = baseline {
            tracing::warn!("console baseline thread: {e}");
        }

        let for_rescan = Arc::downgrade(self);
        let for_signature = Arc::downgrade(self);
        hotplug::start(
            move || {
                let Some(reg) = for_rescan.upgrade() else {
                    return false;
                };
                let run = std::panic::AssertUnwindSafe(|| reg.rescan());
                if std::panic::catch_unwind(run).is_err() {
                    tracing::error!("console rescan panicked; watch continues");
                }
                true
            },
            move || for_signature.upgrade().and_then(|reg| reg.signature()),
        );
    }
}

/// Lookups used by the session layer (identity bar, port memory).
// TODO(P1-T06): drop the allow once the session layer is merged; until then nothing
// in this tree calls these.
#[allow(dead_code)]
impl ConsoleRegistry {
    /// Stable key for a port path (USB serial number if known, else the path).
    pub fn port_key(&self, path: &str) -> String {
        self.find_port(path)
            .map(|p| p.port.key)
            .unwrap_or_else(|| format!("path:{path}"))
    }

    /// Chip of the port at `path`, if it is a known USB console.
    pub fn chip_for(&self, path: &str) -> Option<ChipInfo> {
        self.find_port(path).and_then(|p| p.port.chip)
    }

    /// Short label (`COM4`, `usbserial-A10K3XYZ`).
    pub fn display_name(&self, path: &str) -> String {
        self.find_port(path)
            .map(|p| p.port.display_name)
            .unwrap_or_else(|| ports::display_name_for(path))
    }

    pub fn remembered_baud(&self, path: &str) -> Option<u32> {
        let key = self.port_key(path);
        self.load_memory(&key).map(|m| m.baud).filter(|&b| b > 0)
    }

    /// Persists the working baud (and optional device name) for this cable.
    pub fn remember(&self, path: &str, baud: u32, last_device: Option<&str>) -> Result<()> {
        if baud == 0 {
            return Err(AppError::invalid("baud 0"));
        }
        let found = self.find_port(path);
        let key = found
            .as_ref()
            .map(|p| p.port.key.clone())
            .unwrap_or_else(|| format!("path:{path}"));
        let new_device = last_device
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(MAX_DEVICE_NAME).collect::<String>());
        let last_device = new_device.or_else(|| self.load_memory(&key).and_then(|m| m.last_device));
        let usb = found.as_ref().and_then(|p| p.usb.clone());
        let mem = PortMemory {
            key: key.clone(),
            path: found
                .as_ref()
                .map(|p| p.port.path.clone())
                .unwrap_or_else(|| path.to_owned()),
            vid: usb.as_ref().map(|u| u.vid),
            pid: usb.as_ref().map(|u| u.pid),
            serial_number: usb.and_then(|u| u.serial_number),
            baud,
            last_device: last_device.clone(),
            updated_at: 0,
        };
        self.store.upsert_port_memory(&mem)?;

        let mut st = lock(&self.state);
        if let Some(port) = st
            .snapshot
            .as_mut()
            .and_then(|s| s.ports.iter_mut().find(|p| p.key == key))
        {
            port.remembered_baud = Some(baud);
            port.last_device = last_device;
        }
        Ok(())
    }
}

impl ConsoleRegistry {
    fn load_memory(&self, key: &str) -> Option<PortMemory> {
        self.store.get_port_memory(key).unwrap_or_else(|e| {
            tracing::debug!("port memory lookup failed: {e}");
            None
        })
    }

    /// Port by `path` / `alt_path`: cached snapshot first, then a fresh enumeration
    /// (a cable plugged a moment ago may not be in the snapshot yet). Never touches
    /// `scan_lock` and never notifies.
    fn find_port(&self, path: &str) -> Option<ScannedPort> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let matches = |p: &crate::model::ConsolePort| {
            ports::paths_equal(&p.path, path)
                || p.alt_path
                    .as_deref()
                    .is_some_and(|a| ports::paths_equal(a, path))
        };
        {
            let st = lock(&self.state);
            if let Some(port) = st
                .snapshot
                .as_ref()
                .and_then(|s| s.ports.iter().find(|p| matches(p)))
            {
                return Some(ScannedPort {
                    usb: st.usb.get(&port.key).cloned(),
                    port: port.clone(),
                });
            }
        }
        let infos = self.source.serial_ports().ok()?;
        self.normalize(&infos)
            .into_iter()
            .find(|sp| matches(&sp.port))
    }

    fn normalize(&self, infos: &[SerialPortInfo]) -> Vec<ScannedPort> {
        ports::normalize(infos, &|key| self.load_memory(key))
    }

    /// Scans and stores the new snapshot; returns the diff to the previous one.
    /// Caller holds `scan_lock`.
    fn scan_and_store(&self) -> Option<ConsoleChange> {
        let infos = match self.source.serial_ports() {
            Ok(infos) => infos,
            Err(e) => {
                tracing::warn!("serial port enumeration failed: {e}");
                return None;
            }
        };
        let scanned = self.normalize(&infos);
        let usb_devices = self.source.usb_devices();
        let now = Instant::now();

        let mut st = lock(&self.state);
        let initial = st.snapshot.is_none();
        let driver_hints = match usb_devices {
            Some(devices) => {
                let port_usb: Vec<UsbDevice> =
                    scanned.iter().filter_map(|s| s.usb.clone()).collect();
                let raw = driver_hint::hints(&devices, &port_usb);
                apply_grace(raw, &mut st.hint_first_seen, now, self.hint_grace, initial)
            }
            None => st
                .snapshot
                .as_ref()
                .map(|s| s.driver_hints.clone())
                .unwrap_or_default(),
        };
        st.usb = scanned
            .iter()
            .filter_map(|s| Some((s.port.key.clone(), s.usb.clone()?)))
            .collect();
        let new = ConsoleSnapshot {
            ports: scanned.into_iter().map(|s| s.port).collect(),
            driver_hints,
        };
        let old = st.snapshot.replace(new.clone()).unwrap_or_default();
        diff(&old, new)
    }

    /// Cheap fingerprint of attached devices for the polling fallback.
    fn signature(&self) -> Option<u64> {
        let mut names: Vec<String> = self
            .source
            .serial_ports()
            .ok()?
            .into_iter()
            .map(|p| p.port_name)
            .collect();
        names.sort();
        let mut usb = self.source.usb_devices().unwrap_or_default();
        usb.sort_by(|a, b| (a.vid, a.pid, &a.serial_number).cmp(&(b.vid, b.pid, &b.serial_number)));
        let mut h = DefaultHasher::new();
        names.hash(&mut h);
        usb.hash(&mut h);
        Some(h.finish())
    }
}

/// Change between two snapshots, `None` when equal.
fn diff(old: &ConsoleSnapshot, new: ConsoleSnapshot) -> Option<ConsoleChange> {
    if *old == new {
        return None;
    }
    let old_keys: HashSet<&str> = old.ports.iter().map(|p| p.key.as_str()).collect();
    let new_keys: HashSet<&str> = new.ports.iter().map(|p| p.key.as_str()).collect();
    let attached = new
        .ports
        .iter()
        .filter(|p| !old_keys.contains(p.key.as_str()))
        .map(|p| p.key.clone())
        .collect();
    let detached = old
        .ports
        .iter()
        .filter(|p| !new_keys.contains(p.key.as_str()))
        .map(|p| p.key.clone())
        .collect();
    Some(ConsoleChange {
        snapshot: new,
        attached,
        detached,
    })
}

/// Keeps only hints whose chip has been port-less for at least `grace`; once shown, a
/// hint stays until the chip gets a port or leaves. On the initial scan the device has
/// evidently been attached for a while, so hints show at once.
fn apply_grace(
    raw: Vec<DriverHint>,
    first_seen: &mut HashMap<(u16, u16), (Instant, bool)>,
    now: Instant,
    grace: Duration,
    initial: bool,
) -> Vec<DriverHint> {
    let present: HashSet<(u16, u16)> = raw.iter().map(|h| (h.chip.vid, h.chip.pid)).collect();
    first_seen.retain(|k, _| present.contains(k));
    raw.into_iter()
        .filter(|h| {
            let (since, shown) = first_seen
                .entry((h.chip.vid, h.chip.pid))
                .or_insert((now, false));
            if initial || now.saturating_duration_since(*since) >= grace {
                *shown = true;
            }
            *shown
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use serialport::{SerialPortType, UsbPortInfo};

    #[derive(Default)]
    struct FakeStore {
        rows: Mutex<HashMap<String, PortMemory>>,
        fail: AtomicBool,
    }

    impl PortMemoryStore for FakeStore {
        fn get_port_memory(&self, key: &str) -> Result<Option<PortMemory>> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AppError::with(ErrorCode::Db, "locked"));
            }
            Ok(lock(&self.rows).get(key).cloned())
        }
        fn upsert_port_memory(&self, mem: &PortMemory) -> Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(AppError::with(ErrorCode::Db, "locked"));
            }
            lock(&self.rows).insert(mem.key.clone(), mem.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeSource {
        ports: Mutex<Vec<SerialPortInfo>>,
        usb: Mutex<Option<Vec<UsbDevice>>>,
        fail_ports: AtomicBool,
    }

    impl FakeSource {
        fn set_ports(&self, ports: Vec<SerialPortInfo>) {
            *lock(&self.ports) = ports;
        }
        fn set_usb(&self, usb: Option<Vec<UsbDevice>>) {
            *lock(&self.usb) = usb;
        }
    }

    impl DeviceSource for FakeSource {
        fn serial_ports(&self) -> std::result::Result<Vec<SerialPortInfo>, String> {
            if self.fail_ports.load(Ordering::SeqCst) {
                return Err("enumeration failed".into());
            }
            Ok(lock(&self.ports).clone())
        }
        fn usb_devices(&self) -> Option<Vec<UsbDevice>> {
            lock(&self.usb).clone()
        }
    }

    fn ftdi(name: &str, sn: &str) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.into(),
            port_type: SerialPortType::UsbPort(UsbPortInfo {
                vid: 0x0403,
                pid: 0x6001,
                serial_number: Some(sn.into()),
                manufacturer: Some("FTDI".into()),
                product: Some("FT232R USB UART".into()),
            }),
        }
    }

    fn dev(vid: u16, pid: u16, sn: Option<&str>) -> UsbDevice {
        UsbDevice {
            vid,
            pid,
            serial_number: sn.map(Into::into),
        }
    }

    struct Fixture {
        reg: Arc<ConsoleRegistry>,
        store: Arc<FakeStore>,
        source: Arc<FakeSource>,
    }

    fn fixture(grace: Duration) -> Fixture {
        let store = Arc::new(FakeStore::default());
        let source = Arc::new(FakeSource::default());
        source.set_usb(Some(Vec::new()));
        let reg = Arc::new(ConsoleRegistry::with_parts(
            store.clone(),
            source.clone(),
            grace,
        ));
        Fixture { reg, store, source }
    }

    const KEY: &str = "usb:0403:6001:A10K3XYZ";

    #[test]
    fn snapshot_scans_lazily_and_caches() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![
            ftdi("/dev/tty.usbserial-A10K3XYZ", "A10K3XYZ"),
            ftdi("/dev/cu.usbserial-A10K3XYZ", "A10K3XYZ"),
        ]);
        let s = f.reg.snapshot();
        assert_eq!(s.ports.len(), 1);
        assert_eq!(s.ports[0].key, KEY);
        // Cached: a change in the OS list is not visible until a rescan.
        f.source.set_ports(Vec::new());
        assert_eq!(f.reg.snapshot(), s);
    }

    #[test]
    fn rescan_reports_attach_and_detach_only_on_change() {
        let f = fixture(HINT_GRACE);
        assert_eq!(f.reg.snapshot(), ConsoleSnapshot::default());
        assert_eq!(f.reg.rescan(), None);

        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        let c = f.reg.rescan().expect("attached");
        assert_eq!(c.attached, [KEY]);
        assert!(c.detached.is_empty());
        assert_eq!(c.snapshot, f.reg.snapshot());
        assert_eq!(f.reg.rescan(), None, "unchanged");

        f.source.set_ports(vec![ftdi("COM9", "B")]);
        let c = f.reg.rescan().unwrap();
        assert_eq!(c.attached, ["usb:0403:6001:B"]);
        assert_eq!(c.detached, [KEY]);
    }

    #[test]
    fn enumeration_failure_keeps_previous_snapshot() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        let before = f.reg.snapshot();
        f.source.fail_ports.store(true, Ordering::SeqCst);
        assert_eq!(f.reg.rescan(), None);
        assert_eq!(f.reg.snapshot(), before);
    }

    #[test]
    fn listener_is_notified_and_reentrant_rescan_is_ignored() {
        let f = fixture(HINT_GRACE);
        let _ = f.reg.snapshot();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (s, weak) = (Arc::clone(&seen), Arc::downgrade(&f.reg));
        *lock(&f.reg.listener) = Some(Arc::new(move |c: ConsoleChange| {
            let reg = weak.upgrade().unwrap();
            // Lookups work inside the listener; a nested rescan must not deadlock.
            assert_eq!(reg.port_key("COM4"), KEY);
            assert_eq!(reg.rescan(), None);
            lock(&s).push(c.attached);
        }));
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        assert!(f.reg.rescan().is_some());
        assert_eq!(*lock(&seen), [vec![KEY.to_string()]]);
    }

    #[test]
    fn lookups_by_path_alt_path_and_fallbacks() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![
            ftdi("/dev/tty.usbserial-A10K3XYZ", "A10K3XYZ"),
            ftdi("/dev/cu.usbserial-A10K3XYZ", "A10K3XYZ"),
        ]);
        let reg = &f.reg;
        assert_eq!(reg.port_key("/dev/cu.usbserial-A10K3XYZ"), KEY);
        assert_eq!(reg.port_key("/dev/tty.usbserial-A10K3XYZ"), KEY);
        assert_eq!(
            reg.display_name("/dev/tty.usbserial-A10K3XYZ"),
            "usbserial-A10K3XYZ"
        );
        assert_eq!(
            reg.chip_for("/dev/cu.usbserial-A10K3XYZ").unwrap().model,
            "FT232R"
        );
        assert_eq!(reg.port_key("/dev/cu.gone"), "path:/dev/cu.gone");
        assert_eq!(reg.display_name("/dev/cu.gone"), "gone");
        assert_eq!(reg.display_name("/dev/ttyACM0"), "ttyACM0");
        assert_eq!(reg.chip_for("/dev/cu.gone"), None);
    }

    #[test]
    fn lookup_finds_cable_plugged_after_last_scan_without_swallowing_the_change() {
        let f = fixture(HINT_GRACE);
        let _ = f.reg.snapshot();
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        assert_eq!(f.reg.port_key("com4"), KEY);
        // The hotplug rescan still reports the attach.
        assert_eq!(f.reg.rescan().unwrap().attached, [KEY]);
    }

    #[test]
    fn remember_persists_and_updates_cache() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        let _ = f.reg.snapshot();
        assert_eq!(f.reg.remembered_baud("COM4"), None);

        f.reg
            .remember("COM4", 115200, Some(" core-sw-01 "))
            .unwrap();
        let row = lock(&f.store.rows).get(KEY).cloned().unwrap();
        assert_eq!(
            row,
            PortMemory {
                key: KEY.into(),
                path: "COM4".into(),
                vid: Some(0x0403),
                pid: Some(0x6001),
                serial_number: Some("A10K3XYZ".into()),
                baud: 115200,
                last_device: Some("core-sw-01".into()),
                updated_at: 0,
            }
        );
        assert_eq!(f.reg.remembered_baud("COM4"), Some(115200));
        let p = &f.reg.snapshot().ports[0];
        assert_eq!(p.remembered_baud, Some(115200));
        assert_eq!(p.last_device.as_deref(), Some("core-sw-01"));
        // Cache and DB agree → no spurious change.
        assert_eq!(f.reg.rescan(), None);

        // No device name → previous one is kept.
        f.reg.remember("COM4", 9600, None).unwrap();
        let row = lock(&f.store.rows).get(KEY).cloned().unwrap();
        assert_eq!(row.baud, 9600);
        assert_eq!(row.last_device.as_deref(), Some("core-sw-01"));
    }

    #[test]
    fn remembered_baud_follows_the_cable_to_a_new_port() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        f.reg.remember("COM4", 38400, None).unwrap();
        f.source.set_ports(vec![ftdi("COM7", "A10K3XYZ")]);
        let c = f.reg.rescan().unwrap();
        assert_eq!(c.snapshot.ports[0].remembered_baud, Some(38400));
        assert_eq!(f.reg.remembered_baud("COM7"), Some(38400));
    }

    #[test]
    fn remember_unknown_path_uses_path_key_and_rejects_zero_baud() {
        let f = fixture(HINT_GRACE);
        f.reg.remember("/dev/ttyS0", 9600, None).unwrap();
        let row = lock(&f.store.rows).get("path:/dev/ttyS0").cloned().unwrap();
        assert_eq!((row.vid, row.serial_number), (None, None));
        assert_eq!(
            f.reg.remember("COM4", 0, None).unwrap_err().code,
            ErrorCode::Invalid
        );
    }

    #[test]
    fn db_errors_are_ignored_on_read_and_reported_on_write() {
        let f = fixture(HINT_GRACE);
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        f.store.fail.store(true, Ordering::SeqCst);
        assert_eq!(f.reg.snapshot().ports[0].remembered_baud, None);
        assert_eq!(f.reg.remembered_baud("COM4"), None);
        assert_eq!(
            f.reg.remember("COM4", 9600, None).unwrap_err().code,
            ErrorCode::Db
        );
    }

    #[test]
    fn driver_hints_wait_for_grace_except_on_initial_scan() {
        let f = fixture(Duration::from_millis(300));
        f.source.set_usb(Some(vec![dev(0x067B, 0x2303, None)]));
        // Initial scan: shown immediately.
        assert_eq!(f.reg.snapshot().driver_hints.len(), 1);

        // A second chip appears: hidden until it stays port-less for the grace period.
        f.source.set_usb(Some(vec![
            dev(0x067B, 0x2303, None),
            dev(0x10C4, 0xEA60, None),
        ]));
        // The initially shown hint stays visible meanwhile.
        assert_eq!(f.reg.rescan(), None);
        assert_eq!(f.reg.snapshot().driver_hints.len(), 1);
        std::thread::sleep(Duration::from_millis(320));
        let c = f.reg.rescan().unwrap();
        assert_eq!(c.snapshot.driver_hints.len(), 2);
        assert!(c.attached.is_empty() && c.detached.is_empty());

        // USB listing failure keeps the previous hints.
        f.source.set_usb(None);
        assert_eq!(f.reg.rescan(), None);
    }

    #[test]
    fn port_appearing_in_time_suppresses_hint() {
        let f = fixture(Duration::from_millis(60));
        let _ = f.reg.snapshot();
        f.source
            .set_usb(Some(vec![dev(0x0403, 0x6001, Some("A10K3XYZ"))]));
        assert_eq!(f.reg.rescan(), None); // usb attached, tty not yet: no hint yet
        f.source.set_ports(vec![ftdi("COM4", "A10K3XYZ")]);
        std::thread::sleep(Duration::from_millis(80));
        let c = f.reg.rescan().unwrap();
        assert!(c.snapshot.driver_hints.is_empty());
        assert_eq!(c.attached, [KEY]);
    }

    #[test]
    fn grace_is_pure_and_forgets_resolved_chips() {
        let chip = usb_ids::chip(0x067B, 0x2303).unwrap();
        let raw = vec![DriverHint { chip }];
        let mut seen = HashMap::new();
        let t0 = Instant::now();
        let g = Duration::from_secs(2);
        assert!(apply_grace(raw.clone(), &mut seen, t0, g, false).is_empty());
        assert!(
            apply_grace(
                raw.clone(),
                &mut seen,
                t0 + Duration::from_secs(1),
                g,
                false
            )
            .is_empty()
        );
        assert_eq!(
            apply_grace(raw.clone(), &mut seen, t0 + g, g, false).len(),
            1
        );
        // Once shown it stays (no flicker).
        assert_eq!(
            apply_grace(raw.clone(), &mut seen, t0 + g, g, false).len(),
            1
        );
        // Resolved → forgotten → the grace period starts again.
        assert!(apply_grace(Vec::new(), &mut seen, t0 + g, g, false).is_empty());
        assert!(seen.is_empty());
        assert!(apply_grace(raw, &mut seen, t0 + g * 2, g, false).is_empty());
    }

    /// Manual hardware check: `cargo test -p kenshell live_console -- --ignored --nocapture`
    /// (plug/unplug a console cable during the 10 s window).
    #[test]
    #[ignore = "needs real hardware"]
    fn live_console_snapshot_and_watch() {
        let reg = Arc::new(ConsoleRegistry::with_parts(
            Arc::new(FakeStore::default()),
            Arc::new(SystemSource),
            HINT_GRACE,
        ));
        println!("{:#?}", reg.snapshot());
        reg.start_watch(Box::new(|c| println!("change: {c:#?}")));
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn signature_changes_with_devices() {
        let f = fixture(HINT_GRACE);
        let a = f.reg.signature().unwrap();
        assert_eq!(f.reg.signature().unwrap(), a);
        f.source.set_ports(vec![ftdi("COM4", "X")]);
        let b = f.reg.signature().unwrap();
        assert_ne!(a, b);
        f.source.set_usb(Some(vec![dev(1, 2, None)]));
        assert_ne!(f.reg.signature().unwrap(), b);
        f.source.fail_ports.store(true, Ordering::SeqCst);
        assert_eq!(f.reg.signature(), None);
    }
}
