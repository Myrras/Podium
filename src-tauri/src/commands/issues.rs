//! Issues command — a thin forwarder over the orchestrator's issue listing.

use std::sync::Arc;

use serde::Serialize;
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

/// What the Settings → Issues tab shows. The token is never included — only
/// whether one is stored.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuesSettingsDto {
    pub gitlab_host: Option<String>,
    pub gitlab_token_set: bool,
}

/// The saved GitLab host and whether its token is in the keychain.
#[tauri::command]
pub async fn issues_settings_get(
    state: State<'_, AppState>,
) -> Result<IssuesSettingsDto, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || {
        let (gitlab_host, gitlab_token_set) = orchestrator.issues_settings();
        IssuesSettingsDto {
            gitlab_host,
            gitlab_token_set,
        }
    })
    .await
    .map_err(|e| IpcError::new("io", format!("issues settings task failed: {e}")))
}

/// Save the GitLab host (blank clears it). `gitlab_token`: omitted leaves the
/// stored token alone, blank deletes it, anything else replaces it.
#[tauri::command]
pub async fn issues_settings_set(
    state: State<'_, AppState>,
    gitlab_host: Option<String>,
    gitlab_token: Option<String>,
) -> Result<IssuesSettingsDto, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || {
        orchestrator.set_issues_settings(gitlab_host, gitlab_token)?;
        let (gitlab_host, gitlab_token_set) = orchestrator.issues_settings();
        Ok(IssuesSettingsDto {
            gitlab_host,
            gitlab_token_set,
        })
    })
    .await
    .map_err(|e| IpcError::new("io", format!("issues settings task failed: {e}")))?
    .map_err(|e: podium_core::CoreError| IpcError::from(e))
}
