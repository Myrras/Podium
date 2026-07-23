//! The agent-assignment snapshot shared by to-dos and scratchpads.
//!
//! Both domains let the orchestrator track which agent process is currently
//! working on which item (a to-do or a scratchpad); the runtime-only result
//! shape is identical, so it lives here rather than being duplicated in
//! `todo.rs` and `scratchpad.rs`.

use serde::Serialize;

use crate::ids::ProcessId;

/// The agent currently working on a to-do or scratchpad. Runtime-only (a
/// `ProcessId` is per-run), so it is never persisted and is filled in at
/// list time.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignedAgent {
    /// The agent process working on the item.
    pub process_id: ProcessId,
    /// The agent's display name, for showing in the UI without a lookup.
    pub name: String,
    /// The session's subtle UI colour, so the item can be tinted to match the
    /// agent that owns it. `None` for legacy/edge cases where no colour was set.
    pub color: Option<String>,
}
