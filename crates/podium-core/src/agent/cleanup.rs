//! Automatic cleanup of exited agents.
//!
//! An agent that finished, crashed or was stopped stays in the list — its
//! row, its terminal, its to-do links — so the user (or the assistant) can
//! read what it did and restart it. Left alone, those rows pile up. This
//! sweep removes an agent once it has been exited for
//! `AgentSettings.remove_exited_agents_after_minutes` (default 5, `0` =
//! never), through [`Orchestrator::remove_exited_agents`], which is the same
//! removal as `remove_process`. Terminals and services are never touched.
//!
//! The delay runs from the agent's latest exit: a restart within it puts the
//! agent back to running (the sweep skips it), and its next exit starts the
//! clock again. The setting is read on every tick, so a change applies to
//! the next sweep without a restart.
//!
//! The assistant's exit nudge is published with the exit itself, minutes
//! before this removal, so it is never lost to it.

use std::sync::Arc;
use std::time::Duration;

use crate::orchestrator::Orchestrator;

/// Timings of the sweep. Injectable so tests run in milliseconds (same
/// spirit as [`crate::ProactivityConfig`]).
#[derive(Debug, Clone)]
pub struct AgentCleanupConfig {
    /// How often the sweep runs. An agent goes at most this long after its
    /// delay is up.
    pub tick: Duration,
    /// What one "minute" of the setting is worth: real time in the app,
    /// milliseconds in tests.
    pub minute: Duration,
}

impl Default for AgentCleanupConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(15),
            minute: Duration::from_secs(60),
        }
    }
}

/// Start sweeping. The task holds only a weak reference, so it ends by
/// itself when the app drops the orchestrator.
pub fn spawn(orchestrator: &Arc<Orchestrator>, config: AgentCleanupConfig) {
    let weak = Arc::downgrade(orchestrator);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.tick);
        loop {
            ticker.tick().await;
            let Some(orchestrator) = weak.upgrade() else {
                return;
            };
            sweep(&orchestrator, &config);
        }
    });
}

/// One sweep: read the setting fresh, then remove whatever is due. Returns
/// how many agents it removed.
fn sweep(orchestrator: &Orchestrator, config: &AgentCleanupConfig) -> usize {
    let Some(after) = orchestrator
        .agent_settings()
        .remove_exited_agents_after(config.minute)
    else {
        return 0;
    };
    let removed = orchestrator.remove_exited_agents(after);
    if !removed.is_empty() {
        tracing::debug!(count = removed.len(), "removed exited agents");
    }
    removed.len()
}
