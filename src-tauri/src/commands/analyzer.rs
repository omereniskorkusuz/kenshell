//! Analyzer commands (P4-T01): thin wrappers over `crate::analyzer`.

use crate::analyzer::diagnostics::{self, CommandOutput, Finding};
use crate::analyzer::{self, ParserInfo, Table};
use crate::error::{AppError, Result};

/// Parses `text` with the parser for `command` (or the echoed command line in the text).
/// None = no known command.
#[tauri::command]
pub fn analyze_output(
    text: String,
    family: Option<String>,
    command: Option<String>,
) -> Option<Table> {
    analyzer::analyze(&text, family.as_deref(), command.as_deref())
}

#[tauri::command]
pub fn analyzer_parsers() -> Vec<ParserInfo> {
    analyzer::list()
}

/// Writes a table (as shown: filtered and sorted by the UI) as CSV to `path`, a path the
/// user picked in the save dialog.
#[tauri::command]
pub fn analyzer_export_csv(
    path: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
) -> Result<()> {
    if rows.len() > analyzer::MAX_ROWS {
        return Err(AppError::invalid("too many rows"));
    }
    std::fs::write(&path, analyzer::csv::to_csv(&headers, &rows)).map_err(AppError::from)
}

/// The read-only command set System Diagnostics runs for a vendor family.
#[tauri::command]
pub fn diagnostics_commands(family: Option<String>) -> Vec<String> {
    diagnostics::commands(family.as_deref())
        .iter()
        .map(|c| (*c).to_owned())
        .collect()
}

/// Findings for the outputs of the diagnostics commands, most severe first.
#[tauri::command]
pub fn diagnostics_evaluate(family: Option<String>, outputs: Vec<CommandOutput>) -> Vec<Finding> {
    diagnostics::evaluate(family.as_deref(), &outputs)
}

/// Offline best-practice analysis of a running-config (P4-T07).
#[tauri::command]
pub fn config_audit(family: Option<String>, config: String) -> Vec<Finding> {
    crate::analyzer::config_rules::audit(family.as_deref(), &config)
}

/// Streams a `show tech` / crashinfo / log file and summarises it (P4-T09).
#[tauri::command]
pub async fn logfile_analyze(path: String) -> Result<crate::analyzer::logfile::FileSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let f = std::fs::File::open(&path).map_err(AppError::from)?;
        crate::analyzer::logfile::analyze(std::io::BufReader::with_capacity(256 * 1024, f))
            .map_err(AppError::from)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// Lines `start..=end` of an analysed file (a section), capped at 20 000 lines / 4 MB.
#[tauri::command]
pub async fn logfile_lines(path: String, start: u64, end: u64) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let f = std::fs::File::open(&path).map_err(AppError::from)?;
        crate::analyzer::logfile::read_lines(
            std::io::BufReader::new(f),
            start,
            end,
            20_000,
            4 << 20,
        )
        .map_err(AppError::from)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// CDP / LLDP neighbours of a live session (read hidden), merged (P4-T10). Commands the
/// device does not know are skipped.
#[tauri::command]
pub async fn neighbors_discover(
    state: tauri::State<'_, crate::state::AppState>,
    id: crate::model::RuntimeId,
    family: Option<String>,
) -> Result<Vec<crate::analyzer::neighbors::Neighbor>> {
    use crate::analyzer::neighbors;
    let mut outputs = Vec::new();
    for c in neighbors::commands(family.as_deref()) {
        match state
            .sessions
            .capture(id, c, std::time::Duration::from_secs(60))
            .await
        {
            Ok(text) => outputs.push(((*c).to_owned(), text)),
            Err(e) if e.code == crate::error::ErrorCode::ConnectionClosed => return Err(e),
            Err(_) => {}
        }
    }
    Ok(neighbors::neighbors(family.as_deref(), &outputs))
}
