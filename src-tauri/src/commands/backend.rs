//! Backend commands: which backend runs what Podium executes (Settings →
//! General → "Backend"): the WSL distro, and whether Windows-drive projects
//! opt out of it. The file is read once at startup
//! ([`crate::init_backend`]); these commands only read and rewrite it, so a
//! change applies on the next launch.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use podium_core::backend_settings::{self, BackendSettings};

use crate::error::IpcError;

/// File name inside the app data directory.
pub const BACKEND_FILE: &str = "backend.json";

/// What the General tab shows: the saved choices and the ones this run
/// uses. When they differ, the UI says a restart is needed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendSettingsDto {
    /// Saved WSL distro; empty = the native backend.
    pub wsl_distro: String,
    /// The WSL distro this run actually uses; empty = native.
    pub active_wsl_distro: String,
    /// Saved opt-in: Windows-drive projects run natively in WSL mode.
    pub native_drive_projects: bool,
    /// Whether this run actually does (false when not in WSL mode).
    pub active_native_drive_projects: bool,
}

fn backend_path(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| IpcError::new("io", format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(BACKEND_FILE))
}

fn to_dto(settings: BackendSettings) -> BackendSettingsDto {
    let active = podium_core::platform::wsl();
    BackendSettingsDto {
        wsl_distro: settings.wsl_distro.unwrap_or_default(),
        active_wsl_distro: active.map(|w| w.distro().to_string()).unwrap_or_default(),
        native_drive_projects: settings.native_drive_projects,
        active_native_drive_projects: active.is_some_and(|w| w.native_drive_projects()),
    }
}

/// The saved backend settings plus the backend active in this run.
#[tauri::command]
pub fn backend_settings_get(app: AppHandle) -> Result<BackendSettingsDto, IpcError> {
    Ok(to_dto(backend_settings::load(&backend_path(&app)?)))
}

/// In WSL-backend mode, the distro's networking mode (`mirrored`, `nat`, …);
/// `None` natively or when WSL can't tell. The frontend warns once at
/// startup when it isn't `mirrored`, since agents in WSL then can't reach
/// the MCP server on 127.0.0.1.
#[tauri::command]
pub async fn backend_wsl_networking_mode() -> Result<Option<String>, IpcError> {
    tauri::async_runtime::spawn_blocking(podium_core::platform::wsl_networking_mode)
        .await
        .map_err(|e| IpcError::new("io", format!("networking check failed: {e}")))
}

/// Save the WSL distro to use from the next launch on. Blank or absent =
/// the native backend; a distro that is not installed is refused.
#[tauri::command]
pub fn backend_settings_set_wsl_distro(
    app: AppHandle,
    distro: Option<String>,
) -> Result<BackendSettingsDto, IpcError> {
    let saved = backend_settings::set_wsl_distro(&backend_path(&app)?, distro)?;
    Ok(to_dto(saved))
}

/// Save the WSL-mode opt-in that runs Windows-drive projects natively
/// (cmd.exe, Windows git, Windows CLIs) instead of in the distro. Off by
/// default; applies on the next launch, like the distro itself.
#[tauri::command]
pub fn backend_settings_set_native_drive_projects(
    app: AppHandle,
    enabled: bool,
) -> Result<BackendSettingsDto, IpcError> {
    let saved = backend_settings::set_native_drive_projects(&backend_path(&app)?, enabled)?;
    Ok(to_dto(saved))
}
