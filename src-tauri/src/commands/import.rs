//! Import (PuTTY, SecureCRT, MobaXterm, CSV) and export (JSON, CSV) commands.
//! Exports contain no secrets — the database never has any.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{AppError, ErrorCode, Result};
use crate::import::{self, ImportReport, Imported, MAX_FILE_BYTES};
use crate::state::AppState;

fn read_text(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(AppError::invalid("file is too large for an import"));
    }
    let bytes = std::fs::read(path)?;
    // SecureCRT/MobaXterm on Windows may write UTF-16 LE with BOM.
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn appdata() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

fn securecrt_default() -> Option<PathBuf> {
    let candidates = [
        appdata().map(|a| a.join("VanDyke").join("Config").join("Sessions")),
        home().map(|h| h.join("Library/Application Support/VanDyke/SecureCRT/Config/Sessions")),
        home().map(|h| h.join(".vandyke/SecureCRT/Config/Sessions")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}

fn mobaxterm_default() -> Option<PathBuf> {
    let candidates = [
        appdata().map(|a| a.join("MobaXterm").join("MobaXterm.ini")),
        home().map(|h| h.join("Documents").join("MobaXterm").join("MobaXterm.ini")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// PuTTY sessions: Windows registry, else `~/.putty/sessions`.
fn putty_sessions() -> Vec<Imported> {
    let mut entries = Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\SimonTatham\PuTTY\Sessions", "/s"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            entries = import::parse_reg_query(&String::from_utf8_lossy(&out.stdout));
        }
    }
    if entries.is_empty()
        && let Some(dir) = home().map(|h| h.join(".putty").join("sessions"))
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.flatten() {
            if let (Some(name), Ok(text)) = (e.file_name().to_str(), read_text(&e.path())) {
                entries.push((name.to_owned(), import::parse_putty_file(&text)));
            }
        }
    }
    entries
        .iter()
        .filter_map(|(n, kv)| import::putty_session(n, kv))
        .collect()
}

fn securecrt_sessions(root: &Path) -> Vec<Imported> {
    fn walk(dir: &Path, folders: &[String], out: &mut Vec<Imported>, depth: usize) {
        if depth > 16 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                let mut f = folders.to_vec();
                f.push(name);
                walk(&p, &f, out, depth + 1);
            } else if let Some(stem) = name.strip_suffix(".ini")
                && let Ok(text) = read_text(&p)
                && let Some(s) = import::securecrt_session(stem, folders.to_vec(), &text)
            {
                out.push(s);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &[], &mut out, 0);
    out
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSources {
    pub putty_count: usize,
    pub secure_crt_dir: Option<String>,
    pub mobaxterm_ini: Option<String>,
}

#[tauri::command]
pub async fn import_detect() -> ImportSources {
    tauri::async_runtime::spawn_blocking(|| ImportSources {
        putty_count: putty_sessions().len(),
        secure_crt_dir: securecrt_default().map(|p| p.display().to_string()),
        mobaxterm_ini: mobaxterm_default().map(|p| p.display().to_string()),
    })
    .await
    .unwrap_or(ImportSources {
        putty_count: 0,
        secure_crt_dir: None,
        mobaxterm_ini: None,
    })
}

/// `source`: `putty` | `securecrt` | `mobaxterm` | `csv`. `path` is needed for the
/// last three (folder for SecureCRT, file otherwise).
#[tauri::command]
pub async fn import_sessions(
    state: State<'_, AppState>,
    source: String,
    path: Option<String>,
) -> Result<ImportReport> {
    let db = std::sync::Arc::clone(&state.db);
    let secrets = std::sync::Arc::clone(&state.secrets);
    tauri::async_runtime::spawn_blocking(move || {
        let need_path = || {
            path.clone()
                .map(PathBuf::from)
                .ok_or_else(|| AppError::invalid("no file selected"))
        };
        let mut report = ImportReport::default();
        let (root, items) = match source.as_str() {
            "putty" => ("PuTTY", putty_sessions()),
            "securecrt" => ("SecureCRT", securecrt_sessions(&need_path()?)),
            "mobaxterm" => (
                "MobaXterm",
                import::parse_mobaxterm(&read_text(&need_path()?)?),
            ),
            "csv" => {
                let (items, errors) = import::csv::import(&read_text(&need_path()?)?);
                report.errors = errors;
                ("CSV", items)
            }
            other => return Err(AppError::invalid(format!("unknown import source {other}"))),
        };
        // A new session must never inherit a deleted one's saved password (id reuse).
        Ok(import::apply_with(
            &db,
            Some(root),
            items,
            report,
            |_, s| {
                crate::secrets::forget_session(secrets.as_ref(), s.id);
            },
        ))
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[derive(Serialize)]
struct JsonExport<'a> {
    format: &'static str,
    version: u32,
    folders: &'a [crate::model::Folder],
    sessions: &'a [crate::model::SessionRecord],
}

/// Writes all folders and sessions (no secrets) as `json` or `csv`.
#[tauri::command]
pub fn export_sessions(state: State<'_, AppState>, path: String, format: String) -> Result<()> {
    let folders = state.db.list_folders()?;
    let sessions = state.db.list_sessions()?;
    let text = match format.as_str() {
        "json" => serde_json::to_string_pretty(&JsonExport {
            format: "kenshell-sessions",
            version: 1,
            folders: &folders,
            sessions: &sessions,
        })
        .map_err(|e| AppError::internal(e.to_string()))?,
        "csv" => import::csv::export(&folders, &sessions),
        other => return Err(AppError::invalid(format!("unknown format {other}"))),
    };
    std::fs::write(&path, text).map_err(|e| AppError::with(ErrorCode::Io, format!("{path}: {e}")))
}

/// Native file dialogs. `kind`: `file` | `folder` | `save`; `extensions` filter
/// (e.g. `["csv"]`); `name` = suggested file name for `save`. `None` = cancelled.
#[tauri::command]
pub async fn pick_path(
    app: AppHandle,
    kind: String,
    extensions: Option<Vec<String>>,
    name: Option<String>,
) -> Result<Option<String>> {
    let mut dialog = app.dialog().file();
    if let Some(ext) = &extensions {
        let exts: Vec<&str> = ext.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(exts.join(", "), &exts);
    }
    if let Some(n) = &name {
        dialog = dialog.set_file_name(n);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || match kind.as_str() {
        "folder" => dialog.blocking_pick_folder(),
        "save" => dialog.blocking_save_file(),
        _ => dialog.blocking_pick_file(),
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
}
