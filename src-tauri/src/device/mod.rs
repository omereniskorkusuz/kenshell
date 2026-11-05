//! Device knowledge (Phase 3): interface names, CLI modes, identity.
//!
//! Data tables live in `src/data/*.json` and are shared with the frontend, so both sides
//! normalize and detect the same way.

// Used by interface notes and output colouring (P3-T09/T10); until then only by tests.
#[cfg_attr(not(test), allow(dead_code))]
pub mod iface;
pub mod modes;
pub mod observer;
