# CLAUDE.md — Podium

Guidance for AI agents (and humans) working in this repository.

## What Podium is

Podium is a cross-platform **desktop cockpit for AI coding agents**, built in
**Rust + Tauri**. You open a project folder and Podium runs its dev servers
(from `podium.yml`), ad-hoc terminals, and AI coding agents (Claude Code
today; more adapters later) — each in its own PTY with a real terminal view,
lifecycle supervision, and a built-in **MCP server** so agents can inspect
and orchestrate the very processes Podium manages (including spawning more
agents). macOS-first, but macOS **and** Windows are built and packaged (the
PTY layer runs on Windows via ConPTY; process-group stop is Unix-only, Windows
terminates the ConPTY child directly). Linux is kept in mind, not yet packaged.

> Naming: product and repo are **Podium**. Core crate `podium-core`; bundle id
> `com.podium.app`.

> Lineage: architecturally patterned after an earlier desktop app by the same
> author — same Cargo workspace shape (a `*-core` crate with zero Tauri
> dependency, a thin `src-tauri` IPC layer, a presentation-only React
> frontend), the same `justfile`/version-sync conventions, and a mirrored
> `CLAUDE.md` structure. (That sibling app is a private repo — this is
> background context only; nothing here depends on access to it.)

## Boilerplate (shared skeleton)

Podium's shared skeleton — the Cargo workspace shape (a `*-core` crate with
zero Tauri dependency + a thin `src-tauri` IPC layer + a presentation-only
React frontend), the `justfile`, `scripts/sync-version.sh`, CI, the shared
config files, the design tokens, and the layering/IPC conventions — is
extracted into a standalone template:

- **Repo:** <https://github.com/RobbieMinderhoud/rust-application-boilerplate>.
  It is the **upstream** source of truth for the skeleton; Podium carries its
  domain (processes, PTYs, agents, MCP) on top.
- **Maintainer rule (internal):** when a change here touches the **shared
  skeleton or conventions** (not Podium-specific domain code), the maintainer
  also opens a **PR against the boilerplate** so it stays current. App-specific
  code does not flow back. Outside contributors can ignore this — just send
  your change as a normal PR to this repo.

## Status

Shipped and current (v1.1.x). The counts below are the source of truth — keep
them in sync when you add a command or MCP tool.

- ✅ **`podium-core`**: the whole domain — project open/close + `podium.yml`
  config (strict serde, broken config still opens the project), PTY process
  engine (spawn via `$SHELL -lic`, process groups, `killpg` SIGTERM→SIGKILL
  stop), ring-buffer scrollback with per-chunk `seq`, supervision (exponential
  backoff + circuit breaker), agent adapters (Claude Code, Auggie), per-project
  to-dos and scratchpads (persisted, shared with agents), git worktree
  management for agent isolation, and the built-in MCP server (axum + rmcp
  streamable-HTTP, bearer auth, 28 tools). Zero Tauri
  dependency; unit tests plus real-PTY/MCP integration tests on plain
  `cargo test`.
- ✅ **Tauri IPC layer** (`src-tauri`): **70 commands**, per-attach terminal
  `Channel` streaming (16ms/64KiB batching), a global-event forwarder for
  lifecycle events, persistent recents + workspace list, window-state
  persistence, an app-quit close guard while agents/terminals run
  (`window_confirm_close`), dialog + log plugins, locked-down CSP.
- ✅ **Frontend** (`src/`): project switcher + sidebar (To-dos / Agents /
  Processes / Terminals / Worktrees / Scratchpads), xterm.js terminals living **outside
  React** (registry keyed by processId; scrollback survives tab switches and
  StrictMode), snapshot+seq attach protocol with lag re-attach, new-agent
  modal, agent activity ("working…"/idle) heuristic, to-do and scratchpad
  detail views that fill the work area (description/comment thread; scratchpad
  editor with TOC, mutually exclusive with the focused process), settings +
  theme (dark/light/retro) + toasts, a collapsible, resizable right-side
  assistant chat panel (replayed transcript, action + proposal cards,
  autonomy picker, memory view, proactive `[Podium]` notices; conversation
  reset lives in Settings → Assistant), Zustand stores, typed IPC wrappers
  over all 70 commands.
- ✅ CI (`.github/workflows/ci.yml`): one macOS job — rustfmt, clippy
  `-D warnings`, `cargo test --workspace` (real PTYs), typecheck, ESLint,
  Vitest, production build.

## Tech stack

| Concern        | Choice                                                        |
| -------------- | ------------------------------------------------------------- |
| Shell / UI     | Tauri 2 + React 19 + TypeScript + Vite                        |
| PTY            | `portable-pty` (via `podium-core`), Unix process groups        |
| Agent protocol | MCP over streamable HTTP (`rmcp` + `axum`), bearer token       |
| Terminal UI    | `@xterm/xterm` + `@xterm/addon-fit`                            |
| FE state       | Zustand 5 (no data-fetching lib; push events + list resyncs)   |
| Motion         | CSS tokens (`--dur-*`/`--ease-*`), reduced-motion guard        |

## Architecture

A Cargo **workspace**. `podium-core` holds all orchestration/PTY/MCP logic and
has **zero Tauri dependency** (plain-`cargo test`-able). `src-tauri` is a thin
IPC adapter. `src/` is the React app (presentation only).

