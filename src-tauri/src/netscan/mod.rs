//! Network scanner, "Scan my network" (Phase 4, spec 7.11; R-T24).
//!
//! The scope rules in [`scope`] are mandatory and unit tested: only private (RFC 1918) and
//! link-local ranges, connected or routed, at most a /16 per scan, bounded rate and
//! concurrency, discovery only (ping, ARP table, TCP connect to a few management ports,
//! reverse DNS / NetBIOS name). No vulnerability scanning, brute force or exploitation —
//! ever.

pub mod netbios;
pub mod oui;
pub mod probe;
pub mod scope;
