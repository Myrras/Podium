//! Worktree commands: list and remove a project's Podium-managed git
//! worktrees. Creation happens via the spawn checkbox (`agent_spawn`) or the
//! `create_worktree` MCP tool — there is deliberately no create command here.

use std::sync::Arc;

use tauri::State;

use podium_core::{ProjectId, WorktreeInfo};

use crate::error::IpcError;
use crate::state::AppState;

/// List the project's Podium-managed git worktrees. Shells out to git, so it
/// runs on a blocking thread.
#[tauri::command]
pub async fn worktree_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<WorktreeInfo>, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || orchestrator.list_worktrees(project_id))
        .await
        .map_err(|e| IpcError::new("io", format!("worktree task failed: {e}")))?
        .map_err(IpcError::from)
}

/// Remove a Podium-managed git worktree and return the refreshed list.
/// Refused while a process runs in it, or while it has uncommitted changes
/// unless `force`.
#[tauri::command]
pub async fn worktree_remove(
    state: State<'_, AppState>,
    project_id: ProjectId,
    name: String,
    force: bool,
) -> Result<Vec<WorktreeInfo>, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || {
        orchestrator.remove_worktree(project_id, &name, force)?;
        orchestrator.list_worktrees(project_id)
    })
    .await
    .map_err(|e| IpcError::new("io", format!("worktree task failed: {e}")))?
    .map_err(IpcError::from)
}
