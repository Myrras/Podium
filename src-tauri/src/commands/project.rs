//! Project lifecycle commands: open, close, list, config reload.

use std::path::PathBuf;

use tauri::{AppHandle, State};

use podium_core::{ProjectId, ProjectInfo};

use crate::commands::{recents, workspace};
use crate::error::IpcError;
use crate::state::AppState;

/// Open the directory at `path` (absolute, from the native folder picker) as
/// a project and return its snapshot. Also pushes it onto the recents list
/// and adds it to the persistent workspace list.
#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectInfo, IpcError> {
    let id = state.orchestrator.open_project(PathBuf::from(path)).await?;
    let root = state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.root)
        .ok_or_else(|| IpcError::new("projectNotFound", "project vanished after open"))?;
    // Re-apply any persisted display-name override before returning, so a
    // renamed project comes back with its user-chosen name after a restart.
    if let Ok(Some(name)) = workspace::name_for(&app, &root) {
        if let Err(e) = state.orchestrator.rename_project(id, Some(name)) {
            tracing::warn!("failed to apply persisted project name: {e}");
        }
    }
    let info = state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IpcError::new("projectNotFound", "project vanished after open"))?;
    // Recents are a convenience; failing to persist them must not fail the open.
    if let Err(e) = recents::push(&app, &info.root, &info.name) {
        tracing::warn!("failed to update recents: {e}");
    }
    if let Err(e) = workspace::add(&app, &info.root) {
        tracing::warn!("failed to update workspace: {e}");
    }
    Ok(info)
}

/// Rename a project (a blank name clears the override, reverting to the
/// `podium.yml`/folder name). The override is persisted in `workspace.json`
/// so it survives restarts. Returns the updated snapshot.
#[tauri::command]
pub fn project_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    name: Option<String>,
) -> Result<ProjectInfo, IpcError> {
    let info = state.orchestrator.rename_project(project_id, name)?;
    // Persist against the project root; failure to persist must not fail the
    // in-memory rename (it just won't survive a restart).
    let stored = if info.renamed {
        Some(info.name.clone())
    } else {
        None
    };
    if let Err(e) = workspace::set_name(&app, &info.root, stored) {
        tracing::warn!("failed to persist project name: {e}");
    }
    Ok(info)
}

/// Reorder the sidebar project list to match `ordered` (project ids in the
/// desired order). Persists the new order in `workspace.json` and returns the
/// projects in the new order.
#[tauri::command]
pub fn project_reorder(
    app: AppHandle,
    state: State<'_, AppState>,
    ordered: Vec<ProjectId>,
) -> Result<Vec<ProjectInfo>, IpcError> {
    let projects = state.orchestrator.reorder_projects(ordered);
    let roots: Vec<PathBuf> = projects.iter().map(|p| p.root.clone()).collect();
    if let Err(e) = workspace::reorder(&app, &roots) {
        tracing::warn!("failed to persist project order: {e}");
    }
    Ok(projects)
}

/// Re-read `podium.yml` for a project and return its updated snapshot.
#[tauri::command]
pub async fn project_config_reload(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<ProjectInfo, IpcError> {
    state.orchestrator.reload_project_config(project_id).await?;
    state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| IpcError::new("projectNotFound", "project vanished after reload"))
}

/// Close a project, stopping and removing all of its processes. Also removes
/// it from the persistent workspace list.
#[tauri::command]
pub async fn project_close(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<(), IpcError> {
    let root = state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == project_id)
        .map(|p| p.root);
    state.orchestrator.close_project(project_id).await?;
    if let Some(root) = root {
        if let Err(e) = workspace::remove(&app, &root) {
            tracing::warn!("failed to update workspace: {e}");
        }
    }
    Ok(())
}

#[tauri::command]
pub fn project_list(state: State<'_, AppState>) -> Vec<ProjectInfo> {
    state.orchestrator.list_projects()
}
