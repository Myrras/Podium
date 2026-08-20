//! Personal-assistant commands: sending a turn, replaying the stored
//! transcript, starting over, its memory, the autonomy and proactivity
//! settings, and deciding on a proposed action. Replies (and the outcome of a proposal)
//! stream back as global `assistant:event` events (see `events.rs`).

use tauri::State;

use podium_core::{AutonomyMode, ProposalId, TranscriptEntry};

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

/// The stored chat rows, oldest first — what the panel replays on startup.
#[tauri::command]
pub async fn assistant_transcript(
    state: State<'_, AppState>,
) -> Result<Vec<TranscriptEntry>, IpcError> {
    Ok(state.orchestrator.assistant_transcript())
}

/// Drop the conversation (session, transcript and pending proposals) and
/// start fresh. The assistant's memory is deliberately kept.
#[tauri::command]
pub async fn assistant_new_conversation(state: State<'_, AppState>) -> Result<(), IpcError> {
    state.orchestrator.assistant_new_conversation().await;
    Ok(())
}

/// What the assistant remembers across conversations (empty when it has
/// written nothing yet).
#[tauri::command]
pub async fn assistant_memory_get(state: State<'_, AppState>) -> Result<String, IpcError> {
    Ok(state.orchestrator.assistant_memory())
}

/// Replace that memory; an empty `content` clears it.
#[tauri::command]
pub async fn assistant_memory_set(
    state: State<'_, AppState>,
    content: String,
) -> Result<(), IpcError> {
    state.orchestrator.set_assistant_memory(&content)?;
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

/// Whether Podium may nudge the assistant unprompted (agent finished, agent
/// stuck, lingering to-dos, day-start summary).
#[tauri::command]
pub async fn assistant_proactivity_get(state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state.orchestrator.assistant_proactivity())
}

/// Turn those nudges on or off; returns what was stored.
#[tauri::command]
pub async fn assistant_proactivity_set(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, IpcError> {
    Ok(state.orchestrator.set_assistant_proactivity(enabled)?)
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
