//! Application state shared across all IPC commands.
//!
//! One [`Orchestrator`] lives for the lifetime of the app, behind an
//! [`AppState`] handed to Tauri via `.manage(...)` and pulled into commands as
//! `State<'_, AppState>`. The `Arc` lets detached tasks (the event forwarder,
//! per-attach chunk pumps, the exit hook) hold the orchestrator without
//! borrowing Tauri state.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use podium_core::{McpServer, Orchestrator};

/// Live application state: the podium-core orchestrator and the built-in
/// MCP server handle.
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    /// `None` until the setup hook finishes starting the server (or forever,
    /// if it failed to start). The handle holds the bearer token — it never
    /// leaves the backend.
    pub mcp: Mutex<Option<McpServer>>,
    /// Set once the user confirms closing despite active agents/terminals, so
    /// the close/exit guard lets the next attempt through instead of showing
    /// the warning again (see `commands::window` and `lib.rs`).
    pub force_close: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            orchestrator: Arc::new(Orchestrator::new()),
            mcp: Mutex::new(None),
            force_close: AtomicBool::new(false),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
