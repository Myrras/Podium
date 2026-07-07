//! Window lifecycle commands.
//!
//! The close/exit guard (see `lib.rs`) prevents the app from quitting while
//! agents or terminals are still running and asks the webview to confirm. When
//! the user confirms, the frontend calls [`window_confirm_close`], which sets
//! the force-close flag so the guard steps aside, then exits the app (running
//! the normal `RunEvent::Exit` shutdown that stops every process group).

use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::state::AppState;

/// Confirm closing despite active agents/terminals: arm the force-close flag
/// and exit the app. Exiting triggers the standard shutdown, so no processes
/// are leaked.
#[tauri::command]
pub fn window_confirm_close(app: AppHandle, state: State<'_, AppState>) {
    state.force_close.store(true, Ordering::SeqCst);
    app.exit(0);
}
