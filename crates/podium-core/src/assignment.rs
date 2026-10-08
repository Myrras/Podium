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
    /// agent that owns it. `None` once the agent exited (it gets a new one on
    /// restart).
    pub color: Option<String>,
    /// Whether the owning agent is running. An exited agent keeps the link so
    /// a restart picks the work back up, but it no longer *owns* the item:
    /// another agent may be put on it (spawn or `assign_todo`), which takes
    /// it over.
    pub active: bool,
}
