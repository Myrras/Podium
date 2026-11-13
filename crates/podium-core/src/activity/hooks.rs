//! What an agent's hooks report about the work running behind it.
//!
//! Stub: the [`Store`] holds nothing yet.

use crate::ids::ProcessId;

/// How many sub-agents and background shells an agent has running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub subagents: u32,
    pub shells: u32,
}

/// The counts each agent's hooks last reported. The orchestrator holds one
/// ([`crate::Orchestrator::hooks`]).
// Braces, not a unit struct: the orchestrator builds it with `default()`,
// which keeps working once this holds something.
#[derive(Debug, Default)]
pub struct Store {}

impl Store {
    /// The counts for agent `id`, or `None` when its hooks reported none.
    pub fn counts(&self, _id: ProcessId) -> Option<Counts> {
        None
    }
}
