//! Per-project to-do commands — thin shims over the orchestrator's to-do
//! API. Persistence (a JSON file in the app data dir, keyed by project root)
//! lives in `podium_core::todo`.

use tauri::State;

use podium_core::{CommentId, LinkId, ProjectId, TodoId, TodoInfo};

use crate::error::IpcError;
use crate::state::AppState;

/// List a project's to-dos in creation order.
#[tauri::command]
pub fn todo_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<TodoInfo>, IpcError> {
    state
        .orchestrator
        .list_todos(project_id)
        .map_err(Into::into)
}

/// List a project's archived to-dos, most recently archived first.
#[tauri::command]
pub fn todo_list_archived(
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> Result<Vec<TodoInfo>, IpcError> {
    state
        .orchestrator
        .list_archived_todos(project_id)
        .map_err(Into::into)
}

/// Archive or unarchive a to-do; returns the updated snapshot.
#[tauri::command]
pub fn todo_set_archived(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    archived: bool,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .set_todo_archived(project_id, todo_id, archived)
        .map_err(Into::into)
}

/// Add a to-do; blank text is rejected by the core.
#[tauri::command]
pub fn todo_add(
    state: State<'_, AppState>,
    project_id: ProjectId,
    text: String,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .add_todo(project_id, &text)
        .map_err(Into::into)
}

/// Mark a to-do as done / not done; returns the updated snapshot.
#[tauri::command]
pub fn todo_set_done(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    done: bool,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .set_todo_done(project_id, todo_id, done)
        .map_err(Into::into)
}

/// Revise a to-do's text and/or description. Each `Some` field is applied
/// (a blank `description` clears it); at least one must be given.
#[tauri::command]
pub fn todo_update(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    text: Option<String>,
    description: Option<String>,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .update_todo(project_id, todo_id, text.as_deref(), description.as_deref())
        .map_err(Into::into)
}

/// Append a progress note to a to-do. A blank `author` defaults to `You`
/// (the human), distinguishing user notes from agents' `agent` notes.
#[tauri::command]
pub fn todo_comment(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    text: String,
    author: Option<String>,
) -> Result<TodoInfo, IpcError> {
    let author = author.unwrap_or_else(|| "You".to_string());
    state
        .orchestrator
        .comment_todo(project_id, todo_id, &author, &text)
        .map_err(Into::into)
}

/// Revise an existing comment's text; blank text is rejected by the core.
#[tauri::command]
pub fn todo_comment_update(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    comment_id: CommentId,
    text: String,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .edit_todo_comment(project_id, todo_id, comment_id, &text)
        .map_err(Into::into)
}

/// Remove a comment from a to-do; returns the updated snapshot.
#[tauri::command]
pub fn todo_comment_remove(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    comment_id: CommentId,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .remove_todo_comment(project_id, todo_id, comment_id)
        .map_err(Into::into)
}

/// Pin an issue/PR link to a to-do; a blank `label` falls back to the url,
/// and the url must be http(s) (validated by the core).
#[tauri::command]
pub fn todo_add_link(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    url: String,
    label: Option<String>,
) -> Result<TodoInfo, IpcError> {
    let label = label.unwrap_or_default();
    state
        .orchestrator
        .add_todo_link(project_id, todo_id, &label, &url)
        .map_err(Into::into)
}

/// Remove a pinned link from a to-do; returns the updated snapshot.
#[tauri::command]
pub fn todo_remove_link(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
    link_id: LinkId,
) -> Result<TodoInfo, IpcError> {
    state
        .orchestrator
        .remove_todo_link(project_id, todo_id, link_id)
        .map_err(Into::into)
}

/// Remove a to-do from a project.
#[tauri::command]
pub fn todo_remove(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .remove_todo(project_id, todo_id)
        .map_err(Into::into)
}

/// Best-effort cancel/rollback request sent to an agent's stdin when the user
/// unassigns its to-do. Podium-owned text; a trailing newline submits it if
/// the agent is sitting at an input prompt.
const CANCEL_REQUEST: &str = "Please stop working on the current Podium to-do and roll back any \
     uncommitted changes you made for it.\n";

/// Unassign a to-do from its agent (the sidebar (x) action). Before clearing
/// the link, a best-effort cancel/rollback request is injected into the
/// (still-)assigned agent's stdin — best-effort because it only lands if the
/// agent is running and at an input prompt. Returns the updated to-do.
#[tauri::command]
pub async fn todo_unassign(
    state: State<'_, AppState>,
    project_id: ProjectId,
    todo_id: TodoId,
) -> Result<TodoInfo, IpcError> {
    // Ask the agent to cancel while it is still linked; ignore failures (it
    // may have already exited) so the unassign always proceeds.
    if let Some(agent) = state.orchestrator.agent_for_todo(todo_id) {
        let _ = state
            .orchestrator
            .write_stdin(agent, CANCEL_REQUEST.as_bytes())
            .await;
    }
    state
        .orchestrator
        .unassign_todo(project_id, todo_id)
        .map_err(Into::into)
}
