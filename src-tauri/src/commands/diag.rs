//! Ping / traceroute commands (P5-T05). Logic lives in `diag` and `platform::diag`.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::Deserialize;
use tauri::ipc::Channel;
use tokio::sync::watch;

use crate::diag;
use crate::error::Result;
use crate::platform::diag::{self as args, OS};

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum DiagKind {
    Ping,
    Trace,
}

static RUNNING: LazyLock<Mutex<HashMap<u32, watch::Sender<bool>>>> =
    LazyLock::new(Default::default);

/// Runs ping (`count` None = until stopped) or traceroute to `target`, streaming output
/// lines; resolves with the tool's exit code (null when stopped).
#[tauri::command]
pub async fn diag_run(
    job: u32,
    kind: DiagKind,
    target: String,
    count: Option<u32>,
    resolve: bool,
    on_line: Channel<String>,
) -> Result<Option<i32>> {
    let target = diag::validate_target(&target)?;
    let probes = match kind {
        DiagKind::Ping => vec![args::ping(
            OS,
            target,
            count.map(|c| c.clamp(1, 1000)),
            2000,
        )],
        DiagKind::Trace => args::trace(OS, target, 30, resolve),
    };
    let (tx, rx) = watch::channel(false);
    if let Ok(mut g) = RUNNING.lock() {
        g.insert(job, tx);
    }
    let result = diag::run(&probes, rx, &move |l| {
        let _ = on_line.send(l);
    })
    .await;
    if let Ok(mut g) = RUNNING.lock() {
        g.remove(&job);
    }
    result
}

#[tauri::command]
pub fn diag_stop(job: u32) {
    if let Ok(g) = RUNNING.lock()
        && let Some(tx) = g.get(&job)
    {
        let _ = tx.send(true);
    }
}
