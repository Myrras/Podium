/**
 * Wire types for the Tauri IPC bridge.
 *
 * These mirror the serde shapes in `src-tauri` / `podium-core` exactly —
 * change them only in lockstep with the Rust side:
 * - Ids are plain UUID strings (`#[serde(transparent)]`).
 * - Structs are camelCase (`rename_all = "camelCase"`).
 * - `ProcessKind` is tagged by `kind`, `ProcessStatus` by `state`,
 *   `TermEvent` by `type`; `RestartPolicy` values are kebab-case.
 * - Timestamps are RFC 3339 strings (`chrono::DateTime<Utc>`).
 */

export type ProjectId = string;
export type ProcessId = string;
export type TodoId = string;
export type CommentId = string;
export type LinkId = string;
export type ScratchpadId = string;

/** Open project snapshot (`podium_core::ProjectInfo`). */
export interface ProjectInfo {
  id: ProjectId;
  name: string;
  /** Absolute path of the project root. */
  root: string;
  /** Sidebar badge initials (from `podium.yml` or derived from the name). */
  iconInitials: string;
  /** Readable `podium.yml` error, if the last (re)load failed. */
  configError: string | null;
  /** True when a user-set display-name override is in effect. */
  renamed: boolean;
}

/** One remembered project (`RecentProject` in `commands/recents.rs`). */
export interface RecentProject {
  /** Absolute path of the project root. */
  path: string;
  name: string;
  /** Unix time in milliseconds of the last successful open. */
  lastOpenedAt: number;
}

/** What kind of process this is (`podium_core::ProcessKind`). */
export type ProcessKind =
  | { kind: "service" }
  | { kind: "terminal" }
  | { kind: "agent"; adapter: string };

/** Lifecycle state (`podium_core::ProcessStatus`). */
export type ProcessStatus =
  | { state: "notStarted" }
  | { state: "running"; pid: number; since: string }
  | { state: "stopping" }
  | { state: "exited"; code: number | null; crashed: boolean; at: string };

/** Restart behaviour (`podium_core::RestartPolicy`, kebab-case values). */
export type RestartPolicy = "never" | "on-crash" | "always";

/** Managed process snapshot (`podium_core::ProcessInfo`). */
export interface ProcessInfo {
  id: ProcessId;
  projectId: ProjectId;
  name: string;
  kind: ProcessKind;
  status: ProcessStatus;
  restartPolicy: RestartPolicy;
  command: string;
}

/**
 * Payload for `process_add` (`NewProcess` in `commands/process.rs`).
 * `kind` is flattened on the Rust side, so `kind`/`adapter` sit at the top
 * level next to `name`. `command` may be omitted for terminals (the backend
 * defaults to an interactive shell); `cwd` is relative to the project root.
 */
export type NewProcess = {
  name: string;
  command?: string;
  cwd?: string;
  restartPolicy?: RestartPolicy;
} & ProcessKind;

/** One supported agent adapter (`podium_core::AdapterInfo`). */
export interface AdapterInfo {
  /** Stable id used in config and IPC, e.g. `"claude-code"`. */
  id: string;
  displayName: string;
  /** The adapter's built-in CLI binary (default command). */
  binary: string;
  /** Whether the adapter's CLI binary resolves on the login-shell PATH. */
  available: boolean;
}

/**
 * How global default args combine with a project's `agents.extra_args`
 * (`podium_core::MergeMode`, kebab-case values).
 */
export type MergeMode = "merge" | "project-overrides" | "global-overrides";

/**
 * One adapter row for the Settings → Agents tab: the adapter catalog entry
 * merged with its stored global override (`AgentAdapterConfig` in
 * `commands/agent.rs`).
 */
export interface AgentAdapterConfig {
  id: string;
  displayName: string;
  available: boolean;
  /** The adapter's built-in binary (placeholder / default command). */
  binary: string;
  /** Global command override, or `""` when unset. */
  command: string;
  /** Global default CLI arguments applied whenever this agent starts. */
  defaultArgs: string[];
}

/** Global agent settings + adapter catalog (`AgentSettingsDto`). */
export interface AgentSettingsDto {
  mergeMode: MergeMode;
  /** Global default adapter for bare spawns; empty = built-in default. */
  defaultAdapter: string;
  adapters: AgentAdapterConfig[];
}

/**
 * Options for `agent_spawn` (`commands/agent.rs`). Omitted fields fall back
 * to the project's `agents.default_adapter` / an auto-generated free name /
 * no initial prompt.
 */
export interface AgentSpawnOptions {
  adapterId?: string;
  name?: string;
  prompt?: string;
  /**
   * To-dos to work on; seeds the agent's prompt with their context. Several
   * to-dos are handed to the one agent as a single combined task.
   */
  todoIds?: TodoId[];
  /**
   * Scratchpads to work on; only used when `todoIds` is empty. Several
   * scratchpads are handed to the one agent as a single combined task.
   */
  scratchpadIds?: ScratchpadId[];
}

/**
 * Status of the built-in MCP server (`McpStatus` in `commands/mcp.rs`).
 * Deliberately token-free: the bearer token never crosses the IPC bridge.
 */
export interface McpStatus {
  running: boolean;
  /** Full endpoint URL (e.g. `http://127.0.0.1:49152/mcp`) when running. */
  url: string | null;
}

