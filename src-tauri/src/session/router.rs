//! Re-routable session output (P2-T19): lets a live session move to another window.
//!
//! A session task writes to a [`Router`], which forwards to the window that owns the tab.
//! Moving a tab: `park` sends end markers to the old window (an empty data batch and
//! [`SessionEvent::Parked`], so it knows everything before them has arrived) and holds
//! later output; `attach` replays the held output to the new window in order and makes it
//! the live output. Both happen under one lock, so nothing is lost or reordered.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use super::SessionOutput;
use crate::model::SessionEvent;

/// Output held while a tab is between windows is capped; the oldest data goes first
/// (events are small and always kept).
pub(crate) const PARK_CAP: usize = 4 * 1024 * 1024;

enum Held {
    Data(Vec<u8>),
    Event(SessionEvent),
}

enum Route {
    Live(Arc<dyn SessionOutput>),
    Parked {
        held: VecDeque<Held>,
        bytes: usize,
    },
    /// The window is gone: output is dropped.
    Closed,
}

pub(crate) struct Router(Mutex<Route>);

impl Router {
    pub(crate) fn new(out: Arc<dyn SessionOutput>) -> Self {
        Self(Mutex::new(Route::Live(out)))
    }

    fn route(&self) -> MutexGuard<'_, Route> {
        // Every mutation is a single assignment or push, so a poisoned lock is consistent.
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stops delivering to the current output (it gets the end markers) and holds what
    /// follows. A no-op when already parked.
    pub(crate) fn park(&self) {
        let mut route = self.route();
        if let Route::Live(out) = &*route {
            out.data(Vec::new());
            out.event(SessionEvent::Parked);
            *route = Route::Parked {
                held: VecDeque::new(),
                bytes: 0,
            };
        }
    }

    /// Replays held output to `out` and makes it the live output.
    pub(crate) fn attach(&self, out: Arc<dyn SessionOutput>) {
        let mut route = self.route();
        if let Route::Parked { held, .. } = &mut *route {
            for item in held.drain(..) {
                match item {
                    Held::Data(bytes) => out.data(bytes),
                    Held::Event(event) => out.event(event),
                }
            }
        }
        *route = Route::Live(out);
    }

    /// Drops the output for good (its window is closing). The caller's thread drops the
    /// output itself — a Tauri channel holds its webview, and a window whose webview is
    /// still referenced stays on screen as an empty frame, so this must happen on the main
    /// thread before the window is destroyed, not later in the session task.
    pub(crate) fn disconnect(&self) {
        let old = std::mem::replace(&mut *self.route(), Route::Closed);
        drop(old);
    }
}

impl SessionOutput for Router {
    fn data(&self, bytes: Vec<u8>) {
        match &mut *self.route() {
            Route::Live(out) => out.data(bytes),
            Route::Parked { held, bytes: total } => {
                *total += bytes.len();
                held.push_back(Held::Data(bytes));
                while *total > PARK_CAP {
                    let Some(at) = held.iter().position(|h| matches!(h, Held::Data(_))) else {
                        break;
                    };
                    if let Some(Held::Data(old)) = held.remove(at) {
                        *total -= old.len();
                    }
                }
            }
            Route::Closed => {}
        }
    }

    fn event(&self, event: SessionEvent) {
        match &mut *self.route() {
            Route::Live(out) => out.event(event),
            Route::Parked { held, .. } => held.push_back(Held::Event(event)),
            Route::Closed => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConnState;

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl SessionOutput for Rec {
        fn data(&self, bytes: Vec<u8>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("d:{}", String::from_utf8_lossy(&bytes)));
        }
        fn event(&self, event: SessionEvent) {
            let name = match event {
                SessionEvent::Parked => "parked".to_string(),
                SessionEvent::State { state, .. } => format!("{state:?}"),
                _ => "other".to_string(),
            };
            self.0.lock().unwrap().push(format!("e:{name}"));
        }
    }
    impl Rec {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    fn state(s: ConnState) -> SessionEvent {
        SessionEvent::State {
            state: s,
            error: None,
        }
    }

    #[test]
    fn park_marks_the_end_and_attach_replays_in_order() {
        let a = Arc::new(Rec::default());
        let b = Arc::new(Rec::default());
        let r = Router::new(a.clone());
        r.data(b"one".to_vec());
        r.park();
        r.park(); // idempotent: no second pair of markers
        r.data(b"two".to_vec());
        r.event(state(ConnState::Disconnected));
        r.data(b"three".to_vec());
        assert_eq!(a.take(), ["d:one", "d:", "e:parked"]);
        r.attach(b.clone());
        r.data(b"four".to_vec());
        assert!(a.take().is_empty(), "old window gets nothing after park");
        assert_eq!(b.take(), ["d:two", "e:Disconnected", "d:three", "d:four"]);
    }

    #[test]
    fn disconnect_releases_the_output() {
        let a = Arc::new(Rec::default());
        let r = Router::new(a.clone());
        assert_eq!(Arc::strong_count(&a), 2);
        r.disconnect();
        assert_eq!(Arc::strong_count(&a), 1, "window output dropped at once");
        r.data(b"late".to_vec());
        r.event(state(ConnState::Disconnected));
        assert!(a.take().is_empty());
    }

    #[test]
    fn held_output_is_capped_but_events_survive() {
        let a = Arc::new(Rec::default());
        let r = Router::new(Arc::new(Rec::default()));
        r.park();
        r.data(vec![b'x'; PARK_CAP]);
        r.event(state(ConnState::Connected));
        r.data(b"tail".to_vec());
        r.attach(a.clone());
        assert_eq!(a.take(), ["e:Connected", "d:tail"]);
    }
}
