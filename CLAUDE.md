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
terminates the ConPTY child directly; the PTY reader answers ConPTY's startup
`ESC[6n` cursor query itself and strips it from the output, since ConPTY holds
the child until it's answered and xterm.js only exists once a terminal is
opened — see `CursorHandshake` in `process/pty.rs`). On Windows an optional
**WSL-backend mode** runs everything Podium executes inside a WSL distro (see
_WSL-backend mode_ below). Linux is kept in mind, not yet packaged.

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
  streamable-HTTP, bearer auth, 31 tools). Zero Tauri
  dependency; unit tests plus real-PTY/MCP integration tests on plain
  `cargo test`.
- ✅ **Tauri IPC layer** (`src-tauri`): **83 commands**, per-attach terminal
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
  waiting-message rows with send-now/edit/remove and an interrupt control
  while a turn runs, autonomy picker, memory view, proactive `[Podium]` notices;
  conversation reset lives in Settings → Assistant), Zustand stores, typed
  IPC wrappers over all 83 commands.
- ✅ CI (`.github/workflows/ci.yml`): a macOS job — rustfmt, clippy
  `-D warnings`, `cargo test --workspace` (real PTYs), typecheck, ESLint,
  Vitest, production build — plus a Windows job (rustfmt, clippy, `cargo test
  -p podium-core` only) so `cfg(windows)` code is compiled, linted and tested.

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
│  ├─ mcp/                     # built-in MCP server (mod.rs) + the 31 tools (tools.rs) + stdio bridge (bridge.rs)
│  ├─ assistant.rs             # the personal assistant: headless Claude Code child, stream-json in/out, autonomy gate + proposals, transcript/memory store, nudge queue
│  ├─ proactivity.rs           # EventBus watcher turning agent exits / stuck agents / lingering to-dos / a new day into assistant nudges
│  ├─ worktree.rs              # Podium-managed git worktrees (create/list/remove under .podium/worktrees/)
│  ├─ platform.rs              # the OS gap: login shell, quoting, probe, git command, WSL-backend switch
│  ├─ platform/wsl_path.rs     # pure Windows <-> WSL path translation
│  ├─ backend_settings.rs      # backend.json (the WSL distro), read once at startup
│  ├─ todo.rs                  # per-project to-dos (TodoInfo + persistent TodoStore)
│  ├─ events.rs                # PodiumEvent + broadcast EventBus
│  ├─ ids.rs                   # ProjectId / ProcessId / TodoId (UUID newtypes)
│  └─ error.rs                 # CoreError (messages are Podium-owned text, never output/secrets)
├─ src-tauri/src/
│  ├─ lib.rs                   # Tauri builder, plugins, MCP startup, command registration, shutdown
│  ├─ state.rs                 # AppState: Arc<Orchestrator> + McpServer handle
│  ├─ events.rs                # PodiumEvent → global Tauri events ("process:status", …)
│  ├─ error.rs                 # IpcError { kind, message } ← CoreError
│  └─ commands/{project,recents,workspace,process,agent,assistant,backend,mcp,todo}.rs
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
- **Agent session memory** (`agent/session_memory.rs`, setting
  `remember_sessions` in `agents.json`, default off): `session_memory.json`,
  keyed by project root. `Orchestrator::shutdown()` is the only place a
  session is captured — for each still-running Claude Code agent it looks in
  `<config>/projects/<sanitized-cwd>/`, where `<config>` is
  `$CLAUDE_CONFIG_DIR` or else `~/.claude` (every non-ASCII-alphanumeric char of
  the cwd becomes `-`, Claude Code's own scheme). In WSL-backend mode both are
  the distro's: `<config>` is asked of the distro's login shell once
  (`platform::claude_projects_root`, lazily — the `Orchestrator` is built
  before `init_wsl`) and reached as a `\\wsl.localhost\…` path, and the cwd is
  sanitized in its Linux spelling (`backend_path`). It looks for the `<id>.jsonl` carrying
  the `{"type":"custom-title","customTitle":"podium-<tag>"}` line Claude Code
  writes for the agent's `-n` name, so agents sharing a cwd each get their own
  session. Only files written since the agent's spawn (minus a few seconds
  of slack for coarse mtimes) are read, since session files can be large
  and this runs at exit. Without a title match it falls back to the id the
  agent was resumed with; otherwise nothing is remembered — deliberately no
  "newest file in the cwd" guess, which would adopt a `claude` session the
  user runs themselves in the same repo. Only hyphenated UUIDs
  (`Uuid::try_parse`) are accepted, at capture and again at
  restore (the file is user-writable, and `--resume -x` would read as a
  flag). The next fresh `open_project` on that root passes each remembered id
  to its adapter as `--resume` when `remember_sessions` is on, skipping ids
  whose jsonl Claude Code has since pruned; the remembered list for that root
  is always cleared on open, on or off. The MCP server is started
  synchronously in `setup` so a restored agent never spawns before the MCP
  connect info (and with it `-n`) is set.
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
  `list_worktrees`, `remove_worktree`, `update_assistant_memory` (the
  personal assistant rewriting its own long-term memory), and
  `report_to_assistant` (a finished or blocked agent handing the assistant a
  short summary of its own work, routed to `Orchestrator::assistant_nudge` so
  it gets the same live-or-queued delivery, cap and proactivity switch as any
  other nudge), and `verify_sender` (see _Cross-session messaging_).
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
- **Cross-session messaging**: Podium's own sessions — the assistant and
  every spawned agent — can reach each other with Claude Code's `SendMessage`
  / `ListAgents`. Two things make that work, both set per session so the
  user's own `~/.claude/settings.json` is never touched:
  - **`crossSessionInbound: "accept"`**, passed inline via `--settings`
    (`agent::PEER_INBOUND_SETTINGS`). Without it the CLI decides per message
    from the two sessions' permission-mode classes, and the assistant
    (`bypassPermissions`) never matches an agent (prompting): a message to an
    agent is held for approval, and one to the assistant expires after
    `dialogExpiry` because a `-p` session has no TTY to show the dialog on.
    A `--settings` the user put in `extra_args` wins over Podium's.
  - **A Podium-stamped session name**, `podium-<tag>` where the tag is the
    first 8 hex of the session's id (`-n`, `agent::peer_session_name`). The
    assistant's tag is a random per-app-run UUID prefix, not a fixed literal
    any local session could name itself. Sender verification is
    `Orchestrator::verify_peer_sender`, exposed as the **`verify_sender`**
    MCP tool: it resolves a sender name against Podium's own process registry
    (live agent sessions) plus that tag, and answers `fleet: false` for
    everything else. It is deliberately **not** a secret the model compares —
    a value in the model's context can be leaked or argued about by a
    prompt-injected message; the authority here is Podium's Rust layer,
    reached over the bearer-token-authenticated MCP channel, and nothing the
    message asserts about its own origin is consulted. `verify_sender` is
    classified read-only so the autonomy gate can never park a security check
    as a proposal. Note what this is *not*: inbound peer messages arrive on
    the CLI's own inbox socket (`/tmp/cc-socks`, owner-only), never through
    Podium, so Podium cannot filter them before the model sees them —
    the OS-user boundary remains the real trust boundary, and `verify_sender`
    is what tells Podium's fleet apart inside it.
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
  choice rather than a toast. A scratchpad is **named after its content**: the
  title is the first usable line (heading/bullet/quote markers stripped,
  truncated to 60 chars on a word boundary), refreshed on every content
  update, with `Untitled scratchpad` until something is written. Setting a
  title (`update_title`) pins it, so later content edits leave it alone;
  a blank title hands the name back to the content. Scratchpads stored with
  the old `MM-DD-HH-MM Scratchpad` title adopt their content's name on load
  (`adopt_auto_titles`, idempotent — a hand-picked name is left alone).
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
  runs `platform::git_command()` — natively a git path resolved once via
  the login shell, then reused (not a login shell per call, which was slow on
  heavy shell profiles); in WSL-backend mode the distro's git via `wsl.exe -e
  git` — rather than through the shell. `git worktree list
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
  `exited` / `queueChanged` / `queueNotice` / `queueItemDelivered`) published
  on the `EventBus` and forwarded to the webview as `assistant:event`.
  Unparseable lines (a chatty shell profile) are
  skipped, stderr is dropped, and the binary + cwd are injectable
  (`AssistantConfig`) so integration tests drive a fake `claude` script. The
  assistant never edits code itself: `Edit`/`Write`/`NotebookEdit`/`Bash` are
  refused at the CLI regardless of permission mode, and implementation work
  goes out to a spawned agent (its `--append-system-prompt` identity says so).
  That identity also leans hard towards **acting rather than checking in**:
  a request that plainly implies dev work is spawned straight away instead of
  proposed first, and a **to-do is only for work that runs long or is meant
  for later** — a small request that can be done now goes into the agent's
  prompt with no to-do at all. Asking is reserved for what it genuinely
  cannot tell; everything else takes the sensible default and reports what it
  did. It is also told to stay a **pure orchestrator** (shell/file work and
  lookups go to `spawn_agent`, never its own Agent-tool subagents, which have
  no shell here), to keep agent updates to warnings and done-ness, how to stop
  agents safely (only the project in question; check the output for follow-up
  work and complete the to-do first), and how to get a peer-refused external
  write done (a fresh agent carrying the authorisation and `--allowedTools`).
  All of that is wording, not a gate — pinned by unit tests so a later prompt
  edit cannot quietly drop it.

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
  comes back as `--resume`, so the conversation continues *within* a run
  instead of meeting a stranger; a session that dies without ever announcing
  itself makes Podium drop the stored id, so a pruned session cannot wedge
  every later turn. **An app launch, however, deliberately starts blank**:
  `src-tauri`'s `setup` calls `assistant_new_conversation` before the webview
  can ask for the transcript (`block_on`, not a spawned task — a task would
  race that call and replay a conversation the panel is about to drop), so a
  transcript from days ago is never dumped into a fresh panel and any nudge
  backlog from an earlier run goes with it. That makes `assistant-memory.md`
  the *only* continuity between days, which is why the identity prompt now
  insists on keeping it current. The panel's **new-conversation** button
  (`assistant_new_conversation`) kills the session and drops transcript,
  session id, pending proposals and the message queue — that kill is
  deliberate, so it is not reported as `exited` (any messages the queue was
  still holding are instead reported undelivered as the first row of the
  now-fresh transcript).

  **Message queue**: only one turn is ever in flight. `AssistantManager`
  pairs the session with an `in_flight` flag and one
  `VecDeque<PendingInput>`, all under the *same* lock — so a queued item can
  never slip past a turn that starts between two lock acquisitions. The
  proactivity/`report_to_assistant` nudge drain (`nudge`) and a proposal
  outcome (`notify`) always route through one `deliver_or_queue`: idle with
  nothing already ahead of it → written immediately, exactly as its caller
  already expects; otherwise it waits, FIFO. The user's own turn (`send`)
  goes through the same path, so a message typed mid-turn waits instead of
  landing mid-answer. It can still be pushed into the running turn on
  demand — `assistant_queue_send_now` / `send_pending_now`, the **send now**
  action on the waiting row — which writes it straight into the live
  session, interleaved output and all. That replaced a `queue_messages`
  setting deciding the same thing for every message in advance: the choice
  only ever matters for one message at a time, when you are looking at it.
  A nudge or a proposal-outcome notice has no such escape hatch — their
  ordering is Podium's business.
  `TurnEnd` (`drain_next`)
  delivers exactly one waiting *user* item as its own turn — never merged —
  and a `TurnEnd` carrying an error still advances the queue. **Consecutive
  nudges are the exception**: they go out as one turn, and a nudge whose text
  is already waiting is not queued a second time. Both exist because Podium's
  own status lines used to arrive one per turn: the 6-hourly sweep repeats
  itself verbatim, so a day without chatting stacked the same line several
  times over, and the user's own next message then queued up behind the whole
  backlog — they watched Podium narrate yesterday before it answered today.
  Read together those lines are a summary of where things stand, which is
  what they are for; user messages stay unmerged, since those are separate
  thoughts. Only a *user*
  message is editable/removable (`assistant_queue_edit` /
  `assistant_queue_remove`, `kind: "user"` only); a nudge or a
  proposal-outcome notice is Podium's own text, delivered verbatim. The
  queue caps at the newest 20, dropping the oldest with a `queueNotice`.
  `assistant_interrupt` kills the running child — there is no documented
  stdin control message for interrupting `--input-format stream-json` mode —
  and respawns it (picking the conversation back up via `--resume`),
  immediately delivering whatever is waiting rather than for a `TurnEnd`
  that would never come; with nothing running it is a no-op. The queue is
  **not persisted**, matching `proposals`: a session ending for any reason
  (crash, `new_conversation`, shutdown) drops it and reports which messages
  were never delivered via `queueNotice`. The frontend gets the whole
  snapshot on every change (`queueChanged`), plus `queueItemDelivered` when a
  *waiting* message's turn comes up — the very first, immediately-delivered
  message needs no such event, since the sender already shows it itself.
  `assistant_send` (and `assistant_day_start`) therefore **return whether the
  message was delivered rather than queued**, and the panel adds its chat
  bubble only on `true`. It cannot work that out on its own: a turn Podium
  started for itself — the external-sources poll, a nudge — publishes nothing
  the panel can see, so its `busy` flag reads idle while the session is busy.
  Guessing from `busy` meant a bubble for a message the backend had in fact
  queued, and `queueChanged` then added a waiting row for the same text: the
  user's message on screen twice, with the poll's unrelated answer landing
  between them. For the same reason `drain_next` publishes the shrunken
  `queueChanged` *before* `queueItemDelivered`, so no render has the message
  in both lists.
  In the panel the busy row and the waiting rows sit together in one
  **sticky** block pinned to the bottom of the transcript (`.tail`), waiting
  rows *below* the "thinking" indicator: what is about to happen should not
  scroll out of sight while you read back. It renders only when something is
  running or waiting, so an idle panel looks exactly as it did.

  **Attachments** ride along with a turn (`assistant_send`'s `attachments`,
  `podium_core::Attachment`). An **image** always travels as its own inline
  `{"type":"image","source":{"type":"base64",…}}` content block — the CLI's
  stdin schema for `--input-format stream-json` is not documented, so this
  was verified empirically against the real CLI (an inline PNG it described
  correctly), not assumed. Anything else is handed over as an **absolute
  path** for the assistant's own `Read` tool (its cwd is the app data dir, so
  a relative path is refused), or — for something that only exists on the
  clipboard — as **inline text**. `resolve_attachments` reads an
  image-looking path from disk and promotes it to an inline image, and
  refuses individually (a `queueNotice` naming the file, never its content)
  anything past 5 attachments per message, an image over 5 MB, inline text
  over 100 kB, or a relative path — the rest of the message still goes.
  An accepted image is then **shrunk** (`compress_image`) to at most 1568px
  on its long edge — the size Claude's vision pipeline downscales to anyway,
  so the extra pixels only cost upload and memory. It runs on what the cap
  already accepted, so the 5 MB limit still judges what *arrived*. A JPEG
  stays a JPEG (re-encoding a photo as PNG would grow it), anything else
  becomes PNG, and an image already within the limit travels byte-for-byte.
  **A GIF is never re-encoded** — this crate decodes a single frame, so
  re-encoding an animated one would silently drop the animation; a GIF is
  only ever subject to the plain size cap. An image that fails to decode is
  sent as-is rather than refused: the CLI may understand a format the
  `image` crate does not, and losing the user's attachment to our own
  decoder is the worse failure.
  `turn_content` folds path/text attachments into the turn's own text block,
  and a turn with no attachments serialises byte-for-byte as it did before
  they existed. Attachments survive queueing: `PendingInput` carries them,
  and `QueuedMessage` reports only their names (`AttachmentInfo`), never the
  bytes the frontend already holds. The composer takes them three ways — drop
  on the panel, paste into the composer, or the paperclip picker
  (`@tauri-apps/plugin-dialog`) — and shows a removable chip per attachment.
  Drop/paste give bytes but no path, so a binary file arriving that way is
  reported by name rather than vanishing; the picker gives a path, which is
  why a large PDF should go through it.

  A **user turn's transcript row is appended before the turn is written** to
  the child's stdin (`record_before_write`): the stdout task records the
  reply on its own task, so appending afterwards let a fast reply land
  *above* the message it answers, and a restart replayed the conversation out
  of order. It is idempotent, since a failed write re-queues the same item.

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
  exited (finished or crashed), an agent the UI reported an input prompt on
  (`process_report_waiting` → `Orchestrator::report_agent_waiting` /
  `agents_awaiting_input`), a running agent that has printed nothing for
  5 minutes (so it is probably waiting on a question — `Orchestrator::idle_agents`,
  measured from the scrollback's last append, or from the process start when it
  never printed), and a 6-hourly sweep for to-dos open longer than a week.
  The prompt report and the silence timer share one dedupe record, and a
  reported prompt wins the wording, so a blocked agent is announced once —
  at the next tick rather than five minutes later. The silence timer stays
  as the backstop for agents whose terminal has never been opened, where
  there is no screen to read.

  That record then **escalates rather than repeats**: an agent still blocked
  `escalate_after` later (default 10 minutes) is raised again, but with
  wording that asks the assistant to say which answer it would pick and why —
  something the user can wave through in a word — and, once that answer is
  agreed, to call `respawn_agent` (nothing can type into an agent's
  terminal). It is kept **for as
  long as the agent lives**, not only while it looks stuck: an agent that
  prints one line and falls silent again is waiting on the same unanswered
  question, and re-reporting it from scratch was exactly the "it just keeps
  asking me the same thing" repetition the escalation replaces. Only the
  agent exiting (or vanishing) clears it. Each becomes one `[Podium] …`
  turn plus a `notice` event, so the chat says why the assistant is speaking
  unprompted. Timings are injectable (`ProactivityConfig`) so tests run in
  milliseconds. A nudge **starts the session when there is none**
  (`ensure_session`, the same path `send` uses) — otherwise the user only
  learned an agent had finished whenever they next happened to type, which is
  how a weekend's worth of `[Podium]` lines came to arrive in one pile. This
  reverses the original phase-4 rule; the reasoning behind that rule ("a fresh
  session would not know what the nudge is about") no longer holds now that
  every launch starts blank and `assistant-memory.md` carries the context. A
  spawn that fails falls back to queuing, so a nudge is never lost. Switching
  the setting off silences all of it.

  **Unblocking a stuck agent** is `respawn_agent`
  (`Orchestrator::respawn_agent`): it stops the agent and spawns a fresh one
  carrying the answer in its prompt. Nothing can type into a running agent's
  terminal, so a session blocked on a question is otherwise dead weight, and
  a plain `stop_process` + `spawn_agent` pair would lose the one thing the
  caller cannot reassemble — the **to-dos the old session owned**, which are
  what carry the task over. Those (and the adapter) are inherited; the
  replacement is deliberately *not* given the old name, so the two sessions
  are told apart in the sidebar. An agent that owns no to-do has nothing to
  inherit, so `task` must be supplied as well or the call is refused rather
  than burning a session on an answer to a question nobody asked it. It is
  classified **destructive**, so `confirm-destructive` (the default) shows
  the user a card first.

  Because these are event-driven — Podium already sees an exit or a silent
  agent for free — **a quiet day costs no turns at all**.

  Alongside that runs a slow lane: the **external sweep**, every
  `poll_interval_minutes` (default 10; `0` is off), asking the assistant to
  check the enabled sources for anything needing the user. Slack and Teams
  need no integration — the assistant already inherits the user's own
  user-scope MCP servers — so this is `poll_prompt` wording plus a timer, and
  it is skipped entirely with both sources off or proactivity off. It is
  delivered as a `Notice`, not a `Nudge`: a nudge renders a muted `[Podium]`
  row on arrival, and a routine check doing that every 10 minutes would
  recreate the very pile-up it exists to prevent.

  **The silence contract**: a sweep that found nothing must say nothing. The
  prompt instructs the assistant to answer with exactly `NOTHING_TO_REPORT`,
  and a `Message` carrying that sentinel **on a line of its own** — anywhere
  in the reply — is dropped before it is recorded or published: no chat row,
  no transcript row. It is deliberately not an exact-whole-message match:
  models keep prefixing the word with the very summary the prompt forbids
  ("Same messages as last check.\n\nNOTHING_TO_REPORT"), and matching exactly
  let all of that through as a notification — the noise this contract exists
  to prevent. A reply that quotes the sentinel *mid-sentence* while reporting
  something real is still a real reply.
  That is also what makes notifications right with no frontend code: the
  panel already pings once per `turnEnd` that produced a reply, so a
  swallowed reply is a turn that produced none.

  **The desktop-notification gate** is its mirror image. A reply the user
  asked for pings when the panel is not in front of them, as it always did;
  a turn *Podium* started (an agent report, a nudge, the sweep) only pings
  when the assistant ends it with `ACTION_REQUIRED` on a line of its own —
  the marker the identity prompt asks for when the user has to decide,
  unblock or look at something now. Without it a steady trickle of "agent
  finished" would reach the desktop, which is the spam the user asked not to
  get; with it, an agent stuck on a question still gets through. The marker
  is **stripped from the prose** (`take_action_required`) before anything is
  recorded or published — it is a signal to Podium, not something to show —
  and a reply that is *only* the marker is dropped like a swallowed one.
  Both halves are folded into `TurnEnd { notify }` in the stdout task, the
  only place that knows both who started the turn (`SessionState`'s
  `turn_from_user`, set at write time so a **send now** makes a Podium turn
  the user's too) and whether the marker appeared; the panel just obeys the
  flag alongside its existing "is the pane in front of you" check.

  The **day-start summary** used to be a fourth trigger here and is now the
  panel's **"Start my day" button** (`assistant_day_start`, shown in the
  empty state). A nudge never spawns the CLI, so as a nudge it only surfaced
  whenever the user next happened to type — which is exactly the complaint
  that moved it. The button sends `DAY_START_PROMPT` through the ordinary
  `assistant_send` path instead: it spawns the session, shows as a user turn
  and respects the queue setting. It asks for open work across all projects
  *prioritised with a reason*, and to ask the user rather than guess when the
  to-dos and agents do not settle the order. There is deliberately no
  once-a-day lock — pressing it is explicit, so refusing would only annoy.
  The watcher holds
  only a weak reference, so it ends with the orchestrator. **Settings →
  Assistant** holds this toggle plus the autonomy mode (the chat header keeps
  its own autonomy picker — both mirror the one stored value).

  A pending proposal is also dropped when **its project closes**
  (`invalidate_proposals_for_project`, keyed off the call's `project_id`
  argument): confirming it later could only fail, so the card resolves itself
  with the reason and the assistant is told.

### WSL-backend mode (Windows)

`backend.json` (`backend_settings.rs`) holds an optional WSL distro. Setup
reads it **first thing** (`init_backend` in `src-tauri/src/lib.rs`) and fixes
`platform::init_wsl` before the MCP server, the assistant or any project can
spawn; changing it needs a restart. `platform::wsl()` is the switch:
`Some(&WslBackend)` in WSL mode, always `None` off Windows (`init_wsl`
ignores a configured distro there).

- **Not behind `cfg(windows)`.** Call sites branch at runtime on `wsl()`, so
  macOS/Linux CI compiles, lints and tests the WSL code. Pure helpers take
  the mode as a parameter (`wsl_login_shell`, `worktree_path`,
  `WslBackend::for_tests`) so both modes are unit-testable anywhere; tests
  never call `init_wsl` (it is process-global).
- **Shell, quoting and probe.** `login_shell()` is `wsl.exe -d <distro> -e bash
  -lic`; `login_shell_in(cwd)` adds `--cd <linux path>`. `quote_arg` is POSIX
  (`quote_posix`), since bash parses the string. `command_exists_query` is
  `command -v`. Agents skip the Windows direct launch in `pty.rs` and take the
  shell path, like Unix.
- **Paths.** Podium keeps Windows `PathBuf`s everywhere
  (`\\wsl.localhost\<distro>\…`, `C:\…`). Anything handed to a backend
  process goes through `platform::backend_path` (`wsl_path::to_linux`:
  UNC → `/…`, `C:\x` → `/mnt/c/x`); anything a backend process prints goes
  back through `from_backend_path` (`to_windows`). `git worktree list` paths
  are matched in the Linux spelling and rebuilt under the project's own path,
  so a `\\wsl$\…` project keeps its spelling.
- **cwd and env.** A PTY child's cwd rides on `--cd`, while `wsl.exe` itself
  starts in the local temp dir (never the UNC path). Environment variables
  reach Linux **only** when listed in `WSLENV`: `pty.rs` appends `KEY/u` for
  every `spec.env` key and `TERM` (`platform::wslenv_with`). Add any new
  per-process variable to `spec.env`, not straight to the command.
- **One `wsl.exe` start at a time.** `wsl.exe` can't take a burst of
  concurrent starts: with three or more at once, all but two hang ~30 s and
  fail (`Wsl/Service/E_UNEXPECTED`), which the startup burst of adapter
  probes and per-project git calls hit (and a failed probe is cached for the
  run). So every launch goes through a process-wide gate, held only for the
  *start* (spawn plus a 150 ms settle), never while a command runs:
  `platform::run_gated` for short commands (`run_shell_*`, git — the wait for
  output happens outside the gate), `wsl_launch_guard`/`spawn_gated` for
  long-lived ones (PTY, assistant; held until the PTY reader runs). Waiting
  for the gate gives up after 10 s with a warning and launches anyway, so one
  hung `wsl.exe` cannot stall everything behind it. **Never take the gate
  under the orchestrator's `inner` lock**: `do_start` takes it first and
  hands it to `PtyProcess::spawn` by reference, so it drops after `inner`.
  New code that starts `wsl.exe` must use one of these. A no-op natively.
- **No console flash.** A console-subsystem child (git, `wsl.exe`, `cmd.exe`,
  bash) spawned via a plain `Command`/`tokio::process::Command` from
  release-mode Podium (a GUI-subsystem app, no console of its own) gets its
  own new console window on Windows unless told otherwise — visible for the
  call's lifetime. `platform::hide_console_window` sets `CREATE_NO_WINDOW`;
  every non-ConPTY Windows spawn in this crate uses it (`run_shell_ok`,
  `run_shell_stdout`, `git_command`, the WSL networking check and distro
  listing, the assistant's CLI child — a tokio `Command`, via `as_std_mut()`
  — and Warp's `cmd /C start`). ConPTY-attached spawns (`process::pty`) are already windowless
  and need nothing. New code that spawns a plain `Command` on Windows must
  use it too.
- **git** runs as `wsl.exe -d <distro> -e git` (`platform::git_command`), the
  distro's git without a shell.
- **Stop.** Killing `wsl.exe` (the ConPTY child) takes the whole Linux session
  with it, children included (`stopping_a_service_ends_its_linux_children`),
  so no `setsid`/PGID bookkeeping is needed.
- **MCP.** The server still binds `127.0.0.1` on Windows; agents in WSL reach
  it through **mirrored networking**. `backend_wsl_networking_mode` lets the
  app warn at startup when it's off. The install registers
  `/mnt/c/…/Podium.exe mcp-bridge` with the distro's CLIs, which start the
  bridge through interop.
- **Tests.** `crates/podium-core/tests/wsl_backend.rs` runs the real thing
  (ConPTY, `wsl.exe`, a distro). It is Windows-only and opt-in: every test is
  `#[ignore]`d, so CI compiles it and reports each test as *ignored* — CI
  never exercises WSL itself. Run it on Windows with
  `PODIUM_TEST_WSL_DISTRO=Ubuntu cargo test -p podium-core --test wsl_backend -- --ignored --test-threads=1`.
  One thread, because a burst of concurrent `wsl.exe` starts fails; without
  the variable the run fails instead of passing vacuously, and the localhost
  test skips with a message unless networking is mirrored. It covers
  cwd/env, argv round-trip, stop, config paths, the localhost reachability
  prerequisite, worktrees and interop.

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
| `process_report_waiting` | `{ processId }` (agent showing a prompt)   | –                 |
| `process_activity`      | –                                           | `Record<ProcessId, OutputActivity>` (`{ quietMs, burstMs }` per running process that printed) |
| `process_rename`        | `{ processId, name }`                       | `ProcessInfo`     |
| `process_start`         | `{ processId }`                             | –                 |
| `process_stop`          | `{ processId }`                             | –                 |
| `process_restart`       | `{ processId }`                             | –                 |
| `process_write`         | `{ processId, dataB64 }`                    | –                 |
| `process_resize`        | `{ processId, cols, rows }`                 | –                 |
| `process_attach`        | `{ processId, channel: Channel<TermEvent> }`| –                 |
| `backend_settings_get`  | –                                           | `BackendSettingsDto` (`{ wslDistro, activeWslDistro }`, empty = native) |
| `backend_settings_set_wsl_distro` | `{ distro? }` (blank = native; must be installed — checked against `wsl.exe -l -q`; applies on restart) | `BackendSettingsDto` |
| `backend_wsl_networking_mode` | – (WSL mode only; the app warns when not `mirrored`) | `string \| null` |
| `adapters_list`         | –                                           | `AdapterInfo[]`   |
| `agent_spawn`           | `{ projectId, adapterId?, name?, prompt?, todoIds?, scratchpadIds?, worktree?, worktreeName?, args? }` | `ProcessInfo` |
| `agent_settings_get`    | –                                           | `AgentSettingsDto` |
| `agent_settings_set_adapter` | `{ adapterId, command?, defaultArgs }` | `AgentSettingsDto` |
| `agent_settings_set_default_adapter` | `{ adapterId? }` (blank clears) | `AgentSettingsDto` |
| `agent_settings_set_merge_mode` | `{ mode }`                       | `AgentSettingsDto` |
| `agent_settings_set_suggest_worktree` | `{ enabled }`             | `AgentSettingsDto` |
| `agent_settings_set_remember_sessions` | `{ enabled }`            | `AgentSettingsDto` |
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
| `assistant_send`        | `{ text, attachments }` (`Attachment[]`)    | `bool` — delivered, not queued (reply streams as `assistant:event`) |
| `assistant_day_start`   | –                                           | `bool` — as `assistant_send` |
| `assistant_transcript`  | –                                           | `TranscriptEntry[]` |
| `assistant_new_conversation` | –                                      | –                 |
| `assistant_memory_get`  | –                                           | `string`          |
| `assistant_memory_set`  | `{ content }` (blank clears)                | –                 |
| `assistant_autonomy_get` | –                                          | `AutonomyMode`    |
| `assistant_autonomy_set` | `{ mode }`                                 | `AutonomyMode`    |
| `assistant_proactivity_get` | –                                       | `boolean`         |
| `assistant_proactivity_set` | `{ enabled }`                           | `boolean`         |
| `assistant_polling_get` | –                                           | `AssistantPolling` |
| `assistant_polling_set` | `{ intervalMinutes, slack, teams }` (0 = off) | `AssistantPolling` |
| `assistant_resolve_proposal` | `{ id, approved }`                     | – (outcome streams as `assistant:event`) |
| `assistant_queue_edit`  | `{ id, text }`                              | –                 |
| `assistant_queue_send_now` | `{ id }` (waiting user message)          | –                 |
| `assistant_queue_remove` | `{ id }`                                   | –                 |
| `assistant_interrupt`   | –                                           | – (queue drain streams as `assistant:event`) |
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
> (`notStarted`/`running {pid, since}`/`stopping`/`exited {code, crashed, stopped, …}`),
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
terminals on theme change. The **activity indicator** is derived once a
second by `agentActivityStore`'s monitor. "Working…" (pulsing dot) comes from
the backend: `process_activity` reports, per running process, how long ago
it last printed and how long that burst of output has lasted, read off the
core's scrollback (`ScrollbackBuffer::output_activity`, bursts split by a 1s
gap) — so it covers agents whose terminal was never opened here. A process
reads as working when it printed within the last 2.5s **and** the burst has
lasted ≥300ms: a lone redraw (Claude Code repainting after the pane is
fitted, a prompt being drawn) is a burst of ~0ms, while a spinner keeps one
growing. The previous signal was the frontend's own stream timestamp, which
did not exist for an unopened terminal and which the attach snapshot and the
resize repaint both bumped — so every open showed "working…" for a few
seconds. A quiet screen showing a permission/confirmation prompt or a
credential request (`sudo` password, ssh passphrase) reads as **"needs
input"** instead — that half still reads the xterm viewport, so it needs an
opened terminal — derived for **every** running process, not just agents, so
a terminal stalled on `sudo` surfaces the same way; non-agents only show that
state ("working…"/"idle" is noise for a plain shell).

`notifyAgentWaiting` (`src/lib/notify.ts`) has a sibling, `notifyAssistant`,
for the same toast+OS-notification+sound treatment on the personal assistant.
`assistantStore.apply` (`src/state/assistantStore.ts`) fires it in exactly
two places, both gated on `isAssistantVisible()` (focused window **and**
`useLayoutStore.getState().assistantOpen`) so a user already watching the
pane is never pinged: once at `turnEnd` when the turn produced a reply (the
*last* `message` event streamed that turn — tracked in `turnReplyText`,
reset whenever a new turn starts, so three streamed blocks make one
notification, not three, and a turn with no reply or an error makes none),
and once when a `proposal` event arrives (the assistant waiting on
confirm/reject, same shape as an agent waiting on a permission prompt). A
proactive nudge (`notice`/`nudge` row) never itself notifies — only the
reply it may provoke does, through the same `turnEnd` path.

The composer (`AssistantPane.tsx`) supports shell-style history recall:
arrow-up/down walk the sent `user` rows already in `messages` (no separate
persisted history — they come back for free via transcript `hydrate()`).
Recall only fires when the caret sits on the composer's first line
(arrow-up) or last line (arrow-down, only while already recalling) — checked
by literal `\n`s around the caret (`isCaretOnFirstLine`/`isCaretOnLastLine`),
not visual wrapping, since a plain textarea gives no other way to ask —
so a multi-line draft still lets the arrows move the cursor normally.
Walking past the newest entry restores whatever was being typed before the
first arrow-up; recalling never sends, and editing a recalled message before
sending appends a new entry rather than rewriting history.

**Selection context** (`src/lib/assistantContext.ts` + `src/lib/selection.ts`)
puts whatever the user has selected into the composer as dismissible chips,
sent along as ordinary `text` attachments — the backend already folds those
into the turn, so nothing in Rust knows about selections. Two sources: the
**Cmd/Ctrl+click multi-select** (to-dos, scratchpads and now process/agent
rows, all of them, in pick order), and selected *text*. Merely **opening** an
item is deliberately not context — that is a "show me this" gesture, and
attaching it would send context nobody asked for. The multi-select lives in
`selection.ts` rather than in each subsection's `useState`, because two
readers need it at once: the subsection's "start an agent on these" bar and
the composer's chips. A selected item whose row is gone (project closed,
to-do deleted) simply stops resolving and drops out, so no chip ever names a
bare id.
Text comes from the document's own `selectionchange` plus xterm's
`onSelectionChange` (a terminal renders to a canvas, so its selection never
reaches the DOM). A selection whose anchor — or the focused element — sits
inside the pane (`[data-assistant-pane]`) is ignored: that is the user
reading or editing the conversation, and clicking into the composer collapses
the document selection, which would otherwise discard the very text they are
about to ask about. `selection.ts` is store-only, importing nothing from
`src/state`, so the terminal registry can feed it without closing a cycle.
User-picked attachments are sent before context items, so the explicit one
wins when both would overflow the backend's five-per-message cap.

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
(no Docker) on both macOS and Windows. A group of them depend on Unix-only
commands/shell syntax or process-group semantics that have no Windows
equivalent; those are `#[cfg(unix)]` (compiled out entirely on Windows) or
`#[cfg_attr(windows, ignore = "…")]` (compiled and linted, skipped at run
time) with a comment explaining why. `tests/wsl_backend.rs` is the opt-in
Windows + WSL suite (see _WSL-backend mode_). Frontend tests are Vitest + jsdom +
React Testing Library (`src/test/setup.ts` registers matchers and cleanup);
Tauri APIs are mocked per test file.

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
    round-trip assertion. If a test is unreliable on Windows (a Unix-only
    command, shell syntax, or process-group semantics), prefer
    `#[cfg_attr(windows, ignore = "…")]` over `#[cfg(unix)]`: it still
    compiles and lints on Windows, only skipping at run time.
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
- **Documentation (always-on):** this file is the contributor-facing
  documentation and [`README.md`](./README.md) the user-facing one. A change
  that alters what a user can **see or do** — a feature, a keyboard/mouse
  interaction, a setting, a `podium.yml` key, an MCP tool — updates the README
  **in the same change**, not as a follow-up. Its _Feature tour_, _Settings_
  and _Things you'd otherwise never find_ sections go stale first. Its
  screenshots/GIFs live in `docs/media/` and are the maintainer's to record —
  when a change makes one of them wrong, or a new feature is better shown
  than described, say so rather than replacing the file yourself. Internal-only
  changes (layering, test harnesses, build plumbing) stay in this file alone.

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
- **CI** (`.github/workflows/ci.yml`): on every PR and push to `main`, a
  `macos-latest` job — rustfmt, clippy `-D warnings`, `cargo test
  --workspace` (real PTYs), then typecheck, ESLint, Vitest, and a production
  `pnpm build` — and a `windows-latest` job. Both run rustfmt and clippy
  `-D warnings` over the **whole workspace** (so `cfg(windows)` code in
  `src-tauri` is linted too); only the test step differs, since the macOS job
  already covers `--workspace` — Windows runs `cargo test -p podium-core`
  (ConPTY, minus the Unix-shell-dependent cases). Both stub `dist/` first so
  `tauri::generate_context!` compiles. About 30 podium-core integration tests
  spawn Unix-only commands/shell syntax with no ConPTY equivalent and are
  `#[cfg_attr(windows, ignore)]`; the macOS job still runs them.
- **Bundle build** (`.github/workflows/build.yml`): a matrix over macOS and
  Windows, each building natively (cross-compiling Tauri is painful). macOS
  packages the `.app` + APFS `.dmg` (via `scripts/make-dmg.sh`); Windows
  packages the NSIS `.exe`. Runs on `workflow_dispatch` and any pushed `v*`
  tag, uploading each OS's installer as an artifact.
- **Releasing** is done via the **`release` skill** (see _Git workflow_).
  Do **not** create git tags or releases without explicit user approval
  (invoking the skill + its confirmation step is that approval).
- **Not yet set up:** Conventional Commits, git-cliff changelog automation,
  signed/notarized releases.

## Roadmap

All the originally planned phases are shipped, plus post-v1.0 work
(scratchpads, the app-quit close guard, the Auggie adapter, the retro theme).
This file and the code are the source of truth for current behaviour.
