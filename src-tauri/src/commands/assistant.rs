//! Personal-assistant commands: sending a turn, replaying the stored
//! transcript, starting over, its memory, the autonomy, proactivity and
//! message-queue settings, deciding on a proposed action, and managing the
//! pending-input queue (edit/remove a waiting message, interrupt the running
//! turn). Replies (and the outcome of a proposal, and queue changes) stream
//! back as global `assistant:event` events (see `events.rs`).

use tauri::State;

use podium_core::{Attachment, AutonomyMode, PendingId, ProposalId, TranscriptEntry};

use crate::error::IpcError;
use crate::state::AppState;

/// Send a chat turn to the assistant, spawning its headless session on first
/// use. `attachments` ride along per `podium_core::Attachment`: an image
/// travels inline (read from disk first when only a path was given), other
/// files pass their absolute path — or inline text when there is none — for
/// the assistant's own `Read` tool. An attachment over the limits is refused
/// on its own; the rest of the message still goes. The reply arrives as
/// `assistant:event` payloads, not as the return value.
#[tauri::command]
pub async fn assistant_send(
    state: State<'_, AppState>,
    text: String,
    attachments: Vec<Attachment>,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .assistant_send_with_attachments(&text, attachments)
        .await?;
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

/// Whether a message typed while a turn is running waits its turn (queued)
/// instead of going straight into the live session. Governs only the user's
/// own messages.
#[tauri::command]
pub async fn assistant_queue_messages_get(state: State<'_, AppState>) -> Result<bool, IpcError> {
    Ok(state.orchestrator.assistant_queue_messages())
}

/// Turn the message queue on or off; returns what was stored.
#[tauri::command]
pub async fn assistant_queue_messages_set(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<bool, IpcError> {
    Ok(state.orchestrator.set_assistant_queue_messages(enabled)?)
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

/// Change a waiting user message's text before it is delivered. Nudges and
/// proposal-outcome notices are not editable — they are Podium's own text.
#[tauri::command]
pub async fn assistant_queue_edit(
    state: State<'_, AppState>,
    id: PendingId,
    text: String,
) -> Result<(), IpcError> {
    state.orchestrator.assistant_queue_edit(id, &text).await?;
    Ok(())
}

/// Withdraw a waiting user message so nothing is delivered for it.
#[tauri::command]
pub async fn assistant_queue_remove(
    state: State<'_, AppState>,
    id: PendingId,
) -> Result<(), IpcError> {
    state.orchestrator.assistant_queue_remove(id).await?;
    Ok(())
}

/// Stop the assistant's current turn, if any, and immediately hand the next
/// waiting message over rather than for a `TurnEnd` that would never come.
/// With nothing running this is a no-op.
#[tauri::command]
pub async fn assistant_interrupt(state: State<'_, AppState>) -> Result<(), IpcError> {
    state.orchestrator.assistant_interrupt().await?;
    Ok(())
}
