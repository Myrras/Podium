//! Per-project scratchpad commands — thin shims over the orchestrator's
//! scratchpad API. Persistence (a JSON file in the app data dir, keyed by
//! project root) lives in `podium_core::scratchpad`.

use chrono::{DateTime, Utc};
use tauri::State;

use podium_core::{ProjectId, ScratchpadId, ScratchpadInfo};

use crate::error::IpcError;
use crate::state::AppState;

/// List a project's active (non-archived) scratchpads.
#[tauri::command]
pub fn scratchpad_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<ScratchpadInfo>, IpcError> {
    state
        .orchestrator
        .list_scratchpads(project_id)
        .map_err(Into::into)
}

/// List a project's archived scratchpads, most recently archived first.
#[tauri::command]
pub fn scratchpad_list_archived(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<ScratchpadInfo>, IpcError> {
    state
        .orchestrator
        .list_archived_scratchpads(project_id)
        .map_err(Into::into)
}

/// Create a new scratchpad (auto-generated timestamp title, empty content).
/// Always attributed to `"User"` — this command is only ever called from the
/// human-facing UI; agents get their own MCP tool.
#[tauri::command]
pub fn scratchpad_add(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .add_scratchpad(project_id, "User")
        .map_err(Into::into)
}

/// Replace a scratchpad's content (bumps its version). Always attributed to
/// `"User"`. `expected_updated_at` must match the scratchpad's current
/// `updatedAt` or this fails with `IpcError.kind === "scratchpadConflict"` —
/// someone else (an agent, most likely) edited it first.
#[tauri::command]
pub fn scratchpad_update_content(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
    content: String,
    expected_updated_at: DateTime<Utc>,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .update_scratchpad_content(project_id, id, &content, expected_updated_at, "User")
        .map_err(Into::into)
}

/// Revise a scratchpad's title (blank falls back to a timestamp title).
/// Always attributed to `"User"`. `expected_updated_at` is checked the same
/// way as in `scratchpad_update_content`.
#[tauri::command]
pub fn scratchpad_update_title(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
    title: String,
    expected_updated_at: DateTime<Utc>,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .update_scratchpad_title(project_id, id, &title, expected_updated_at, "User")
        .map_err(Into::into)
}

/// Add a free-text tag to a scratchpad (blank rejected, dedup by value).
#[tauri::command]
pub fn scratchpad_add_tag(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
    tag: String,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .add_scratchpad_tag(project_id, id, &tag)
        .map_err(Into::into)
}

/// Remove a tag from a scratchpad by exact value (idempotent).
#[tauri::command]
pub fn scratchpad_remove_tag(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
    tag: String,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .remove_scratchpad_tag(project_id, id, &tag)
        .map_err(Into::into)
}

/// Archive or unarchive a scratchpad.
#[tauri::command]
pub fn scratchpad_set_archived(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
    archived: bool,
) -> Result<ScratchpadInfo, IpcError> {
    state
        .orchestrator
        .set_scratchpad_archived(project_id, id, archived)
        .map_err(Into::into)
}

/// Permanently remove a scratchpad (from the Archive modal, mirroring
/// `todo_remove`).
#[tauri::command]
pub fn scratchpad_remove(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .remove_scratchpad(project_id, id)
        .map_err(Into::into)
}

/// Best-effort cancel/rollback request sent to an agent's stdin when the user
/// unassigns its scratchpad. Podium-owned text; a trailing newline submits it
/// if the agent is sitting at an input prompt.
const CANCEL_REQUEST: &str = "Please stop working on the current Podium scratchpad and roll back \
     any uncommitted changes you made for it.\n";

/// Unassign a scratchpad from its agent (the sidebar (x) action). Before
/// clearing the link, a best-effort cancel/rollback request is injected into
/// the (still-)assigned agent's stdin — best-effort because it only lands if
/// the agent is running and at an input prompt. Returns the updated
/// scratchpad. There is no MCP-facing counterpart (unlike `todo_unassign`'s
/// sibling `assign_todo` tool) — a scratchpad's assignment is only ever set
/// at spawn time.
#[tauri::command]
pub async fn scratchpad_unassign(
    state: State<'_, AppState>,
    project_id: ProjectId,
    id: ScratchpadId,
) -> Result<ScratchpadInfo, IpcError> {
    // Ask the agent to cancel while it is still linked; ignore failures (it
    // may have already exited) so the unassign always proceeds.
    if let Some(agent) = state.orchestrator.agent_for_scratchpad(id) {
        let _ = state
            .orchestrator
            .write_stdin(agent, CANCEL_REQUEST.as_bytes())
            .await;
    }
    state
        .orchestrator
        .unassign_scratchpad(project_id, id)
        .map_err(Into::into)
}
