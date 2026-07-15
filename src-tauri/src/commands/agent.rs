//! Agent commands: adapter discovery, one-shot agent spawning, and the
//! global agent settings (Settings → Agents).

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use podium_core::{
    AdapterInfo, MergeMode, Orchestrator, ProcessInfo, ProjectId, ScratchpadId, TodoId,
};

use crate::error::IpcError;
use crate::state::AppState;

/// List the supported agent adapters. Availability probing shells out
/// (`command -v` via the login shell), so it runs on a blocking thread.
#[tauri::command]
pub async fn adapters_list(state: State<'_, AppState>) -> Result<Vec<AdapterInfo>, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || orchestrator.list_adapters())
        .await
        .map_err(|e| IpcError::new("io", format!("adapter probe task failed: {e}")))
}

/// Spawn (add + immediately start) an agent in a project. Blank `name` /
/// `prompt` are treated as absent; the core picks a free default name.
/// `todo_ids` seeds the agent's prompt with one or more to-dos to work on
/// (multiple are handed over as one combined task). `scratchpad_ids` does the
/// same for scratchpads, and is only used when `todo_ids` is empty.
#[tauri::command]
pub async fn agent_spawn(
    state: State<'_, AppState>,
    project_id: ProjectId,
    adapter_id: Option<String>,
    name: Option<String>,
    prompt: Option<String>,
    todo_ids: Option<Vec<TodoId>>,
    scratchpad_ids: Option<Vec<ScratchpadId>>,
) -> Result<ProcessInfo, IpcError> {
    let id = state
        .orchestrator
        .spawn_agent(
            project_id,
            adapter_id,
            name,
            prompt,
            todo_ids.unwrap_or_default(),
            scratchpad_ids.unwrap_or_default(),
        )
        .await?;
    state
        .orchestrator
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IpcError::new("processNotFound", "process vanished after spawn"))
}

/// One adapter row for the Settings → Agents tab: the adapter catalog entry
/// merged with its stored global override (if any).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAdapterConfig {
    pub id: String,
    pub display_name: String,
    pub available: bool,
    /// The adapter's built-in binary (the placeholder / default command).
    pub binary: String,
    /// Global command override, or empty when unset.
    pub command: String,
    /// Global default CLI arguments applied whenever this agent starts.
    pub default_args: Vec<String>,
}

/// Everything the Agents settings tab needs in one call.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettingsDto {
    pub merge_mode: MergeMode,
    /// Global default adapter for bare spawns; empty = built-in default.
    pub default_adapter: String,
    pub adapters: Vec<AgentAdapterConfig>,
}

/// Merge the adapter catalog (probes binary availability) with the stored
/// global overrides. Shells out, so callers run it on a blocking thread.
fn build_settings_dto(orchestrator: &Orchestrator) -> AgentSettingsDto {
    let settings = orchestrator.agent_settings();
    let adapters = orchestrator
        .list_adapters()
        .into_iter()
        .map(|a| {
            let ov = settings.override_for(&a.id);
            AgentAdapterConfig {
                id: a.id,
                display_name: a.display_name,
                available: a.available,
                binary: a.binary,
                command: ov.and_then(|o| o.command.clone()).unwrap_or_default(),
                default_args: ov.map(|o| o.default_args.clone()).unwrap_or_default(),
            }
        })
        .collect();
    AgentSettingsDto {
        merge_mode: settings.merge_mode,
        default_adapter: settings.default_adapter.unwrap_or_default(),
        adapters,
    }
}

/// The global agent settings plus the adapter catalog, for the settings tab.
#[tauri::command]
pub async fn agent_settings_get(state: State<'_, AppState>) -> Result<AgentSettingsDto, IpcError> {
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || build_settings_dto(&orchestrator))
        .await
        .map_err(|e| IpcError::new("io", format!("adapter probe task failed: {e}")))
}

/// Set (or clear) one adapter's global command override + default args; blank
/// `command` clears the override. Returns the refreshed settings.
#[tauri::command]
pub async fn agent_settings_set_adapter(
    state: State<'_, AppState>,
    adapter_id: String,
    command: Option<String>,
    default_args: Vec<String>,
) -> Result<AgentSettingsDto, IpcError> {
    state
        .orchestrator
        .set_agent_override(&adapter_id, command, default_args)?;
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || build_settings_dto(&orchestrator))
        .await
        .map_err(|e| IpcError::new("io", format!("adapter probe task failed: {e}")))
}

/// Set (or clear) the global default adapter used by bare spawns; a blank id
/// clears it (back to the built-in default). Returns the refreshed settings.
#[tauri::command]
pub async fn agent_settings_set_default_adapter(
    state: State<'_, AppState>,
    adapter_id: Option<String>,
) -> Result<AgentSettingsDto, IpcError> {
    state.orchestrator.set_agent_default_adapter(adapter_id)?;
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || build_settings_dto(&orchestrator))
        .await
        .map_err(|e| IpcError::new("io", format!("adapter probe task failed: {e}")))
}

/// Set how global default args combine with a project's `agents.extra_args`.
/// Returns the refreshed settings.
#[tauri::command]
pub async fn agent_settings_set_merge_mode(
    state: State<'_, AppState>,
    mode: MergeMode,
) -> Result<AgentSettingsDto, IpcError> {
    state.orchestrator.set_agent_merge_mode(mode)?;
    let orchestrator = Arc::clone(&state.orchestrator);
    tauri::async_runtime::spawn_blocking(move || build_settings_dto(&orchestrator))
        .await
        .map_err(|e| IpcError::new("io", format!("adapter probe task failed: {e}")))
}
