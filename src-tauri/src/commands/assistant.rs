//! Personal-assistant commands: one send, replies stream back as global
//! `assistant:event` events (see `events.rs`).

use tauri::State;

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
