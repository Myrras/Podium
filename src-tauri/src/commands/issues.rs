//! Issues command — a thin forwarder over the orchestrator's issue listing.

use std::sync::Arc;

use tauri::State;

use podium_core::{IssueInfo, ProjectId};

use crate::error::IpcError;
use crate::state::AppState;

/// List the user's open assigned issues for the project's repo. Shells out
/// to git/gh, so it runs on a blocking thread.
#[tauri::command]
pub async fn issues_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<IssueInfo>, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || orchestrator.list_issues(project_id))
        .await
        .map_err(|e| IpcError::new("io", format!("issues task failed: {e}")))?
        .map_err(IpcError::from)
}