```
podium/
├─ Cargo.toml                  # [workspace]; [workspace.package] version is the source of truth
├─ justfile                    # dev/build/check/test/lint/format/version recipes
├─ scripts/sync-version.sh     # propagates the version to tauri.conf.json + package.json
├─ crates/podium-core/src/
│  ├─ orchestrator.rs          # Orchestrator — THE public entry point (all mutation goes through it)
│  ├─ project.rs               # project open/reload, cwd resolution (traversal-guarded)
│  ├─ config.rs                # podium.yml serde types (deny_unknown_fields)
│  ├─ process/                 # ProcessKind/Status/Spec, pty.rs (engine), scrollback.rs, supervisor.rs
│  ├─ agent/                   # AgentAdapter trait + claude.rs (ClaudeCodeAdapter) + auggie.rs (AuggieAdapter), McpConnectInfo
│  ├─ mcp/                     # built-in MCP server (mod.rs) + the 28 tools (tools.rs) + stdio bridge (bridge.rs)
│  ├─ assistant.rs             # the personal assistant: headless Claude Code child, stream-json in/out, autonomy gate + proposals, transcript/memory store, nudge queue
│  ├─ proactivity.rs           # EventBus watcher turning agent exits / stuck agents / lingering to-dos / a new day into assistant nudges
│  ├─ worktree.rs              # Podium-managed git worktrees (create/list/remove under .podium/worktrees/)
│  ├─ todo.rs                  # per-project to-dos (TodoInfo + persistent TodoStore)
│  ├─ events.rs                # PodiumEvent + broadcast EventBus
│  ├─ ids.rs                   # ProjectId / ProcessId / TodoId (UUID newtypes)
│  └─ error.rs                 # CoreError (messages are Podium-owned text, never output/secrets)
├─ src-tauri/src/
│  ├─ lib.rs                   # Tauri builder, plugins, MCP startup, command registration, shutdown
│  ├─ state.rs                 # AppState: Arc<Orchestrator> + McpServer handle
│  ├─ events.rs                # PodiumEvent → global Tauri events ("process:status", …)
│  ├─ error.rs                 # IpcError { kind, message } ← CoreError
│  └─ commands/{project,recents,workspace,process,agent,assistant,mcp,todo}.rs
└─ src/                        # React + Vite frontend
   ├─ components/              # Sidebar, ProcessRow, TodoSubsection, TerminalPane/View, StatusDot, modals, …
   ├─ state/                   # Zustand stores (project, process, todo, layout, settings, theme, toast)
   ├─ lib/                     # terminalRegistry, termProtocol, useAgentActivity, motion, …
   └─ ipc/                     # typed command wrappers + event listeners + hand-written types
```

### The core contract

`Orchestrator` (`crates/podium-core/src/orchestrator.rs`) is the **single
entry point**: projects, processes, agents, attach/stdin/resize, events,
shutdown. `src-tauri` holds one `Arc<Orchestrator>` in `AppState` and never
reaches around it.

