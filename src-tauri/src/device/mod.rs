//! Device knowledge (Phase 3): interface names, CLI modes, identity.
//!
//! Data tables live in `src/data/*.json` and are shared with the frontend, so both sides
//! normalize and detect the same way.

pub mod capture;
pub mod iface;
pub mod lines;
pub mod modes;
pub mod observer;
pub mod probe;
