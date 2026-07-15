//! Core domain events and the broadcast bus they travel on.

use serde::Serialize;
use tokio::sync::broadcast;

use crate::ids::{ProcessId, ProjectId};
use crate::process::ProcessStatus;

const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Events emitted by the [`crate::Orchestrator`] for UI layers to react to.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PodiumEvent {
    ProcessAdded {
        project_id: ProjectId,
        process_id: ProcessId,
    },
    ProcessRemoved {
        project_id: ProjectId,
        process_id: ProcessId,
    },
    ProcessStatusChanged {
        project_id: ProjectId,
        process_id: ProcessId,
        status: ProcessStatus,
    },
    /// A process's metadata changed (e.g. it was renamed); the UI re-pulls
    /// the process list via `list_processes`.
    ProcessUpdated {
        project_id: ProjectId,
        process_id: ProcessId,
    },
    ProjectOpened {
        project_id: ProjectId,
    },
    /// Project metadata changed (e.g. `podium.yml` was reloaded).
    ProjectUpdated {
        project_id: ProjectId,
    },
    ProjectClosed {
        project_id: ProjectId,
    },
    /// A project's to-do list changed (add / done-toggle / remove); the UI
    /// re-pulls the list via `list_todos`.
    TodosChanged {
        project_id: ProjectId,
    },
    /// A project's scratchpads changed (add / content or title update); the
    /// UI re-pulls the list via `list_scratchpads`.
    ScratchpadsChanged {
        project_id: ProjectId,
    },
}

/// Fan-out bus for [`PodiumEvent`]s, backed by a tokio broadcast channel.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<PodiumEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Publish an event; silently drops it when there are no subscribers.
    pub fn publish(&self, event: PodiumEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PodiumEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
