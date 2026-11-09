//! Event-driven USB hotplug watch — P1-T10.
//!
//! Two threads, both sleeping in blocking calls (no busy loop):
//! * **source** — `nusb::watch_devices()` (IOKit notifications on macOS, CM notifications
//!   on Windows); every connect/disconnect event becomes a trigger. If the watch cannot
//!   be created, it degrades to polling a cheap signature every [`POLL_INTERVAL`] and
//!   triggers only when the signature changed.
//! * **scheduler** — turns triggers into rescans: debounced first scan, then re-checks
//!   because the OS creates the tty/COM node some time after the USB attach (the cable
//!   must show up within 2 s), plus a late re-check so the driver-hint grace period can
//!   elapse.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// When to rescan after a trigger.
#[derive(Debug, Clone)]
pub struct Schedule {
    /// Quiet time before the first rescan; every new trigger restarts it…
    pub debounce: Duration,
    /// …but a continuous event storm cannot postpone the first rescan beyond this.
    pub max_delay: Duration,
    /// Additional rescans, as offsets from the latest trigger.
    pub rechecks: Vec<Duration>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(250),
            max_delay: Duration::from_millis(1000),
            rechecks: vec![
                Duration::from_millis(700),
                Duration::from_millis(1500),
                Duration::from_millis(3000),
            ],
        }
    }
}

pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Starts watching. `rescan` runs on the scheduler thread and returns `false` to stop
/// watching (e.g. its owner was dropped). `signature` is only used by the polling
/// fallback: a cheap fingerprint of the attached devices (`None` = could not tell).
pub fn start<R, S>(rescan: R, signature: S)
where
    R: FnMut() -> bool + Send + 'static,
    S: FnMut() -> Option<u64> + Send + 'static,
{
    let trigger = match spawn_scheduler(Schedule::default(), rescan) {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("console hotplug scheduler thread: {e}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("kenshell-usb-watch".into())
        .spawn(move || watch_or_poll(trigger, signature));
    if let Err(e) = spawned {
        tracing::error!("console hotplug watch thread: {e}");
    }
}

fn watch_or_poll<S: FnMut() -> Option<u64>>(trigger: Sender<()>, signature: S) {
    use futures_lite::StreamExt;
    match nusb::watch_devices() {
        Ok(mut watch) => {
            tracing::debug!("usb hotplug watch active");
            loop {
                match futures_lite::future::block_on(watch.next()) {
                    Some(event) => {
                        tracing::debug!("usb hotplug: {event:?}");
                        if trigger.send(()).is_err() {
                            return; // scheduler stopped
                        }
                    }
                    None => {
                        tracing::warn!("usb hotplug stream ended; falling back to polling");
                        break;
                    }
                }
            }
        }
        Err(e) => tracing::warn!("usb hotplug watch unavailable ({e}); polling every 2 s"),
    }
    poll(trigger, signature, POLL_INTERVAL);
}

/// Polling fallback: compares fingerprints, triggers only on change.
fn poll<S: FnMut() -> Option<u64>>(trigger: Sender<()>, mut signature: S, every: Duration) {
    let mut last = signature();
    loop {
        std::thread::sleep(every);
        let now = signature();
        if now.is_some() && now != last {
            last = now;
            if trigger.send(()).is_err() {
                return;
            }
        }
    }
}

/// Spawns the scheduler thread; triggers are sent through the returned channel. The
/// thread ends when `rescan` returns `false`, or when every sender is gone and no
/// rescans are pending.
pub(crate) fn spawn_scheduler<R>(schedule: Schedule, rescan: R) -> std::io::Result<Sender<()>>
where
    R: FnMut() -> bool + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kenshell-console-rescan".into())
        .spawn(move || run_scheduler(&schedule, &rx, rescan))?;
    Ok(tx)
}

fn run_scheduler<R: FnMut() -> bool>(schedule: &Schedule, rx: &Receiver<()>, mut rescan: R) {
    let mut pending: Vec<Instant> = Vec::new(); // ascending
    let mut burst_start: Option<Instant> = None;
    let mut senders_gone = false;
    loop {
        let event = match pending.first() {
            None if senders_gone => return,
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(&next) if senders_gone => {
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                Err(RecvTimeoutError::Timeout)
            }
            Some(&next) => rx.recv_timeout(next.saturating_duration_since(Instant::now())),
        };
        match event {
            Ok(()) => {
                let now = Instant::now();
                let start = *burst_start.get_or_insert(now);
                let first = (now + schedule.debounce).min(start + schedule.max_delay);
                pending.clear();
                pending.push(first);
                pending.extend(
                    schedule
                        .rechecks
                        .iter()
                        .map(|d| now + *d)
                        .filter(|t| *t > first),
                );
                pending.sort();
            }
            Err(RecvTimeoutError::Disconnected) => senders_gone = true,
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if pending.first().is_some_and(|t| *t <= now) {
                    // Coalesce every deadline that is already due into one rescan.
                    pending.retain(|t| *t > now);
                    burst_start = None;
                    if !rescan() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn test_schedule() -> Schedule {
        Schedule {
            debounce: ms(80),
            max_delay: ms(300),
            rechecks: vec![ms(200), ms(350)],
        }
    }

    fn counting() -> (Arc<AtomicUsize>, impl FnMut() -> bool + Send + 'static) {
        let n = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&n);
        (n, move || {
            c.fetch_add(1, Ordering::SeqCst);
            true
        })
    }

    #[test]
    fn burst_is_debounced_then_rechecked() {
        let (n, rescan) = counting();
        let tx = spawn_scheduler(test_schedule(), rescan).unwrap();
        // A burst well inside the debounce window (no sleeps: CI timers are coarse).
        for _ in 0..5 {
            tx.send(()).unwrap();
        }
        assert_eq!(n.load(Ordering::SeqCst), 0, "still debouncing");
        // first scan + 2 rechecks, not one per event — wait generously on slow runners
        let start = Instant::now();
        while n.load(Ordering::SeqCst) < 3 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(ms(20));
        }
        std::thread::sleep(ms(400));
        assert_eq!(n.load(Ordering::SeqCst), 3);
        drop(tx);
    }

    #[test]
    fn event_storm_cannot_postpone_first_scan_forever() {
        let (n, rescan) = counting();
        let tx = spawn_scheduler(test_schedule(), rescan).unwrap();
        let start = Instant::now();
        while start.elapsed() < ms(500) {
            tx.send(()).unwrap();
            std::thread::sleep(ms(10));
        }
        assert!(n.load(Ordering::SeqCst) >= 1, "max_delay must force a scan");
    }

    #[test]
    fn idle_scheduler_does_nothing_and_exits_when_senders_drop() {
        let n = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(Mutex::new(false));
        let (c, d) = (Arc::clone(&n), Arc::clone(&done));
        let (tx, rx) = mpsc::channel();
        let h = std::thread::spawn(move || {
            run_scheduler(&test_schedule(), &rx, || {
                c.fetch_add(1, Ordering::SeqCst);
                true
            });
            *d.lock().unwrap() = true;
        });
        std::thread::sleep(ms(100));
        assert_eq!(n.load(Ordering::SeqCst), 0);
        tx.send(()).unwrap();
        drop(tx);
        h.join().unwrap();
        // Pending rescans still ran before the thread exited: 3 deadlines (80/200/350 ms),
        // or 2 on a slow machine where a late wake-up finds two already due and coalesces
        // them into one rescan (intended).
        let runs = n.load(Ordering::SeqCst);
        assert!((2..=3).contains(&runs), "rescans after disconnect: {runs}");
        assert!(*done.lock().unwrap());
    }

    #[test]
    fn rescan_returning_false_stops_the_thread() {
        let (tx, rx) = mpsc::channel();
        let h = std::thread::spawn(move || run_scheduler(&test_schedule(), &rx, || false));
        tx.send(()).unwrap();
        h.join().unwrap();
        assert!(tx.send(()).is_err());
    }

    #[test]
    fn polling_triggers_only_on_change() {
        use std::sync::atomic::AtomicBool;
        let sigs = Arc::new(Mutex::new(vec![
            Some(1),
            Some(1),
            None,
            Some(1),
            Some(2),
            Some(2),
            Some(2),
        ]));
        let finish = Arc::new(AtomicBool::new(false));
        let (s, f) = (Arc::clone(&sigs), Arc::clone(&finish));
        let mut next = 100;
        let (tx, rx) = mpsc::channel();
        let h = std::thread::spawn(move || {
            poll(
                tx,
                move || {
                    if f.load(Ordering::SeqCst) {
                        next += 1; // always "changed" → send fails → thread exits
                        return Some(next);
                    }
                    let mut v = s.lock().unwrap();
                    if v.is_empty() { Some(2) } else { v.remove(0) }
                },
                ms(5),
            )
        });
        rx.recv_timeout(ms(500)).expect("change 1 -> 2 triggers");
        assert!(
            rx.recv_timeout(ms(100)).is_err(),
            "no trigger without change"
        );
        drop(rx);
        finish.store(true, Ordering::SeqCst);
        h.join().unwrap();
    }
}
