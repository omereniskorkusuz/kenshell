//! File transfer over a live SSH session (Phase 5): SCP (P5-T01, most network devices
//! offer it) and SFTP browsing. Channels are opened on the session's already
//! authenticated connection — no second login.

pub mod scp;
pub mod sftp;
