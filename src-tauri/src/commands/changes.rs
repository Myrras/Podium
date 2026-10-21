//! Changes-panel commands: the files changed in a project root or one of
//! its worktrees, and one file's diff. Both shell out to git, so they run on
//! a blocking thread.

use std::sync::Arc;

use tauri::State;

use podium_core::{ChangeSet, ChangesBase, FileDiff, ProjectId};

use crate::error::IpcError;
use crate::state::AppState;

/// The files changed in the project root (or the named worktree) against
/// `HEAD` or the default branch.
#[tauri::command]
pub async fn changes_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
    worktree: Option<String>,
    base: ChangesBase,
) -> Result<ChangeSet, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || {
        orchestrator.list_changes(project_id, worktree.as_deref(), base)
    })
    .await
    .map_err(|e| IpcError::new("io", format!("changes task failed: {e}")))?
    .map_err(IpcError::from)
}

/// One changed file's unified diff.
#[tauri::command]
pub async fn changes_file_diff(
    state: State<'_, AppState>,
    project_id: ProjectId,
    worktree: Option<String>,
    base: ChangesBase,
    path: String,
    old_path: Option<String>,
) -> Result<FileDiff, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || {
        orchestrator.change_diff(
            project_id,
            worktree.as_deref(),
            base,
            &path,
            old_path.as_deref(),
        )
    })
    .await
    .map_err(|e| IpcError::new("io", format!("changes task failed: {e}")))?
    .map_err(IpcError::from)
}
