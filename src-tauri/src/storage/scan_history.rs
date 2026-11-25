//! Network scan history (R-T24): the last [`SCAN_HISTORY_KEEP`] finished scans with their
//! live hosts, so a result can be looked at again after the dialog or the app is closed.
//! The result is stored as JSON (it holds no secrets: addresses, MACs, names, ports).

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::{Db, now_ms};
use crate::error::{AppError, Result};
use crate::netscan::probe::{Method, ScanResult};

/// Scans kept; older ones are removed when a new one is saved.
pub const SCAN_HISTORY_KEEP: usize = 20;
/// Live hosts kept per scan (a /16 can hold far more; the rest is marked truncated).
pub const SCAN_HISTORY_MAX_HOSTS: usize = 5000;

/// One row of the history list (without the hosts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub id: i64,
    pub target: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub cancelled: bool,
    pub method: Method,
    pub alive: i64,
    pub total: i64,
    pub truncated: bool,
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::IcmpSocket => "icmpSocket",
        Method::PingCommand => "pingCommand",
    }
}

impl Db {
    /// Saves a finished scan and keeps only the newest [`SCAN_HISTORY_KEEP`].
    pub fn add_scan(&self, target: &str, result: &ScanResult) -> Result<ScanSummary> {
        let mut stored = result.clone();
        let truncated = stored.hosts.len() > SCAN_HISTORY_MAX_HOSTS;
        stored.hosts.truncate(SCAN_HISTORY_MAX_HOSTS);
        let json = serde_json::to_string(&stored).map_err(|e| AppError::internal(e.to_string()))?;
        let target: String = target.trim().chars().take(64).collect();
        let started_at = now_ms() - i64::try_from(result.duration_ms).unwrap_or(0);
        let summary = ScanSummary {
            id: 0,
            target,
            started_at,
            duration_ms: i64::try_from(result.duration_ms).unwrap_or(i64::MAX),
            cancelled: result.cancelled,
            method: result.method,
            alive: result.hosts.len() as i64,
            total: i64::try_from(result.range.len()).unwrap_or(i64::MAX),
            truncated,
        };
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO scan_history
                 (target, started_at, duration_ms, status, method, alive, total, truncated,
                  result_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                summary.target,
                summary.started_at,
                summary.duration_ms,
                if summary.cancelled {
                    "cancelled"
                } else {
                    "done"
                },
                method_name(summary.method),
                summary.alive,
                summary.total,
                truncated,
                json,
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "DELETE FROM scan_history WHERE id NOT IN
                 (SELECT id FROM scan_history ORDER BY started_at DESC, id DESC LIMIT ?1)",
            [SCAN_HISTORY_KEEP as i64],
        )?;
        tx.commit()?;
        Ok(ScanSummary { id, ..summary })
    }

    /// Newest first.
    pub fn list_scans(&self) -> Result<Vec<ScanSummary>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, target, started_at, duration_ms, status, method, alive, total, truncated
             FROM scan_history ORDER BY started_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let method: String = r.get(5)?;
            Ok(ScanSummary {
                id: r.get(0)?,
                target: r.get(1)?,
                started_at: r.get(2)?,
                duration_ms: r.get(3)?,
                cancelled: r.get::<_, String>(4)? == "cancelled",
                method: if method == "pingCommand" {
                    Method::PingCommand
                } else {
                    Method::IcmpSocket
                },
                alive: r.get(6)?,
                total: r.get(7)?,
                truncated: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The stored result (None when it is gone or unreadable).
    pub fn get_scan(&self, id: i64) -> Result<Option<ScanResult>> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT result_json FROM scan_history WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Idempotent.
    pub fn delete_scan(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM scan_history WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netscan::probe::Host;
    use crate::netscan::scope::Range;
    use std::net::Ipv4Addr;

    fn result(hosts: usize, cancelled: bool) -> ScanResult {
        ScanResult {
            range: Range {
                first: Ipv4Addr::new(10, 0, 0, 1),
                last: Ipv4Addr::new(10, 0, 255, 254),
            },
            hosts: (0..hosts)
                .map(|i| Host {
                    ip: Ipv4Addr::from(0x0a00_0001 + i as u32),
                    rtt_ms: Some(1.5),
                    open_ports: vec![22],
                    mac: None,
                    vendor: None,
                    hostname: Some("sw".into()),
                    own: false,
                    gateway: false,
                })
                .collect(),
            method: Method::PingCommand,
            full_tcp: false,
            local: false,
            cancelled,
            duration_ms: 1234,
        }
    }

    #[test]
    fn saves_lists_reads_and_deletes() {
        let db = Db::open_in_memory().unwrap();
        let s = db.add_scan(" 10.0.0.0/16 ", &result(3, true)).unwrap();
        assert_eq!(s.target, "10.0.0.0/16");
        let list = db.list_scans().unwrap();
        assert_eq!(list.len(), 1);
        let row = &list[0];
        assert!(row.cancelled && !row.truncated);
        assert_eq!(
            (row.alive, row.total, row.method),
            (3, 65_534, Method::PingCommand)
        );
        let back = db.get_scan(row.id).unwrap().unwrap();
        assert_eq!(back, result(3, true));
        db.delete_scan(row.id).unwrap();
        db.delete_scan(row.id).unwrap();
        assert!(db.list_scans().unwrap().is_empty());
        assert!(db.get_scan(row.id).unwrap().is_none());
    }

    #[test]
    fn keeps_the_newest_twenty_and_at_most_5000_hosts() {
        let db = Db::open_in_memory().unwrap();
        for i in 0..25 {
            db.add_scan(&format!("10.0.{i}.0/24"), &result(1, false))
                .unwrap();
        }
        let list = db.list_scans().unwrap();
        assert_eq!(list.len(), SCAN_HISTORY_KEEP);
        assert_eq!(list[0].target, "10.0.24.0/24", "newest first");
        assert!(!list.iter().any(|s| s.target == "10.0.4.0/24"));

        let big = db.add_scan("10.0.0.0/16", &result(5001, false)).unwrap();
        assert!(big.truncated && big.alive == 5001);
        assert_eq!(db.get_scan(big.id).unwrap().unwrap().hosts.len(), 5000);
    }
}
