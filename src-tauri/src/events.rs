//! Forwards podium-core domain events to the webview as global Tauri events.
//!
//! These are the low-volume lifecycle notifications (status flips, add/remove,
//! open/close). High-volume terminal output never travels this path — it
//! streams over per-attach IPC channels (see `commands::process`).

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

use podium_core::{Orchestrator, PodiumEvent, ProcessId, ProcessStatus, ProjectId};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessRefPayload {
    project_id: ProjectId,
    process_id: ProcessId,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessStatusPayload {
    project_id: ProjectId,
    process_id: ProcessId,
    status: ProcessStatus,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRefPayload {
    project_id: ProjectId,
}

/// Spawn the long-lived task that re-emits core events to the frontend.
/// Runs for the whole app lifetime (the orchestrator never drops its bus).
pub fn spawn_forwarder(app: AppHandle, orchestrator: Arc<Orchestrator>) {
    let mut rx = orchestrator.subscribe_events();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => forward(&app, event),
                Err(RecvError::Lagged(skipped)) => {
                    // Lifecycle events are tiny and rare; lagging here means
                    // something is very wrong. The UI can resync via list_*.
                    tracing::warn!(skipped, "event bus lagged; dropped UI events");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn forward(app: &AppHandle, event: PodiumEvent) {
    let result = match event {
        PodiumEvent::ProcessAdded {
            project_id,
            process_id,
        } => app.emit(
            "process:added",
            ProcessRefPayload {
                project_id,
                process_id,
            },
        ),
        PodiumEvent::ProcessRemoved {
            project_id,
            process_id,
        } => app.emit(
            "process:removed",
            ProcessRefPayload {
                project_id,
                process_id,
            },
        ),
        PodiumEvent::ProcessStatusChanged {
            project_id,
            process_id,
            status,
        } => app.emit(
            "process:status",
            ProcessStatusPayload {
                project_id,
                process_id,
                status,
            },
        ),
        PodiumEvent::ProcessUpdated {
            project_id,
            process_id,
        } => app.emit(
            "process:updated",
            ProcessRefPayload {
                project_id,
                process_id,
            },
        ),
        PodiumEvent::ProjectOpened { project_id } => {
            app.emit("project:opened", ProjectRefPayload { project_id })
        }
        PodiumEvent::ProjectUpdated { project_id } => {
            app.emit("project:updated", ProjectRefPayload { project_id })
        }
        PodiumEvent::ProjectClosed { project_id } => {
            app.emit("project:closed", ProjectRefPayload { project_id })
        }
        PodiumEvent::TodosChanged { project_id } => {
            app.emit("todo:changed", ProjectRefPayload { project_id })
        }
        PodiumEvent::ScratchpadsChanged { project_id } => {
            app.emit("scratchpad:changed", ProjectRefPayload { project_id })
        }
    };
    if let Err(e) = result {
        tracing::warn!("failed to emit event to webview: {e}");
    }
}