Sidebar projects are a **persistent workspace**: `project_open` adds the
project's root to `workspace.json` (app data dir) and `project_close` removes
it; at startup the frontend calls `workspace_list` and re-opens every path via
`project_open`. A path that fails to open (folder moved/deleted, drive not
yet mounted) is *not* auto-pruned — a transient failure must not silently
drop the project — but its error toast offers a "Remove from workspace"
action wired to `workspace_remove` for permanent failures. Each entry is
a `{ path, name? }` object — the optional `name` is a user-set display-name
override (set via `project_rename`) and list position is the sidebar order
(set via `project_reorder`); legacy bare-path-string files are migrated on
load. `project_open` re-applies a stored name override so a renamed project
comes back named after a restart. Opening the same folder concurrently (e.g.
React StrictMode's double-mount firing `restoreWorkspace` twice) dedupes to
one project: the "already open" check and the id reservation happen under
one lock acquisition, atomically, before the async `podium.yml` load.

- **Processes** are shell command lines run via `$SHELL -lic` (login +
  interactive, so both `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc`
  PATH/env edits are honoured) in their own PTY + process group. Stop =
  SIGTERM to the group, SIGKILL after a grace period. A user stop is never
  counted as a crash.
- **Supervision** (`process/supervisor.rs`): `RestartPolicy` `never` /
  `on-crash` / `always`; exponential backoff 500ms → 30s (doubling), circuit
  breaker at 5 restarts per rolling 60s, backoff resets after 60s of stable
  running. Manual start fully resets both. Timings injectable
  (`SupervisorConfig`) so tests run in milliseconds.
- **Agent adapters** (`agent/mod.rs`): the `AgentAdapter` trait turns an
  `AgentLaunchCtx` (project root, prompt, MCP connect info, config
  `extra_args`) into a `LaunchPlan { command, env }`. Availability is probed
  via the login shell (`command -v`), so nvm/homebrew PATH edits are
  honoured. Adding an agent CLI = new module + registry entry; no IPC or
  frontend change. Max **8 active agents per project** (recursion guard —
  agents can spawn agents over MCP).
- **Built-in MCP server** (`mcp/`): rmcp streamable-HTTP nested in axum on
  `127.0.0.1:0` (ephemeral port) behind a **per-run bearer token**. Tools:
  `list_projects`, `list_processes`, `get_process_status`,
  `get_process_output` (tail, ANSI-stripped, default 100 / max 2000 lines),
  `spawn_agent` (its `args` param sets the model/reasoning effort for that
  agent, e.g. `["--model", "opus"]`), `start_process`, `stop_process`,
  `restart_process`, `remove_process`,
  `list_todos`, `add_todo`, `complete_todo`, `update_todo`, `comment_todo`
  (its `author` is rejected server-side if it impersonates the user —
  "you"/"user"/"gebruiker", case/whitespace-insensitive — falling back to
  `agent`, so no MCP caller can make a comment read as if the user wrote it),
  `add_todo_link` (pin an issue/PR URL to the top of a to-do),
  `set_todo_archived`, `assign_todo` (a running agent self-assigns a to-do via its
  `PODIUM_PROCESS_ID`, so the user sees who owns it — blocked if the to-do is
  already owned by another live session),
  `rename_session` (a running agent renames itself via its
  `PODIUM_PROCESS_ID` to reflect what the session is about), the
  scratchpad tools `list_scratchpads`, `create_scratchpad`,
  `update_scratchpad`, `add_scratchpad_tag`, `remove_scratchpad_tag`,
  `set_scratchpad_archived`, the worktree tools `create_worktree`,
  `list_worktrees`, `remove_worktree`, and `update_assistant_memory` (the
  personal assistant rewriting its own long-term memory).
  The same surface is served a second time at **`/mcp-assistant`**
  (`mcp::ASSISTANT_PATH`, same bearer token) for the personal assistant: that
  route is its identity, and `PodiumTools::call_tool` puts calls arriving there
  through the autonomy gate (see _Personal assistant_).
  Spawned agents get the URL + token via 0600 per-agent config files under
  `{app_data_dir}/mcp` (wiped on every start); the same dir holds
  `server.json` (current URL + token, 0600) for the **stdio bridge**:
  external MCP clients configure `Podium mcp-bridge` (a subcommand of the
  app binary, `mcp/bridge.rs`) once — the bridge proxies stdio ↔ HTTP,
  re-reads `server.json`, and transparently replays the `initialize`
  handshake with backoff when the app restarts, so client config never goes
  stale even though port and token rotate every launch
  (`PODIUM_APP_DATA_DIR` overrides the app data dir). `mcp/install.rs`
  registers the bridge with external clients (Claude Code —
  `claude mcp add --scope user --transport stdio podium -- <exe>
  mcp-bridge` — and Auggie —
  `auggie mcp add podium --command <exe> --args mcp-bridge --replace`, its
  installed-check reading `auggie mcp list --json`; all via the login
  shell) — each surfaced in the Settings → MCP tab as a one-click Run/Copy
  card with an installed indicator.
- **To-dos** (`todo.rs`): each project has a shared to-do list, visible to
  the user (sidebar) and every agent (MCP). Persisted in one `todos.json`
  in the app data dir — **keyed by project root path**, not project id, so
  lists survive restarts; atomic writes (temp file + rename) keep the
  project folder clean and the file uncorrupted. Every mutation emits
  `TodosChanged`. To-dos can carry pinned issue/PR **links** (agents add
  them via `add_todo_link`, shown at the top of the detail pane) and can be
  **archived**: listing auto-archives any done to-do left over from an
  earlier day, and a to-do can be archived/unarchived manually — archived
  items drop out of the active list and show in the Archive modal. An
  **assigned** to-do carries its owning agent's session colour (each agent
  gets a subtle colour at spawn from an 8-hue palette, avoiding colours other
  live agents hold; it rides on `AssignedAgent`, and is also surfaced on
  `ProcessInfo.color`): the to-do row **and the agent's own sidebar row** are
  tinted the same colour so the two can be matched at a glance; the to-do's
  spawn button is hidden and it drops out of multi-select — one session owns
  it. Sidebar rows distinguish being **open** in the work area (a subtle
  accent highlight, like agents/terminals) from being in a **Cmd/Ctrl+click
  multi-select** (a bolder accent fill — those rows are queued for the next
  agent).
- **Scratchpads** (`scratchpad.rs`): each project has shared freeform-notes
  scratchpads, visible to the user and every agent (MCP). Persisted in one
  `scratchpads.json`, keyed by project root path like to-dos. Scratchpads
  carry free-text **tags** (`add_tag`/`remove_tag`, dedup by exact value) and
  can be **archived/unarchived** manually (`set_archived`, mirroring to-dos'
  archive semantics but with no auto-archive sweep) and **deleted** from the
  Archive view (`scratchpad_remove` / `Orchestrator::remove_scratchpad`, again
  mirroring to-dos — deletion lives behind archiving). Content/title updates
  require an `expected_updated_at` matching the scratchpad's current
  `updatedAt` — a stale value (a concurrent user or agent edit landed first)
  is rejected as `CoreError::ScratchpadConflict` instead of silently
  overwriting it; the frontend surfaces this as an in-pane reload/force-save
  choice rather than a toast.
