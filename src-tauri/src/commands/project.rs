//! Project lifecycle commands: open, close, list, config reload.

use std::path::PathBuf;

use tauri::{AppHandle, State};

use podium_core::{ProjectId, ProjectInfo, ProjectSections};

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
    let raw_path = PathBuf::from(path);
    // Before the folder is canonicalized (see `Orchestrator::open_project`),
    // note its old (pre-`dunce`) identity, if different — on Windows, a
    // project that was opened by an older build persisted its workspace and
    // recents entries under that `\\?\`-prefixed path.
    let legacy_root = podium_core::project::legacy_canonical_root(&raw_path);
    let id = state.orchestrator.open_project(raw_path).await?;
    let root = state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.root)
        .ok_or_else(|| IpcError::new("projectNotFound", "project vanished after open"))?;
    if let Some(legacy_root) = &legacy_root {
        if let Err(e) = workspace::migrate_legacy_path(&app, legacy_root, &root) {
            tracing::warn!("failed to migrate legacy workspace entry: {e}");
        }
        if let Err(e) = recents::migrate_legacy_path(&app, legacy_root, &root) {
            tracing::warn!("failed to migrate legacy recents entry: {e}");
        }
    }
    // Re-apply any persisted display-name override and badge colour before
    // returning, so a renamed/recoloured project comes back that way after a
    // restart. One read of workspace.json, one ProjectUpdated for both.
    match workspace::overrides_for(&app, &root) {
        Ok((None, None)) => {}
        Ok((name, color)) => {
            if let Err(e) = state
                .orchestrator
                .restore_project_overrides(id, name, color)
            {
                tracing::warn!("failed to apply persisted project overrides: {e}");
            }
        }
        Err(e) => tracing::warn!("failed to read persisted project overrides: {e}"),
    }
    // Same for the section toggles.
    if let Ok(Some(sections)) = workspace::sections_for(&app, &root) {
        if let Err(e) = state.orchestrator.set_project_sections(id, sections) {
            tracing::warn!("failed to apply persisted project sections: {e}");
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

/// Set which optional sidebar sections a project shows. Persisted in
/// `workspace.json`; a persist failure only means it won't survive a restart.
#[tauri::command]
pub fn project_set_sections(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    sections: ProjectSections,
) -> Result<ProjectInfo, IpcError> {
    let info = state
        .orchestrator
        .set_project_sections(project_id, sections)?;
    if let Err(e) = workspace::set_sections(&app, &info.root, sections) {
        tracing::warn!("failed to persist project sections: {e}");
    }
    Ok(info)
}

/// Set or clear a project's sidebar badge colour (`#rrggbb`; blank clears).
/// Persisted in `workspace.json` like the name override.
#[tauri::command]
pub fn project_set_color(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    color: Option<String>,
) -> Result<ProjectInfo, IpcError> {
    let info = state.orchestrator.set_project_color(project_id, color)?;
    if let Err(e) = workspace::set_color(&app, &info.root, info.color.clone()) {
        tracing::warn!("failed to persist project colour: {e}");
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

/// Public recipe names from the project root's justfile (empty when there is
/// none). Feeds the Terminals section's quick-commands list.
#[tauri::command]
pub fn project_just_recipes(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<String>, IpcError> {
    state
        .orchestrator
        .project_just_recipes(project_id)
        .map_err(Into::into)
}

/// Open the project root in a new Warp tab via Warp's URI scheme.
#[tauri::command]
pub async fn project_open_in_warp(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<(), IpcError> {
    let orchestrator = std::sync::Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || orchestrator.open_project_in_warp(project_id))
        .await
        .map_err(|e| IpcError::new("io", format!("open-in-Warp task failed: {e}")))?
        .map_err(Into::into)
}
