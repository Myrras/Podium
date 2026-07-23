/**
 * Typed wrappers over every Tauri IPC command.
 *
 * All command rejections are `IpcError` shapes from the Rust side;
 * `toIpcError` normalizes anything else (bridge failures, plugin errors)
 * into the same shape so callers have exactly one error type to handle.
 */

import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  AdapterInfo,
  AgentSettingsDto,
  AgentSpawnOptions,
  CommentId,
  IpcError,
  LinkId,
  McpClientInfo,
  McpStatus,
  MergeMode,
  NewProcess,
  ProcessId,
  ProcessInfo,
  ProjectId,
  ProjectInfo,
  RecentProject,
  ScratchpadId,
  ScratchpadInfo,
  TermEvent,
  TodoId,
  TodoInfo,
  WorktreeInfo,
} from "./types";

/** Narrow an unknown rejection to the structured `IpcError` shape. */
export function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as IpcError).message === "string" &&
    typeof (value as IpcError).kind === "string"
  );
}

/** Coerce any rejection into an `IpcError` (kind `"unknown"` as fallback). */
export function toIpcError(value: unknown): IpcError {
  if (isIpcError(value)) return value;
  if (value instanceof Error)
    return { message: value.message, kind: "unknown" };
  return { message: String(value), kind: "unknown" };
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

/** Open the directory at `path` (absolute) as a project. */
export function projectOpen(path: string): Promise<ProjectInfo> {
  return invoke("project_open", { path });
}

/** Close a project, stopping and removing all of its processes. */
export function projectClose(projectId: ProjectId): Promise<void> {
  return invoke("project_close", { projectId });
}

export function projectList(): Promise<ProjectInfo[]> {
  return invoke("project_list");
}

/** Re-read `podium.yml` for a project; returns the updated snapshot. */
export function projectConfigReload(
  projectId: ProjectId,
): Promise<ProjectInfo> {
  return invoke("project_config_reload", { projectId });
}

/**
 * Rename a project. A blank/undefined `name` clears the override, reverting
 * to the `podium.yml`/folder name. Returns the updated snapshot.
 */
export function projectRename(
  projectId: ProjectId,
  name: string | null,
): Promise<ProjectInfo> {
  return invoke("project_rename", { projectId, name });
}

/**
 * Reorder the sidebar project list to match `ordered` (project ids in the
 * desired order). Returns the projects in the new order.
 */
export function projectReorder(ordered: ProjectId[]): Promise<ProjectInfo[]> {
  return invoke("project_reorder", { ordered });
}

// ---------------------------------------------------------------------------
// Recent projects
// ---------------------------------------------------------------------------

/** Recently opened projects, most recent first (capped at 20). */
export function recentsList(): Promise<RecentProject[]> {
  return invoke("recents_list");
}

/** Remove one recents entry by path; returns the updated list. */
export function recentsRemove(path: string): Promise<RecentProject[]> {
  return invoke("recents_remove", { path });
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

/** Ordered absolute root paths of the persisted workspace projects. */
export function workspaceList(): Promise<string[]> {
  return invoke("workspace_list");
}

/** Remove one workspace entry by path; returns the updated list. */
export function workspaceRemove(path: string): Promise<string[]> {
  return invoke("workspace_remove", { path });
}

// ---------------------------------------------------------------------------
// Processes
// ---------------------------------------------------------------------------

export function processAdd(
  projectId: ProjectId,
  spec: NewProcess,
): Promise<ProcessInfo> {
  return invoke("process_add", { projectId, spec });
}

/** Remove a process, stopping it first if it is still running. */
export function processRemove(processId: ProcessId): Promise<void> {
  return invoke("process_remove", { processId });
}

/** List processes, optionally filtered to one project. */
export function processList(projectId?: ProjectId): Promise<ProcessInfo[]> {
  return invoke("process_list", { projectId: projectId ?? null });
}

/**
 * The git branch checked out in the process's working directory, or `null`
 * when its cwd is not a git repo / detached. Fetched on demand for the
 * focused process (shells out to git).
 */
export function processGitBranch(processId: ProcessId): Promise<string | null> {
  return invoke("process_git_branch", { processId });
}

/** Rename a process's display label; blank names are rejected. */
export function processRename(
  processId: ProcessId,
  name: string,
): Promise<ProcessInfo> {
  return invoke("process_rename", { processId, name });
}

export function processStart(processId: ProcessId): Promise<void> {
  return invoke("process_start", { processId });
}

/** Graceful stop: SIGTERM now, SIGKILL after a grace period. */
export function processStop(processId: ProcessId): Promise<void> {
  return invoke("process_stop", { processId });
}

/** Stop (if running), wait for exit, then start again. */
export function processRestart(processId: ProcessId): Promise<void> {
  return invoke("process_restart", { processId });
}

/** Write raw bytes (base64) to the process's stdin. */
export function processWrite(
  processId: ProcessId,
  dataB64: string,
): Promise<void> {
  return invoke("process_write", { processId, dataB64 });
}

export function processResize(
  processId: ProcessId,
  cols: number,
  rows: number,
): Promise<void> {
  return invoke("process_resize", { processId, cols, rows });
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

/** List the supported agent adapters (probes binary availability). */
export function adaptersList(): Promise<AdapterInfo[]> {
  return invoke("adapters_list");
}

/** Spawn (add + immediately start) an agent in a project. */
export function agentSpawn(
  projectId: ProjectId,
  options: AgentSpawnOptions = {},
): Promise<ProcessInfo> {
  return invoke("agent_spawn", {
    projectId,
    adapterId: options.adapterId ?? null,
    name: options.name ?? null,
    prompt: options.prompt ?? null,
    todoIds: options.todoIds ?? null,
    scratchpadIds: options.scratchpadIds ?? null,
    worktree: options.worktree ?? null,
    worktreeName: options.worktreeName ?? null,
    worktreeOnHead: options.worktreeOnHead ?? null,
    args: options.args ?? null,
  });
}

/** Global agent settings (command override + default args) + adapter catalog. */
export function agentSettingsGet(): Promise<AgentSettingsDto> {
  return invoke("agent_settings_get");
}

/**
 * Set (or clear) one adapter's global command override + default args. A blank
 * `command` clears the override. Returns the refreshed settings.
 */
export function agentSettingsSetAdapter(
  adapterId: string,
  command: string,
  defaultArgs: string[],
): Promise<AgentSettingsDto> {
  return invoke("agent_settings_set_adapter", {
    adapterId,
    command: command.trim() || null,
    defaultArgs,
  });
}

/**
 * Set (or clear) the global default adapter used by bare spawns. A blank id
 * clears it (back to the built-in default). Returns the refreshed settings.
 */
export function agentSettingsSetDefaultAdapter(
  adapterId: string,
): Promise<AgentSettingsDto> {
  return invoke("agent_settings_set_default_adapter", {
    adapterId: adapterId || null,
  });
}

/** Set how global default args combine with a project's `agents.extra_args`. */
export function agentSettingsSetMergeMode(
  mode: MergeMode,
): Promise<AgentSettingsDto> {
  return invoke("agent_settings_set_merge_mode", { mode });
}

/**
 * Toggle whether agents are asked to offer a git worktree before modifying
 * code. Returns the refreshed settings.
 */
export function agentSettingsSetSuggestWorktree(
  enabled: boolean,
): Promise<AgentSettingsDto> {
  return invoke("agent_settings_set_suggest_worktree", { enabled });
}

// ---------------------------------------------------------------------------
// Worktrees
// ---------------------------------------------------------------------------

/** List a project's Podium-managed git worktrees. */
export function worktreeList(projectId: ProjectId): Promise<WorktreeInfo[]> {
  return invoke("worktree_list", { projectId });
}

/**
 * Remove a Podium-managed git worktree; returns the refreshed list. Rejects
 * with kind `"worktreeInUse"` while a process runs in it, and with
 * `"worktreeDirty"` when it has uncommitted changes and `force` is false.
 */
export function worktreeRemove(
  projectId: ProjectId,
  name: string,
  force: boolean,
): Promise<WorktreeInfo[]> {
  return invoke("worktree_remove", { projectId, name, force });
}

// ---------------------------------------------------------------------------
// To-dos
// ---------------------------------------------------------------------------

/** List a project's active (non-archived) to-dos in creation order. */
export function todoList(projectId: ProjectId): Promise<TodoInfo[]> {
  return invoke("todo_list", { projectId });
}

/** List a project's archived to-dos, most recently archived first. */
export function todoListArchived(projectId: ProjectId): Promise<TodoInfo[]> {
  return invoke("todo_list_archived", { projectId });
}

/** Archive or unarchive a to-do; returns the updated snapshot. */
export function todoSetArchived(
  projectId: ProjectId,
  todoId: TodoId,
  archived: boolean,
): Promise<TodoInfo> {
  return invoke("todo_set_archived", { projectId, todoId, archived });
}

/** Add a to-do to a project; blank text is rejected. */
export function todoAdd(projectId: ProjectId, text: string): Promise<TodoInfo> {
  return invoke("todo_add", { projectId, text });
}

/** Mark a to-do as done / not done; returns the updated snapshot. */
export function todoSetDone(
  projectId: ProjectId,
  todoId: TodoId,
  done: boolean,
): Promise<TodoInfo> {
  return invoke("todo_set_done", { projectId, todoId, done });
}

/**
 * Revise a to-do's text and/or description. Pass `undefined` to leave a
 * field unchanged; a blank `description` clears it.
 */
export function todoUpdate(
  projectId: ProjectId,
  todoId: TodoId,
  changes: { text?: string; description?: string },
): Promise<TodoInfo> {
  return invoke("todo_update", {
    projectId,
    todoId,
    text: changes.text ?? null,
    description: changes.description ?? null,
  });
}

/** Append a progress note to a to-do (author defaults to "You"). */
export function todoComment(
  projectId: ProjectId,
  todoId: TodoId,
  text: string,
  author?: string,
): Promise<TodoInfo> {
  return invoke("todo_comment", {
    projectId,
    todoId,
    text,
    author: author ?? null,
  });
}

/** Revise an existing comment's text; blank text is rejected. */
export function todoCommentUpdate(
  projectId: ProjectId,
  todoId: TodoId,
  commentId: CommentId,
  text: string,
): Promise<TodoInfo> {
  return invoke("todo_comment_update", {
    projectId,
    todoId,
    commentId,
    text,
  });
}

/** Remove a comment from a to-do; returns the updated snapshot. */
export function todoCommentRemove(
  projectId: ProjectId,
  todoId: TodoId,
  commentId: CommentId,
): Promise<TodoInfo> {
  return invoke("todo_comment_remove", { projectId, todoId, commentId });
}

/**
 * Pin an issue/PR link to a to-do; a blank `label` falls back to the url, and
 * the url must be http(s). Returns the updated to-do.
 */
export function todoAddLink(
  projectId: ProjectId,
  todoId: TodoId,
  url: string,
  label?: string,
): Promise<TodoInfo> {
  return invoke("todo_add_link", {
    projectId,
    todoId,
    url,
    label: label?.trim() || null,
  });
}

/** Remove a pinned link from a to-do; returns the updated snapshot. */
export function todoRemoveLink(
  projectId: ProjectId,
  todoId: TodoId,
  linkId: LinkId,
): Promise<TodoInfo> {
  return invoke("todo_remove_link", { projectId, todoId, linkId });
}

/** Remove a to-do from a project. */
export function todoRemove(
  projectId: ProjectId,
  todoId: TodoId,
): Promise<void> {
  return invoke("todo_remove", { projectId, todoId });
}

/**
 * Unassign a to-do from its agent (the sidebar (x) action). Sends a
 * best-effort cancel/rollback request to the agent's stdin first, then clears
 * the link. Returns the updated to-do.
 */
export function todoUnassign(
  projectId: ProjectId,
  todoId: TodoId,
): Promise<TodoInfo> {
  return invoke("todo_unassign", { projectId, todoId });
}

// ---------------------------------------------------------------------------
// Scratchpads
// ---------------------------------------------------------------------------

/** List a project's active (non-archived) scratchpads. */
export function scratchpadList(
  projectId: ProjectId,
): Promise<ScratchpadInfo[]> {
  return invoke("scratchpad_list", { projectId });
}

/** List a project's archived scratchpads, most recently archived first. */
export function scratchpadListArchived(
  projectId: ProjectId,
): Promise<ScratchpadInfo[]> {
  return invoke("scratchpad_list_archived", { projectId });
}

/** Create a new scratchpad (auto-generated timestamp title, empty content). */
export function scratchpadAdd(projectId: ProjectId): Promise<ScratchpadInfo> {
  return invoke("scratchpad_add", { projectId });
}

/**
 * Replace a scratchpad's content (bumps its version). `expectedUpdatedAt`
 * must be the exact `updatedAt` string last seen for this scratchpad — echo
 * it back verbatim (never reparse/reformat it) or the timestamp comparison
 * on the backend will never match. Rejects with
 * `IpcError.kind === "scratchpadConflict"` if someone else edited it first.
 */
export function scratchpadUpdateContent(
  projectId: ProjectId,
  id: ScratchpadId,
  content: string,
  expectedUpdatedAt: string,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_update_content", {
    projectId,
    id,
    content,
    expectedUpdatedAt,
  });
}

/**
 * Revise a scratchpad's title (blank falls back to a timestamp title).
 * `expectedUpdatedAt` is checked the same way as
 * {@link scratchpadUpdateContent}.
 */
export function scratchpadUpdateTitle(
  projectId: ProjectId,
  id: ScratchpadId,
  title: string,
  expectedUpdatedAt: string,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_update_title", {
    projectId,
    id,
    title,
    expectedUpdatedAt,
  });
}

