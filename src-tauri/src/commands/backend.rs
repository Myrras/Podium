//! Backend commands: which backend runs what Podium executes (Settings →
//! General → "Backend"). The file is read once at startup
//! ([`crate::init_backend`]); these commands only read and rewrite it, so a
//! change applies on the next launch.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use podium_core::backend_settings::{self, BackendSettings};

use crate::error::IpcError;

/// File name inside the app data directory.
pub const BACKEND_FILE: &str = "backend.json";

/// What the General tab shows: the saved choice and the one this run uses.
/// When they differ, the UI says a restart is needed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendSettingsDto {
    /// Saved WSL distro; empty = the native backend.
    pub wsl_distro: String,
    /// The WSL distro this run actually uses; empty = native.
    pub active_wsl_distro: String,
}

fn backend_path(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| IpcError::new("io", format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(BACKEND_FILE))
}

fn to_dto(settings: BackendSettings) -> BackendSettingsDto {
    BackendSettingsDto {
        wsl_distro: settings.wsl_distro.unwrap_or_default(),
        active_wsl_distro: podium_core::platform::wsl()
            .map(|w| w.distro().to_string())
            .unwrap_or_default(),
    }
}

/// The saved backend settings plus the backend active in this run.
#[tauri::command]
pub fn backend_settings_get(app: AppHandle) -> Result<BackendSettingsDto, IpcError> {
    Ok(to_dto(backend_settings::load(&backend_path(&app)?)))
}

/// Save the WSL distro to use from the next launch on. Blank or absent =
/// the native backend; a distro that is not installed is refused.
#[tauri::command]
pub fn backend_settings_set_wsl_distro(
    app: AppHandle,
    distro: Option<String>,
) -> Result<BackendSettingsDto, IpcError> {
    let saved = backend_settings::save_checked(
        &backend_path(&app)?,
        BackendSettings { wsl_distro: distro },
    )?;
    Ok(to_dto(saved))
}
