//! Tab handoff between windows (P2-T19).
//!
//! The old window parks the session, then hands its tab (state + scrollback as an opaque
//! JSON payload) to a new window. The payload waits here, keyed by the new window's label,
//! until that window takes it at startup. It lives only in memory and is never logged.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;

use crate::error::{AppError, Result};
use crate::model::RuntimeId;

/// Upper bound for one payload (scrollback text + not yet rendered output).
pub const MAX_PAYLOAD: usize = 24 * 1024 * 1024;
/// Label prefix of detached session windows (also listed in the capability file).
pub const WINDOW_PREFIX: &str = "session-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handoff {
    pub runtime_id: RuntimeId,
    pub payload: String,
}

#[derive(Default)]
pub struct Handoffs {
    next: AtomicU32,
    pending: Mutex<HashMap<String, Handoff>>,
}

impl Handoffs {
    /// A fresh, never reused window label.
    pub fn next_label(&self) -> String {
        format!(
            "{WINDOW_PREFIX}{}",
            self.next.fetch_add(1, Ordering::Relaxed) + 1
        )
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Handoff>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn put(&self, label: &str, handoff: Handoff) -> Result<()> {
        if handoff.payload.len() > MAX_PAYLOAD {
            return Err(AppError::invalid("tab is too large to move"));
        }
        self.map().insert(label.to_owned(), handoff);
        Ok(())
    }

    /// Takes the handoff of window `label` (once).
    pub fn take(&self, label: &str) -> Option<Handoff> {
        self.map().remove(label)
    }
}

/// Window title for a moved tab: the tab name without control characters, shortened.
pub fn window_title(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "KenShell".to_owned()
    } else {
        format!("{clean} — KenShell")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_unique_and_handoffs_are_taken_once() {
        let h = Handoffs::default();
        let (a, b) = (h.next_label(), h.next_label());
        assert_eq!(a, "session-1");
        assert_ne!(a, b);
        let item = Handoff {
            runtime_id: 7,
            payload: "{}".into(),
        };
        h.put(&a, item.clone()).unwrap();
        assert_eq!(h.take(&b), None);
        assert_eq!(h.take(&a), Some(item));
        assert_eq!(h.take(&a), None);
    }

    #[test]
    fn oversized_payloads_and_odd_titles() {
        let h = Handoffs::default();
        let big = Handoff {
            runtime_id: 1,
            payload: "x".repeat(MAX_PAYLOAD + 1),
        };
        assert!(h.put("session-1", big).is_err());
        assert_eq!(window_title("core-sw\u{1b}[31m"), "core-sw[31m — KenShell");
        assert_eq!(window_title(" \n "), "KenShell");
        assert_eq!(window_title(&"a".repeat(200)).chars().count(), 80 + 11);
    }
}
