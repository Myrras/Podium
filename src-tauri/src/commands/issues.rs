//! Issues commands — thin forwarders over the orchestrator's Git board
//! and Settings → Issues setups. Keychain/git/gh work runs on a blocking thread.

use std::sync::Arc;

use tauri::State;

use podium_core::{
    Board, CachedBoard, CoreResult, IssueProvider, IssuesOverview, ItemKind, Orchestrator,
    ProjectId,
};

use crate::error::IpcError;
use crate::state::AppState;

/// The project's Git board (`mine` = only the user's own issues/PRs). Shells
/// out to git/gh, so it runs on a blocking thread.
#[tauri::command]
pub async fn issues_board(
    state: State<'_, AppState>,
    project_id: ProjectId,
    mine: bool,
) -> Result<Board, IpcError> {
    blocking(state, move |o| {
        o.issue_board(project_id, mine).map(|c| c.board)
    })
    .await
}

/// The last fetched board of this scope, without touching the network.
#[tauri::command]
pub fn issues_board_cached(
    state: State<'_, AppState>,
    project_id: ProjectId,
    mine: bool,
) -> Result<Option<CachedBoard>, IpcError> {
    state
        .orchestrator
        .cached_issue_board(project_id, mine)
        .map_err(IpcError::from)
}

/// Move a card to another column on the platform (validated against the
/// cached board); the UI follows the `board:changed` event.
#[tauri::command]
pub async fn issues_board_move(
    state: State<'_, AppState>,
    project_id: ProjectId,
    kind: ItemKind,
    number: u64,
    column: String,
) -> Result<(), IpcError> {
    blocking(state, move |o| {
        o.move_board_item(project_id, kind, number, &column)
    })
    .await
}

/// Unassign the signed-in user from a card on the platform (validated against
/// the cached board); the UI follows the `board:changed` event.
#[tauri::command]
pub async fn issues_board_unassign(
    state: State<'_, AppState>,
    project_id: ProjectId,
    kind: ItemKind,
    number: u64,
) -> Result<(), IpcError> {
    blocking(state, move |o| {
        o.unassign_board_item(project_id, kind, number)
    })
    .await
}

/// Run an issues call on a blocking thread (git/gh, GitLab HTTP, the keychain).
async fn blocking<T: Send + 'static>(
    state: State<'_, AppState>,
    f: impl FnOnce(&Orchestrator) -> CoreResult<T> + Send + 'static,
) -> Result<T, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || f(&orchestrator))
        .await
        .map_err(|e| IpcError::new("io", format!("issues task failed: {e}")))?
        .map_err(IpcError::from)
}

/// Every setup (tokens never included — only `tokenSet`) and each project's pick.
#[tauri::command]
pub async fn issues_settings_get(state: State<'_, AppState>) -> Result<IssuesOverview, IpcError> {
    blocking(state, |o| Ok(o.issues_overview())).await
}

/// Add (`id` omitted) or update a setup. `token` (GitLab): omitted keeps the
/// stored one, blank deletes it, anything else replaces it.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn issues_setup_save(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    provider: IssueProvider,
    url: String,
    account: Option<String>,
    token: Option<String>,
) -> Result<IssuesOverview, IpcError> {
    blocking(state, move |o| {
        o.save_issue_setup(id, name, provider, url, account, token)
    })
    .await
}

/// Remove a setup and its stored token.
#[tauri::command]
pub async fn issues_setup_remove(
    state: State<'_, AppState>,
    id: String,
) -> Result<IssuesOverview, IpcError> {
    blocking(state, move |o| o.remove_issue_setup(&id)).await
}

/// Pick the setup a project's Issues section uses; omitted = match by host.
#[tauri::command]
pub async fn issues_project_setup_set(
    state: State<'_, AppState>,
    project_id: ProjectId,
    setup_id: Option<String>,
) -> Result<IssuesOverview, IpcError> {
    blocking(state, move |o| {
        o.set_project_issue_setup(project_id, setup_id)
    })
    .await
}