/** Add a free-text tag to a scratchpad (blank rejected, dedup by value). */
export function scratchpadAddTag(
  projectId: ProjectId,
  id: ScratchpadId,
  tag: string,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_add_tag", { projectId, id, tag });
}

/** Remove a tag from a scratchpad by exact value (idempotent). */
export function scratchpadRemoveTag(
  projectId: ProjectId,
  id: ScratchpadId,
  tag: string,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_remove_tag", { projectId, id, tag });
}

/** Archive or unarchive a scratchpad. */
export function scratchpadSetArchived(
  projectId: ProjectId,
  id: ScratchpadId,
  archived: boolean,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_set_archived", { projectId, id, archived });
}

/** Permanently remove a scratchpad (from the Archive modal). */
export function scratchpadRemove(
  projectId: ProjectId,
  id: ScratchpadId,
): Promise<void> {
  return invoke("scratchpad_remove", { projectId, id });
}

/**
 * Unassign a scratchpad from its agent (the sidebar (x) action). Sends a
 * best-effort cancel/rollback request to the agent's stdin first, then clears
 * the link. Returns the updated scratchpad.
 */
export function scratchpadUnassign(
  projectId: ProjectId,
  id: ScratchpadId,
): Promise<ScratchpadInfo> {
  return invoke("scratchpad_unassign", { projectId, id });
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/**
 * Confirm quitting despite active agents/terminals: the backend arms its
 * force-close flag and exits the app (running the normal shutdown). Called
 * from the close-warning dialog's "Close anyway" action.
 */
export function windowConfirmClose(): Promise<void> {
  return invoke("window_confirm_close");
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

/** Status of the built-in MCP server (never includes the bearer token). */
export function mcpStatus(): Promise<McpStatus> {
  return invoke("mcp_status");
}

/** External MCP clients and whether Podium's bridge is registered in each. */
export function mcpClientsStatus(): Promise<McpClientInfo[]> {
  return invoke("mcp_clients_status");
}

/** Register the bridge with a client; returns the refreshed client list. */
export function mcpClientInstall(clientId: string): Promise<McpClientInfo[]> {
  return invoke("mcp_client_install", { clientId });
}

/**
 * Attach to a process's output stream. `onEvent` receives one `snapshot`,
 * then batched `data` events; a `lagged` event means bytes were lost and the
 * caller should re-attach. Returns the channel — keep it referenced while
 * attached (dropping it ends the stream server-side on the next send).
 */
export async function processAttach(
  processId: ProcessId,
  onEvent: (event: TermEvent) => void,
): Promise<Channel<TermEvent>> {
  const channel = new Channel<TermEvent>();
  channel.onmessage = onEvent;
  await invoke("process_attach", { processId, channel });
  return channel;
}