- **Worktrees** (`worktree.rs`): Podium-managed git worktrees for agent
  isolation, at `<project_root>/.podium/worktrees/<name>` (name slugified,
  auto-de-duplicated with `-2`, `-3`, …). `create(root, name)` checks out a
  fresh `podium/<name>` branch off HEAD. `.podium/` is excluded via
  `.git/info/exclude` (never
  `.gitignore`). **Podium never creates a worktree at spawn time**: an agent
  always starts in the project root (where the user's tooling and caches
  live), and `agent_spawn`'s `worktree` flag — the spawn checkbox — only makes
  its identity prompt tell it to create one itself via the `create_worktree`
  MCP tool before its first code change, named `worktreeName` when given, else
  a name the agent picks. So every worktree comes from `create_worktree`,
  whether at the start of a session or mid-session. Every worktree git call
  runs `platform::git_binary()` directly (a path resolved once via the login
  shell, then reused — not a login shell per call, which was slow on heavy
  shell profiles) rather than through the shell. `git worktree list
  --porcelain` is the source of truth and `list` has no events, but
  `create`/`remove` emit `WorktreesChanged` for the project, which the
  frontend's `worktreeStore` uses (alongside a background load on project
  open) to keep its cache warm. `ProcessInfo.worktree` is derived from the
  process cwd and feeds the sidebar badge, `in_use` and the removal guard —
  an agent that creates its own worktree keeps the root as cwd, so it does
  not light those up. Removal is refused while an active process runs in the
  worktree (`WorktreeInUse`) or while it has uncommitted changes unless
  forced (`WorktreeDirty`); the branch is always kept. The global
  `suggest_worktree` setting (default on) makes agent identity prompts *ask*
  the user about a worktree before the first code change; the spawn flag is
  the same instruction without the asking, and wins over the setting.

  **Sidebar** (`WorktreeSubsection.tsx`): a "Worktrees" subsection directly
  under Terminals, reading from `worktreeStore` — hidden entirely when the
  project has none (no header "+"; worktrees only ever come from
  `create_worktree`, never a sidebar action). Each row carries four actions:
  open a terminal rooted in it (`cwd: .podium/worktrees/<name>`), open it in
  Warp (`worktree_open_in_warp`, same URI scheme as the project-level
  button), open the Commands modal targeted at it (its quick commands then
  run there instead of the project root), and remove it
  (`worktreeRemove(force=false)`; a
  `worktreeDirty` rejection prompts a confirm before retrying with
  `force=true`). A worktree's own terminal processes (`p.worktree ===
  wt.name`) render nested under its row via the shared `ProcessRow` and are
  excluded from the plain Terminals list above. "In use" for the delete
  button is derived live from the process store (any non-exited process with
  that `worktree`) rather than the worktree's own cached `inUse` field, which
  can go stale between backend refreshes while the sidebar stays live; the
  backend's `WorktreeInUse` guard remains the authority and surfaces as a
  toast if it fires anyway.

- **Personal assistant** (`assistant.rs`): a workspace-wide chat panel backed
  by **one long-lived headless Claude Code child** — a plain piped process
  (`$SHELL -lic 'exec claude -p --input-format stream-json --output-format
  stream-json --verbose --permission-mode bypassPermissions --disallowedTools
  …'`), not a PTY, not in the sidebar process list. `--permission-mode
  bypassPermissions` is required because `-p`/stream-json mode has no TTY for
  an interactive permission prompt — without it the CLI would stall asking
  approval for every Podium MCP tool it can never answer; deny rules still
  take precedence over it (see below), so this does not loosen what the
  assistant can touch. It is spawned lazily on the first message and killed on
  shutdown; a session that died is replaced on the next turn. User turns are
  written to its stdin as one JSON line each; its stdout stream is parsed by
  `parse_stream_line` into `AssistantEvent`s (`started` / `message` / `tool` /
  `toolFailed` / `notice` / `proposal` / `proposalResolved` / `turnEnd` /
  `exited`) published on the `EventBus` and forwarded to the webview as
  `assistant:event`. Unparseable lines (a chatty shell profile) are
  skipped, stderr is dropped, and the binary + cwd are injectable
  (`AssistantConfig`) so integration tests drive a fake `claude` script. The
  assistant never edits code itself: `Edit`/`Write`/`NotebookEdit`/`Bash` are
  refused at the CLI regardless of permission mode, and implementation work is
  meant to go out as a to-do + spawned agent (its `--append-system-prompt`
  identity says so).

  It reaches Podium over its **own MCP endpoint** (`/mcp-assistant`), wired in
  via a 0600 `assistant.json` next to the per-agent configs — the whole tool
  surface, workspace-wide. `--strict-mcp-config` is deliberately *not* passed,
  so the user's own MCP servers (mail, Slack, …) stay available.

  **Autonomy** (`AutonomyMode`, stored in `agents.json` under `assistant`,
  default `confirm-destructive`) is enforced **server-side**, not by prompt
  wording: `PodiumTools::call_tool` asks `assistant::requires_proposal` whether
  this tool needs the user, and a gated call never runs. Instead it is parked as
  a `ProposalInfo` — carrying a closure that replays that very dispatch later —
  and the tool answers that a proposal is awaiting the user. `full` gates
  nothing; `confirm-destructive` gates the stopping/archiving/removing tools;
  `propose-all` gates everything that is not read-only (an *unknown* tool counts
  as state-changing, so a tool added later cannot slip through). Proposals live
  in memory (never expiring, but lost on restart) and surface as
  `assistant:event` `proposal` / `proposalResolved`; the user's Confirm/Reject
  in the chat calls `assistant_resolve_proposal`, which runs the stored action
  and tells the assistant the outcome as a `[Podium]` turn so it can carry on.
  A failed tool call is never silent: `tool_result` blocks flagged `is_error`
  become a `toolFailed` event.

  **Persistence** is one `assistant.json` in the app data dir (atomic writes,
  the same kind of store as `todos.json`): Claude's own session id plus the
  chat rows the panel replays — user turns, assistant prose, actions run and
  actions that failed, capped at the most recent 500. On the next spawn that id
  comes back as `--resume`, so a restart continues the same conversation
  instead of meeting a stranger; a session that dies without ever announcing
  itself makes Podium drop the stored id, so a pruned session cannot wedge
  every later turn. The panel's **new-conversation** button
  (`assistant_new_conversation`) kills the session and drops transcript,
  session id and pending proposals — that kill is deliberate, so it is not
  reported as `exited`.

  **Memory** is a separate `assistant-memory.md` in the same dir: free-form
  markdown appended to the identity prompt at spawn under `# Your memory`, so
  it survives new conversations. The assistant maintains it itself through the
  `update_assistant_memory` MCP tool (full replace — an empty string clears
  it), and the user can read, edit or clear it in the panel's memory view. It
  is read fresh from disk on every spawn, so hand-editing the file works; a
  change reaches a *running* session only on its next spawn.

  **Proactivity** (`proactivity.rs`, setting `proactive` in the same
  `assistant` block, default **on**) lets Podium speak first. One task watches
  the `EventBus` and, on a slow tick, the process/to-do state: an agent that
  exited (finished or crashed), a running agent that has printed nothing for
  5 minutes (so it is probably waiting on a question — `Orchestrator::idle_agents`,
  measured from the scrollback's last append, or from the process start when it
  never printed), a 6-hourly sweep for to-dos open longer than a week, and one
  day-start summary the first time Podium opens on a given day (claimed in
  `assistant.json`, so restarts do not re-ask). Each becomes one `[Podium] …`
  turn plus a `notice` event, so the chat says why the assistant is speaking
  unprompted. Timings are injectable (`ProactivityConfig`) so tests run in
  milliseconds. A nudge **never spawns the CLI child**: with no conversation
  running it queues (newest 20) and rides along with the user's next turn — a
  fresh session would have no idea what the nudge was about. Switching the
  setting off silences all of it, day-start claim included. The watcher holds
  only a weak reference, so it ends with the orchestrator. **Settings →
  Assistant** holds this toggle plus the autonomy mode (the chat header keeps
  its own autonomy picker — both mirror the one stored value).

  A pending proposal is also dropped when **its project closes**
  (`invalidate_proposals_for_project`, keyed off the call's `project_id`
  argument): confirming it later could only fail, so the card resolves itself
  with the reason and the assistant is told.

