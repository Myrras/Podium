//! Process commands: CRUD, lifecycle, stdin/resize, and the attach stream.
//!
//! ## Attach protocol
//! `process_attach` sends one [`TermEvent::Snapshot`] (the full scrollback,
//! base64) over the provided channel, then streams [`TermEvent::Data`]
//! batches. Output is batched for up to [`BATCH_INTERVAL`] or until
//! [`BATCH_MAX_BYTES`] accumulate, so a chatty child cannot flood the IPC
//! bridge. `seq` numbers come from the core's scrollback (one per raw PTY
//! chunk): the snapshot's `seq` is the first live seq that follows it, and a
//! `Data` batch's `seq` is the seq of its first chunk — the frontend drops
//! any batch whose `seq` is below the snapshot's to avoid double-writes.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::MissedTickBehavior;

use podium_core::{
    ProcessId, ProcessInfo, ProcessKind, ProcessSpec, ProjectId, RestartPolicy, TermChunk,
};

use crate::error::IpcError;
use crate::state::AppState;

/// Max time output may sit in the batch buffer before being flushed.
const BATCH_INTERVAL: Duration = Duration::from_millis(16);
/// Flush immediately once this many bytes are buffered.
const BATCH_MAX_BYTES: usize = 64 * 1024;

/// Default command for terminal processes: replace the wrapper `$SHELL -lc`
/// with an interactive shell so the user gets prompts, job control, etc.
const TERMINAL_DEFAULT_COMMAND: &str = "exec \"$SHELL\" -i";

/// Frontend payload for creating a process. `kind` is flattened, so the wire
/// shape is `{ name, command?, cwd?, kind: "service"|"terminal"|"agent",
/// adapter?, restartPolicy? }`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProcess {
    pub name: String,
    /// Shell command line; optional for terminals (interactive shell default).
    #[serde(default)]
    pub command: Option<String>,
    /// Working directory *relative to the project root* (defaults to the root).
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(flatten)]
    pub kind: ProcessKind,
    #[serde(default)]
    pub restart_policy: RestartPolicy,
}

/// One message on a `process_attach` channel.
#[derive(Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TermEvent {
    /// Full scrollback; `seq` is the seq of the first live chunk after it.
    Snapshot { seq: u64, data_b64: String },
    /// Batched live output; `seq` is the seq of the batch's first chunk.
    Data { seq: u64, data_b64: String },
    /// The stream lost data (broadcast lag); the frontend must re-attach.
    Lagged,
}

#[tauri::command]
pub async fn process_add(
    state: State<'_, AppState>,
    project_id: ProjectId,
    spec: NewProcess,
) -> Result<ProcessInfo, IpcError> {
    if spec.name.trim().is_empty() {
        return Err(IpcError::invalid_input("process name must not be empty"));
    }
    let root = state
        .orchestrator
        .list_projects()
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or(podium_core::CoreError::ProjectNotFound)?
        .root;
    // Core's guard: rejects absolute paths and traversal out of the root.
    let cwd = podium_core::project::resolve_cwd(&root, spec.cwd.as_deref())?;
    let command = match spec.command.filter(|c| !c.trim().is_empty()) {
        Some(c) => c,
        None if matches!(spec.kind, ProcessKind::Terminal) => TERMINAL_DEFAULT_COMMAND.to_string(),
        None => {
            return Err(IpcError::invalid_input(
                "command is required for non-terminal processes",
            ))
        }
    };
    let core_spec = ProcessSpec {
        name: spec.name,
        command,
        cwd,
        env: Vec::new(),
        kind: spec.kind,
        restart_policy: spec.restart_policy,
    };
    let id = state
        .orchestrator
        .add_process(project_id, core_spec)
        .await?;
    state
        .orchestrator
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IpcError::new("processNotFound", "process vanished after add"))
}