/**
 * One external MCP client Podium can register its stdio bridge with
 * (`McpClientInfo` in `commands/mcp.rs`).
 */
export interface McpClientInfo {
  /** Stable identifier (e.g. `"claude-code"`). */
  id: string;
  displayName: string;
  /** Whether the client's CLI resolves on the login-shell PATH. */
  cliAvailable: boolean;
  /** Whether the `podium` server entry is currently registered. */
  installed: boolean;
  /** The registration command line, for display / manual copy-paste. */
  installCommand: string;
  /**
   * The CLI command that lists registered servers, shown in the card hint
   * (e.g. `claude mcp list` / `auggie mcp list`).
   */
  checkCommand: string;
}

/**
 * One message on a `process_attach` channel (`TermEvent` in
 * `commands/process.rs`). `dataB64` is base64 of raw PTY bytes; `seq` is the
 * core scrollback sequence used to guard against snapshot/stream overlap.
 */
export type TermEvent =
  | { type: "snapshot"; seq: number; dataB64: string }
  | { type: "data"; seq: number; dataB64: string }
  | { type: "lagged" };

/**
 * The agent currently working on a to-do or scratchpad
 * (`podium_core::AssignedAgent`). Runtime-only on the Rust side (a process id
 * is per-run), so it is `null` after a restart until an agent is
 * (re)assigned. Shared by `TodoInfo` and `ScratchpadInfo`.
 */
export interface AssignedAgent {
  /** The agent process working on the item. */
  processId: ProcessId;
  /** The agent's display name, for showing without a process lookup. */
  name: string;
}

/**
 * One issue/PR link pinned to the top of a to-do (`podium_core::TodoLink`).
 * Agents add these over MCP when they open a GitLab issue or MR/PR.
 */
export interface TodoLink {
  /** Stable id, so a link can be removed. */
  id: LinkId;
  /** Human-readable label (e.g. `"#42 Fix login"`). */
  label: string;
  /** The http(s) URL to open. */
  url: string;
  /** RFC 3339 timestamp. */
  createdAt: string;
}

/** One progress note on a to-do (`podium_core::TodoComment`). */
export interface TodoComment {
  /** Stable id, so a comment can be edited or removed. */
  id: CommentId;
  /** Who left the note (e.g. an agent's name). */
  author: string;
  /** Raw markdown source, rendered on display. */
  text: string;
  /** RFC 3339 timestamp. */
  createdAt: string;
  /** RFC 3339 timestamp of the last edit; `null` if never edited. */
  editedAt: string | null;
}

/**
 * One to-do item (`podium_core::TodoInfo`). To-dos are keyed by project
 * root on the Rust side, so they survive app restarts. Agents keep the
 * description current and append comments over MCP as they work.
 */
export interface TodoInfo {
  id: TodoId;
  projectId: ProjectId;
  text: string;
  /** Longer detail, kept current by agents; `null` when unset. */
  description: string | null;
  done: boolean;
  /** RFC 3339 creation timestamp. */
  createdAt: string;
  /** RFC 3339 completion timestamp; `null` while open. */
  doneAt: string | null;
  /** Whether the to-do is archived (hidden from the main list). */
  archived: boolean;
  /** RFC 3339 timestamp of when it was archived; `null` while active. */
  archivedAt: string | null;
  /** Issue/PR links pinned to the top of the to-do, oldest first. */
  links: TodoLink[];
  /** Progress notes, oldest first. */
  comments: TodoComment[];
  /** The agent currently working on this to-do, or `null` if unassigned. */
  assignedAgent: AssignedAgent | null;
}

/**
 * One scratchpad (`podium_core::ScratchpadInfo`). Scratchpads are keyed by
 * project root on the Rust side, so they survive app restarts.
 */
export interface ScratchpadInfo {
  id: ScratchpadId;
  projectId: ProjectId;
  title: string;
  content: string;
  /** Whether the scratchpad is archived (hidden from the main list). */
  archived: boolean;
  /** RFC 3339 timestamp of when it was archived; `null` while active. */
  archivedAt: string | null;
  /** RFC 3339 creation timestamp. */
  createdAt: string;
  /** RFC 3339 timestamp of the last edit. */
  updatedAt: string;
  /** Who last touched the content: `"User"` or an agent name. */
  updatedBy: string;
  /** Increments on every content update, starting at 1. */
  version: number;
  /** Free-text tags, addable by the user and by agents over MCP. */
  tags: string[];
  /** The agent currently working on this scratchpad, or `null` if unassigned. */
  assignedAgent: AssignedAgent | null;
}

/** Structured error every IPC command can reject with (`IpcError`). */
export interface IpcError {
  /** Human-readable, sanitized description. */
  message: string;
  /** Stable machine-readable category, e.g. `"processNotFound"`. */
  kind: string;
}

/** Payload of the `process:added` / `process:removed` events. */
export interface ProcessRefEvent {
  projectId: ProjectId;
  processId: ProcessId;
}

/** Payload of the `process:status` event. */
export interface ProcessStatusEvent extends ProcessRefEvent {
  status: ProcessStatus;
}

/** Payload of the `project:opened` / `project:updated` / `project:closed` events. */
export interface ProjectRefEvent {
  projectId: ProjectId;
}