### Tauri IPC contract

Commands return `Result<T, IpcError>` where `IpcError = { kind, message }`
(messages are Podium-owned text — never terminal output or secrets). Tauri 2
maps camelCase JS argument keys to the snake_case Rust parameters.

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `project_open`          | `{ path }` (absolute, from folder picker)   | `ProjectInfo`     |
| `project_close`         | `{ projectId }`                             | –                 |
| `project_list`          | –                                           | `ProjectInfo[]`   |
| `project_config_reload` | `{ projectId }`                             | `ProjectInfo`     |
| `project_rename`        | `{ projectId, name? }` (blank clears)       | `ProjectInfo`     |
| `project_reorder`       | `{ ordered: string[] }` (project ids)       | `ProjectInfo[]`   |
| `project_just_recipes`  | `{ projectId }`                             | `string[]`        |
| `project_open_in_warp`  | `{ projectId }`                             | –                 |
| `recents_list`          | –                                           | `RecentProject[]` |
| `recents_remove`        | `{ path }`                                  | `RecentProject[]` |
| `workspace_list`        | –                                           | `string[]`        |
| `workspace_remove`      | `{ path }`                                  | `string[]`        |
| `process_add`           | `{ projectId, spec: NewProcess }`           | `ProcessInfo`     |
| `process_remove`        | `{ processId }`                             | –                 |
| `process_list`          | `{ projectId? }`                            | `ProcessInfo[]`   |
| `process_git_branch`    | `{ processId }`                             | `string \| null`  |
| `process_rename`        | `{ processId, name }`                       | `ProcessInfo`     |
| `process_start`         | `{ processId }`                             | –                 |
| `process_stop`          | `{ processId }`                             | –                 |
| `process_restart`       | `{ processId }`                             | –                 |
| `process_write`         | `{ processId, dataB64 }`                    | –                 |
| `process_resize`        | `{ processId, cols, rows }`                 | –                 |
| `process_attach`        | `{ processId, channel: Channel<TermEvent> }`| –                 |
| `adapters_list`         | –                                           | `AdapterInfo[]`   |
| `agent_spawn`           | `{ projectId, adapterId?, name?, prompt?, todoIds?, scratchpadIds?, worktree?, worktreeName?, args? }` | `ProcessInfo` |
| `agent_settings_get`    | –                                           | `AgentSettingsDto` |
| `agent_settings_set_adapter` | `{ adapterId, command?, defaultArgs }` | `AgentSettingsDto` |
| `agent_settings_set_default_adapter` | `{ adapterId? }` (blank clears) | `AgentSettingsDto` |
| `agent_settings_set_merge_mode` | `{ mode }`                       | `AgentSettingsDto` |
| `agent_settings_set_suggest_worktree` | `{ enabled }`             | `AgentSettingsDto` |
| `mcp_status`            | –                                           | `{ running, url? }` (token-free by design) |
| `mcp_clients_status`    | –                                           | `McpClientInfo[]` |
| `mcp_client_install`    | `{ clientId }`                              | `McpClientInfo[]` |
| `todo_list`             | `{ projectId }`                             | `TodoInfo[]`      |
| `todo_list_archived`    | `{ projectId }`                             | `TodoInfo[]`      |
| `todo_set_archived`     | `{ projectId, todoId, archived }`           | `TodoInfo`        |
| `todo_add`              | `{ projectId, text }`                       | `TodoInfo`        |
| `todo_set_done`         | `{ projectId, todoId, done }`               | `TodoInfo`        |
| `todo_update`           | `{ projectId, todoId, text?, description? }`| `TodoInfo`        |
| `todo_comment`          | `{ projectId, todoId, text, author? }`      | `TodoInfo`        |
| `todo_comment_update`   | `{ projectId, todoId, commentId, text }`    | `TodoInfo`        |
| `todo_comment_remove`   | `{ projectId, todoId, commentId }`          | `TodoInfo`        |
| `todo_add_link`         | `{ projectId, todoId, url, label? }`        | `TodoInfo`        |
| `todo_remove_link`      | `{ projectId, todoId, linkId }`             | `TodoInfo`        |
| `todo_remove`           | `{ projectId, todoId }`                     | –                 |
| `todo_unassign`         | `{ projectId, todoId }`                     | `TodoInfo`        |
| `scratchpad_list`       | `{ projectId }`                             | `ScratchpadInfo[]`|
| `scratchpad_list_archived` | `{ projectId }`                          | `ScratchpadInfo[]`|
| `scratchpad_add`        | `{ projectId }`                             | `ScratchpadInfo`  |
| `scratchpad_update_content` | `{ projectId, id, content, expectedUpdatedAt }` | `ScratchpadInfo` |
| `scratchpad_update_title`   | `{ projectId, id, title, expectedUpdatedAt }`   | `ScratchpadInfo` |
| `scratchpad_add_tag`    | `{ projectId, id, tag }`                    | `ScratchpadInfo`  |
| `scratchpad_remove_tag` | `{ projectId, id, tag }`                    | `ScratchpadInfo`  |
| `scratchpad_set_archived` | `{ projectId, id, archived }`             | `ScratchpadInfo`  |
| `scratchpad_remove`     | `{ projectId, id }`                         | –                 |
| `scratchpad_unassign`   | `{ projectId, id }`                         | `ScratchpadInfo`  |
| `worktree_list`         | `{ projectId }`                             | `WorktreeInfo[]`  |
| `worktree_remove`       | `{ projectId, name, force }`                | `WorktreeInfo[]` (refreshed list) |
| `worktree_open_in_warp` | `{ projectId, name }`                       | –                 |
| `assistant_send`        | `{ text }`                                  | – (reply streams as `assistant:event`) |
| `assistant_transcript`  | –                                           | `TranscriptEntry[]` |
| `assistant_new_conversation` | –                                      | –                 |
| `assistant_memory_get`  | –                                           | `string`          |
| `assistant_memory_set`  | `{ content }` (blank clears)                | –                 |
| `assistant_autonomy_get` | –                                          | `AutonomyMode`    |
| `assistant_autonomy_set` | `{ mode }`                                 | `AutonomyMode`    |
| `assistant_proactivity_get` | –                                       | `boolean`         |
| `assistant_proactivity_set` | `{ enabled }`                           | `boolean`         |
| `assistant_resolve_proposal` | `{ id, approved }`                     | – (outcome streams as `assistant:event`) |
| `window_confirm_close`  | –                                           | –                 |

