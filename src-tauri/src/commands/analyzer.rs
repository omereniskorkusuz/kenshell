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
