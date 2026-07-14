//! Built-in MCP server status + external-client registration for the UI.
//! The bearer token is intentionally absent from this surface — the frontend
//! never sees it.

use serde::Serialize;
use tauri::State;

use podium_core::mcp::install;

use crate::error::IpcError;
use crate::state::AppState;

/// Status snapshot of the built-in MCP server (token-free by design).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub running: bool,
    /// Full endpoint URL (e.g. `http://127.0.0.1:49152/mcp`) when running.
    pub url: Option<String>,
}

#[tauri::command]
pub async fn mcp_status(state: State<'_, AppState>) -> Result<McpStatus, IpcError> {
    let guard = state
        .mcp
        .lock()
        .map_err(|_| IpcError::new("io", "mcp state lock poisoned"))?;
    Ok(match guard.as_ref() {
        Some(server) => McpStatus {
            running: true,
            url: Some(format!("{}/mcp", server.url())),
        },
        None => McpStatus {
            running: false,
            url: None,
        },
    })
}

/// One external MCP client Podium can register its stdio bridge with.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientInfo {
    /// Stable identifier (e.g. `"claude-code"`).
    pub id: String,
    pub display_name: String,
    /// Whether the client's CLI resolves on the login-shell PATH.
    pub cli_available: bool,
    /// Whether the `podium` server entry is currently registered.
    pub installed: bool,
    /// The registration command line, for display / manual copy-paste.
    pub install_command: String,
    /// The CLI command that lists registered servers, for the card hint
    /// (e.g. `claude mcp list` / `auggie mcp list`).
    pub check_command: String,
}

fn current_exe() -> Result<std::path::PathBuf, IpcError> {
    std::env::current_exe()
        .map_err(|e| IpcError::new("io", format!("cannot resolve app path: {e}")))
}

/// List external MCP clients and their registration state. Probing shells
/// out via the login shell, so it runs on a blocking thread.
#[tauri::command]
pub async fn mcp_clients_status() -> Result<Vec<McpClientInfo>, IpcError> {
    let exe = current_exe()?;
    tauri::async_runtime::spawn_blocking(move || {
        install::CLIENTS
            .iter()
            .map(|client| {
                let status = client.status();
                Ok(McpClientInfo {
                    id: client.id.to_string(),
                    display_name: client.display_name.to_string(),
                    cli_available: status.cli_available,
                    installed: status.installed,
                    install_command: client.add_command(&exe).map_err(IpcError::from)?,
                    check_command: client.check_command().to_string(),
                })
            })
            .collect::<Result<Vec<_>, IpcError>>()
    })
    .await
    .map_err(|e| IpcError::new("io", format!("client probe task failed: {e}")))?
}

/// Register the stdio bridge with an external client (replacing any stale
/// entry). Returns the refreshed client list.
#[tauri::command]
pub async fn mcp_client_install(client_id: String) -> Result<Vec<McpClientInfo>, IpcError> {
    let client = install::client(&client_id)
        .ok_or_else(|| IpcError::new("invalidInput", "unknown MCP client"))?;
    let exe = current_exe()?;
    tauri::async_runtime::spawn_blocking(move || client.install(&exe))
        .await
        .map_err(|e| IpcError::new("io", format!("client install task failed: {e}")))?
        .map_err(IpcError::from)?;
    mcp_clients_status().await
}