`window_confirm_close` is the frontend's answer to the app-quit close guard:
when agents/terminals are still running the quit is intercepted and a warning
modal shown; confirming calls this command, which sets the force-close flag so
the next quit proceeds.

> ⚠️ **Breaking change (Phase 4):** `scratchpad_update_content` and
> `scratchpad_update_title` — plus the matching `Orchestrator` methods
> (`update_scratchpad_content`/`update_scratchpad_title`) and the
> `update_scratchpad` MCP tool — now require `expectedUpdatedAt`
> (`expected_updated_at` for the MCP tool), the scratchpad's current
> `updatedAt` echoed back verbatim. A stale value fails with
> `IpcError.kind === "scratchpadConflict"` over Tauri, or an
> `invalid_params` MCP error whose message starts with `scratchpad
> conflict:`. Any caller written against Phase 1's 4-argument signature
> (frontend, MCP client, or another agent) must be updated to pass the
> timestamp through.

`NewProcess` is `{ name, command?, cwd?, kind, adapter?, restartPolicy? }`
(`kind` is serde-flattened). `cwd` is **relative to the project root**; the
core rejects absolute paths and traversal out of the root. Terminals default
`command` to an interactive shell (`exec "$SHELL" -i`).

**Terminal streaming (`process_attach`)** — attach is *snapshot + channel*:
one `{type:"snapshot", seq, dataB64}` (full scrollback; `seq` = first live
chunk after it), then batched `{type:"data", seq, dataB64}` (flushed every
16ms or at 64KiB). The frontend drops any batch whose `seq` is below the
snapshot's (no double-writes) and must **re-attach on `{type:"lagged"}`** —
a partial stream would corrupt the terminal. Payloads are base64 both ways
(`process_write` too) so raw bytes survive the JSON hop.

