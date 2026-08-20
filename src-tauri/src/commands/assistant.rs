//! Personal-assistant commands: sending a turn, the autonomy setting, and
//! deciding on a proposed action. Replies (and the outcome of a proposal)
//! stream back as global `assistant:event` events (see `events.rs`).

use tauri::State;

use podium_core::{AutonomyMode, ProposalId};

use crate::error::IpcError;
use crate::state::AppState;

/// Send a chat turn to the assistant, spawning its headless session on first
/// use. The reply arrives as `assistant:event` payloads, not as the return
/// value.
#[tauri::command]
pub async fn assistant_send(state: State<'_, AppState>, text: String) -> Result<(), IpcError> {
    state.orchestrator.assistant_send(&text).await?;
    Ok(())
}

/// The assistant's current autonomy mode (how much it may do unasked).
#[tauri::command]
pub async fn assistant_autonomy_get(state: State<'_, AppState>) -> Result<AutonomyMode, IpcError> {
    Ok(state.orchestrator.assistant_autonomy())
}

/// Change the autonomy mode; returns what was stored.
#[tauri::command]
pub async fn assistant_autonomy_set(
    state: State<'_, AppState>,
    mode: AutonomyMode,
) -> Result<AutonomyMode, IpcError> {
    Ok(state.orchestrator.set_assistant_autonomy(mode)?)
}

/// Confirm (`approved`) or reject one proposed action. A confirmed proposal
/// runs now; the outcome arrives as `assistant:event` payloads.
#[tauri::command]
pub async fn assistant_resolve_proposal(
    state: State<'_, AppState>,
    id: ProposalId,
    approved: bool,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .resolve_assistant_proposal(id, approved)
        .await?;
    Ok(())
}
