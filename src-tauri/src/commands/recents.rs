//! Persistent "recent projects" list, stored as JSON in the app data dir.
//!
//! Writes are atomic (temp file + rename) and the list is capped at
//! [`MAX_RECENTS`]. A missing or corrupt file is treated as an empty list.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::IpcError;

/// Maximum number of remembered projects.
const MAX_RECENTS: usize = 20;
/// File name inside the app data directory.
const RECENTS_FILE: &str = "recents.json";

/// Serializes load-modify-save cycles so concurrent commands cannot clobber
/// each other's writes.
static RECENTS_LOCK: Mutex<()> = Mutex::new(());

/// One remembered project, most recent first on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: PathBuf,
    pub name: String,
    /// Unix time in milliseconds of the last successful open.
    pub last_opened_at: u64,
}

fn recents_path(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| IpcError::new("io", format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(RECENTS_FILE))
}

/// Read the list; a missing or corrupt file is an empty list.
fn load(path: &Path) -> Vec<RecentProject> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("recents.json is corrupt, starting fresh: {e}");
        Vec::new()
    })
}

/// Atomically persist the list: write a temp file, then rename it over the
/// old one so a crash mid-write can never leave a torn file.
fn save(path: &Path, recents: &[RecentProject]) -> Result<(), IpcError> {
    let dir = path
        .parent()
        .ok_or_else(|| IpcError::new("io", "recents path has no parent directory"))?;
    std::fs::create_dir_all(dir).map_err(IpcError::from)?;
    let json = serde_json::to_string_pretty(recents)
        .map_err(|e| IpcError::new("io", format!("failed to encode recents: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(IpcError::from)?;
    std::fs::rename(&tmp, path).map_err(IpcError::from)?;
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Move `project_path` to the top of the recents list (most recent first).
pub fn push(app: &AppHandle, project_path: &Path, name: &str) -> Result<(), IpcError> {
    let file = recents_path(app)?;
    let _guard = RECENTS_LOCK.lock().expect("recents lock poisoned");
    let mut recents = load(&file);
    recents.retain(|r| r.path != project_path);
    recents.insert(
        0,
        RecentProject {
            path: project_path.to_path_buf(),
            name: name.to_string(),
            last_opened_at: now_millis(),
        },
    );
    recents.truncate(MAX_RECENTS);
    save(&file, &recents)
}

#[tauri::command]
pub fn recents_list(app: AppHandle) -> Result<Vec<RecentProject>, IpcError> {
    let file = recents_path(&app)?;
    let _guard = RECENTS_LOCK.lock().expect("recents lock poisoned");
    Ok(load(&file))
}

/// Remove one entry by path; returns the updated list.
#[tauri::command]
pub fn recents_remove(app: AppHandle, path: String) -> Result<Vec<RecentProject>, IpcError> {
    let file = recents_path(&app)?;
    let _guard = RECENTS_LOCK.lock().expect("recents lock poisoned");
    let mut recents = load(&file);
    recents.retain(|r| r.path != Path::new(&path));
    save(&file, &recents)?;
    Ok(recents)
}
