//! Local network scanner, "Scan my network" (Phase 4, spec 7.11).
//!
//! The scope rules in [`scope`] are mandatory and unit tested: only subnets directly
//! connected to this computer, only private (RFC 1918) and link-local ranges, at most a
//! /22 per scan, bounded concurrency, discovery only (ping, ARP table, TCP connect to a
//! few management ports). No vulnerability scanning, brute force or exploitation — ever.

pub mod oui;
pub mod probe;
pub mod scope;