**Lifecycle events** are global Tauri events (low-volume only; terminal
output never travels this path): `process:added` / `process:removed` /
`process:updated` `{projectId, processId}`, `process:status`
`{projectId, processId, status}`, `project:opened` / `project:updated` /
`project:closed` / `todo:changed` / `scratchpad:changed` /
`worktree:changed` `{projectId}`, plus `assistant:event`
(the assistant's chat stream — a bare, `type`-tagged `AssistantEvent`).

> ⚠️ **Field casing for the frontend:** everything on the wire is
> **camelCase** — DTO fields (`projectId`, `dataB64`, `configError`, …) and
> the serde tags: `ProcessKind` is tagged by `kind`
> (`service`/`terminal`/`agent {adapter}`), `ProcessStatus` by `state`
> (`notStarted`/`running {pid, since}`/`stopping`/`exited {code, crashed, …}`),
> `TermEvent` by `type` (`snapshot`/`data`/`lagged`). The one exception:
> `RestartPolicy` values are **kebab-case** (`never`/`on-crash`/`always`) —
> matching `podium.yml`'s `auto_restart` — and so are `MergeMode` and
> `AutonomyMode` (`full`/`confirm-destructive`/`propose-all`), which share the
> settings file with it. Match these exactly in the hand-written types in
> `src/ipc/types.ts`.

### `podium.yml`

Optional, at the project root, **strict** (`deny_unknown_fields` — typos are
readable errors; a broken config still opens the project and surfaces via
`ProjectInfo.configError`). Snake_case keys:

```yaml
name: Webshop            # display name (default: folder name)
icon_initials: WS        # sidebar badge, max 2 chars
processes:
  - name: dev-server
    command: pnpm dev    # run via $SHELL -lic
    cwd: web             # relative to the project root
    auto_start: true
    auto_restart: on-crash   # never | on-crash | always
    env: { PORT: "3000" }
agents:
  default_adapter: claude-code
  extra_args: []
```

Config-defined processes are owned by the file: a reload replaces them
(stopping running ones) but keeps manually added processes. The Processes
"+" button is intentionally absent — services come from `podium.yml`.

### Terminal & activity (frontend)

xterm.js instances live **outside React** in `src/lib/terminalRegistry.ts`
(keyed by processId, reparentable host div) — this survives StrictMode
double-mounts and keeps scrollback across tab switches. The registry owns
attach/re-attach (including the `lagged` recovery) and re-themes live
terminals on theme change. The agent **activity indicator** is a frontend
heuristic: the registry timestamps incoming output, and `useAgentActivity`
(`src/lib/useAgentActivity.ts`) polls it (1s) — a running agent with output
in the last 2.5s shows as "working…" (pulsing dot), otherwise "idle". No
extra IPC. A quiet screen showing a permission/confirmation prompt or a
credential request (`sudo` password, ssh passphrase) reads as **"needs
input"** instead — derived for **every** running process, not just agents, so
a terminal stalled on `sudo` surfaces the same way; non-agents only show that
state ("working…"/"idle" is noise for a plain shell).

## Build, run, test

Prefer `just` (run `just` to list recipes). The Tauri CLI is the local
`@tauri-apps/cli` dev-dependency, so use `pnpm tauri …` (no global install).

```
just dev            # run the app (Vite + Tauri, hot reload)
just dev-signed     # debug .app, stable signing identity, no hot reload (macOS)
just build          # release bundle: .app + .dmg (what CI ships)
just build-signed   # signed release .app, opens Finder on it (macOS)
just install-signed # build-signed, then copy into /Applications
just check          # cargo check --workspace
just test           # cargo test --workspace + frontend vitest
just test-core      # podium-core tests only
just lint           # clippy -D warnings + eslint
just format         # rustfmt + prettier
just version 0.2.0  # bump + sync version across all manifests
just version-check  # verify versions are in sync
```

Every recipe exports `CARGO_TARGET_DIR` pointing at the **main checkout's**
`target/`, so git worktrees share one build cache instead of each starting
cold. Cargo locks that directory: two builds in different worktrees serialise
rather than run in parallel. Release builds use cargo's default codegen
settings — fat LTO + `codegen-units = 1` made the final link ~110s slower on
every build for ~3.5 MB of binary size.

`just dev` ad-hoc-signs the binary with a fresh identity on every rebuild, so
the macOS keychain re-prompts each run and "Always Allow" never sticks. The
`*-signed` recipes pass `src-tauri/tauri.dev-signed.conf.json`, which only sets
`bundle.macOS.signingIdentity` — one stable identity across rebuilds, so a
single "Always Allow" persists and there's no Gatekeeper prompt on this
machine. They build a bundle, so there is **no hot reload**: use `just dev` for
iteration, `just dev-signed` when the keychain prompt is in the way, and
`just install-signed` to put the app in `/Applications`. Only `just build`
produces the `.dmg` (via `scripts/make-dmg.sh`) — that is the release path CI
uses; the signed recipes deliberately skip it (`--bundles app`).

Core integration tests (`crates/podium-core/tests/`) exercise **real PTYs**
and a real MCP server on ephemeral ports — they run in plain `cargo test`
(no Docker, no `#[ignore]`) but need a Unix environment; CI runs on macOS
for this reason. Frontend tests are Vitest + jsdom + React Testing Library
(`src/test/setup.ts` registers matchers and cleanup); Tauri APIs are mocked
per test file.

## Conventions

- **Rust:** `#![forbid(unsafe_code)]` in every crate; `cargo clippy -- -D
  warnings` must pass; `cargo fmt`. Comment the _why_ of non-obvious logic.
- **Layering (always-on):** all orchestration logic belongs in
  `podium-core`; `src-tauri` only (de)serializes and forwards. If a command
  handler grows logic, move it down. The core must keep compiling without
  Tauri.
- **Security (always-on):**
  - The MCP **bearer token** is never logged, never crosses the Tauri IPC
    bridge (`mcp_status` is token-free by design), and reaches agents and
    the stdio bridge only via 0600 config files that are wiped on every
    start. `McpConnectInfo` has a manual `Debug` impl so `{:?}` cannot leak
    it, and the bridge never writes it to its client-facing stdio stream.
  - **Never log** terminal output or process command lines above
    `DEBUG`/`TRACE`; `IpcError`/`CoreError` messages are Podium-owned text.
  - `cwd` for new processes is validated against the project root (no
    absolute paths, no `..` escape).
  - Stops kill the whole **process group** (`killpg`) so children cannot
    outlive their process.
  - In tests/fixtures use **only fictitious** sample data.
- **Motion:** timing comes from the `--dur-*` / `--ease-*` tokens in
  `src/styles/tokens.css` — no raw values in components. Animate only
  `transform`/`opacity` in hot paths; the global reduced-motion guard in
  `global.css` must keep working (the pulsing activity dot is opacity-only
  for this reason).
- **Casing:** wire casing is fixed (see the IPC contract above); keep
  `src/ipc/types.ts` in lockstep with the Rust serde derives whenever either
  side changes.
- **Testing (always-on):** whenever you add or change behaviour, add or
  update the tests **in the same change** — don't leave it as a follow-up.
  Match the existing conventions rather than inventing new harnesses:
  - **Core logic** → Rust unit tests next to the code (`#[cfg(test)] mod
    tests`) and, for PTY/MCP/supervision behaviour, integration tests in
    `crates/podium-core/tests/` (real PTYs/MCP on ephemeral ports, injectable
    timings so they finish in ms). New MCP tools get a `tools/list` +
    round-trip assertion.
  - **New Tauri command** → keep it a thin forwarder and cover the logic it
    calls in `podium-core`; extend the IPC contract table in this file.
  - **Frontend component/store** → a Vitest + jsdom + RTL file beside it
    (`Foo.test.tsx`), mocking `@tauri-apps/*` per file and seeding Zustand
    via `setState(..., true)` (see `WelcomeScreen.test.tsx` /
    `TodoDetailPane.test.tsx`); assert user-visible behaviour and that store
    actions are called with the right args.
  - Use **only fictitious** sample data, and run the relevant suite
    (`just test` / `just test-core` / `pnpm vitest run`) before reporting
    done. Only skip adding a test when it is genuinely infeasible (e.g. a
    pure canvas/xterm path jsdom can't exercise) — say so explicitly.

## Git workflow

The repo is on GitHub (`RobbieMinderhoud/Podium`); use the **`gh` CLI** for PRs.
**Every code change goes through a branch → PR** — do not commit straight to
`main`. `main` stays releasable; each change merges via a PR and is eventually
released (see _Versioning & CI_). We do **not** open a GitHub issue per change:
the PR is the unit of work (title + body capture the what and why).
Conversations that touch no code (questions, exploration) need no branch.

1. **Branch** — from an up-to-date `main`, named `short-description`
   (e.g. `agent-activity-fix`). No issue-number prefix.
2. **Implement, commit, push** the branch. Commit subjects are free-form,
   imperative (Keep-a-Changelog voice). Per the global rule, **never add a
   `Co-Authored-By` trailer** (a `commit-msg` hook also strips Anthropic ones).
3. **PR** — `gh pr create --base main --fill` (or with an explicit title/body).
   `ci.yml` runs on the PR and must be green.
4. **Merge + release** — cut the release with the **`release` skill**
   (`.claude/skills/release/SKILL.md`): it folds the CHANGELOG entry + version
   bump into the feature PR, then (with explicit approval) squash-merges and
   tags. Pushing a `v*` tag triggers the macOS bundle build.

## Versioning & CI

- **SemVer.** The `[workspace.package]` version in the root `Cargo.toml` is
  the single source of truth; `scripts/sync-version.sh` propagates it to
  `tauri.conf.json` and `package.json` (`just version-check` detects drift).
- **`CHANGELOG.md`** follows _Keep a Changelog_ and lists **only functional
  (user-facing) changes** — the `release` skill adds an entry per release.
- **CI** (`.github/workflows/ci.yml`): one `macos-latest` job on every PR
  and push to `main` — rustfmt, clippy `-D warnings`, `cargo test
  --workspace` (needs real PTYs, hence macOS), then typecheck, ESLint,
  Vitest, and a production `pnpm build`. A stub `dist/` is created first so
  `tauri::generate_context!` compiles.
- **Bundle build** (`.github/workflows/build.yml`): a matrix over macOS and
  Windows, each building natively (cross-compiling Tauri is painful). macOS
  packages the `.app` + APFS `.dmg` (via `scripts/make-dmg.sh`); Windows
  packages the NSIS `.exe`. Runs on `workflow_dispatch` and any pushed `v*`
  tag, uploading each OS's installer as an artifact. CI (`ci.yml`) still runs
  tests on macOS only — they need real Unix PTYs.
- **Releasing** is done via the **`release` skill** (see _Git workflow_).
  Do **not** create git tags or releases without explicit user approval
  (invoking the skill + its confirmation step is that approval).
- **Not yet set up:** Conventional Commits, git-cliff changelog automation,
  signed/notarized releases.

## Roadmap

All the originally planned phases are shipped, plus post-v1.0 work
(scratchpads, the app-quit close guard, the Auggie adapter, the retro theme).
This file and the code are the source of truth for current behaviour.