#[tauri::command]
pub async fn process_remove(
    state: State<'_, AppState>,
    process_id: ProcessId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .remove_process(process_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub fn process_list(state: State<'_, AppState>, project_id: Option<ProjectId>) -> Vec<ProcessInfo> {
    state.orchestrator.list_processes(project_id)
}

/// Rename a process's display label (sidebar/window name). Does not restart
/// the process or change its command. Blank names are rejected.
#[tauri::command]
pub fn process_rename(
    state: State<'_, AppState>,
    process_id: ProcessId,
    name: String,
) -> Result<ProcessInfo, IpcError> {
    state
        .orchestrator
        .rename_process(process_id, &name)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn process_start(
    state: State<'_, AppState>,
    process_id: ProcessId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .start_process(process_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn process_stop(
    state: State<'_, AppState>,
    process_id: ProcessId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .stop_process(process_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn process_restart(
    state: State<'_, AppState>,
    process_id: ProcessId,
) -> Result<(), IpcError> {
    state
        .orchestrator
        .restart_process(process_id)
        .await
        .map_err(Into::into)
}

/// Write user keystrokes to the process's stdin. `data_b64` is base64 so raw
/// terminal bytes (control sequences, multibyte UTF-8) survive the JSON hop.
#[tauri::command]
pub async fn process_write(
    state: State<'_, AppState>,
    process_id: ProcessId,
    data_b64: String,
) -> Result<(), IpcError> {
    let bytes = BASE64
        .decode(data_b64.as_bytes())
        .map_err(|_| IpcError::invalid_input("malformed base64 stdin payload"))?;
    state
        .orchestrator
        .write_stdin(process_id, &bytes)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn process_resize(
    state: State<'_, AppState>,
    process_id: ProcessId,
    cols: u16,
    rows: u16,
) -> Result<(), IpcError> {
    if cols == 0 || rows == 0 {
        return Err(IpcError::invalid_input("cols and rows must be non-zero"));
    }
    state
        .orchestrator
        .resize(process_id, cols, rows)
        .await
        .map_err(Into::into)
}

/// Attach to a process's output: sends one `Snapshot`, then streams batched
/// `Data` events until the process is removed, the stream lags, or the
/// channel dies (webview gone / frontend re-attached and dropped this one).
#[tauri::command]
pub async fn process_attach(
    state: State<'_, AppState>,
    process_id: ProcessId,
    channel: Channel<TermEvent>,
) -> Result<(), IpcError> {
    let (snapshot, next_seq, rx) = state.orchestrator.attach(process_id).await?;
    channel
        .send(TermEvent::Snapshot {
            seq: next_seq,
            data_b64: BASE64.encode(&snapshot),
        })
        .map_err(|e| IpcError::new("io", format!("failed to send snapshot: {e}")))?;
    tauri::async_runtime::spawn(pump_chunks(rx, channel));
    Ok(())
}

/// Forward broadcast chunks to the channel, batching by time and size.
async fn pump_chunks(mut rx: broadcast::Receiver<TermChunk>, channel: Channel<TermEvent>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut batch_seq: u64 = 0;
    let mut flush_tick = tokio::time::interval(BATCH_INTERVAL);
    flush_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            chunk = rx.recv() => match chunk {
                Ok(TermChunk { seq, bytes }) => {
                    if buf.is_empty() {
                        batch_seq = seq;
                    }
                    buf.extend_from_slice(&bytes);
                    if buf.len() >= BATCH_MAX_BYTES && !flush(&channel, &mut buf, batch_seq) {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // Bytes were dropped; a partial stream would corrupt the
                    // terminal. Tell the frontend to re-attach for a clean
                    // snapshot and end this pump.
                    let _ = channel.send(TermEvent::Lagged);
                    return;
                }
                Err(RecvError::Closed) => {
                    // Process removed — flush the tail and end the stream.
                    flush(&channel, &mut buf, batch_seq);
                    return;
                }
            },
            _ = flush_tick.tick() => {
                if !buf.is_empty() && !flush(&channel, &mut buf, batch_seq) {
                    return;
                }
            }
        }
    }
}

/// Send the buffered bytes as one `Data` batch. Returns `false` when the
/// channel is dead (the pump should stop).
fn flush(channel: &Channel<TermEvent>, buf: &mut Vec<u8>, seq: u64) -> bool {
    if buf.is_empty() {
        return true;
    }
    let data_b64 = BASE64.encode(buf.as_slice());
    buf.clear();
    channel.send(TermEvent::Data { seq, data_b64 }).is_ok()
}
