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
  streamable-HTTP, bearer auth, 38 tools). Zero Tauri
  dependency; unit tests plus real-PTY/MCP integration tests on plain
  `cargo test`.
- ✅ **Tauri IPC layer** (`src-tauri`): **103 commands**, per-attach terminal
  `Channel` streaming (16ms/64KiB batching), a global-event forwarder for
  lifecycle events, persistent recents + workspace list, window-state
  persistence, an app-quit close guard while agents/terminals run
  (`window_confirm_close`), dialog + log plugins, locked-down CSP.
- ✅ **Frontend** (`src/`): project switcher + sidebar (To-dos / Agents /
  Processes / Terminals / Worktrees / Scratchpads / Git, the last opening a Kanban
  board in the work area), xterm.js terminals living **outside
  React** (registry keyed by processId; scrollback survives tab switches and
  StrictMode), snapshot+seq attach protocol with lag re-attach, new-agent
  modal, agent activity ("working…"/idle) heuristic, to-do and scratchpad
  detail views that fill the work area (description/comment thread; scratchpad
  editor with TOC, mutually exclusive with the focused process), settings +
  theme (dark/light/retro) + toasts, a collapsible, resizable right-side
  assistant chat panel (replayed transcript, a Responses list and the sub-agent
  list (`SubagentsSection`) pinned above it, action + proposal cards,
  waiting-message rows with send-now/edit/remove and an interrupt control
  while a turn runs, autonomy picker, memory view, proactive `[Podium]` notices;
  conversation reset lives in Settings → Assistant), Zustand stores, typed
  IPC wrappers over all 103 commands.
- ✅ **Window chrome**: `App.tsx`'s `<header>` is the title bar on Windows
  and macOS (`data-tauri-drag-region` makes the strip draggable). Windows
  turns the native decorations off (`"decorations": false`) and draws its own
  minimize/maximize/close in `WindowControls.tsx` at the top-right
  (`@tauri-apps/api/window`, polling `isMaximized()` on `onResized` to track
  Aero Snap). macOS keeps its decorations with `"titleBarStyle": "Overlay"`
  and `"hiddenTitle": true`, so the native traffic lights (and native
  fullscreen) are drawn over the strip's top-left corner; `.titlebarMac`
  leaves 78px of room for them and the brand is centered. macOS also keeps
  its native *menu bar* (⌘, opens Settings — see the `setup` hook's
  `#[cfg(target_os = "macos")]` block). Linux keeps its native title bar for
  now. `src-tauri/tauri.windows.conf.json` / `tauri.macos.conf.json` are
  platform overrides merged over `tauri.conf.json` as RFC 7396 JSON Merge
  Patch — a JSON *array* replaces wholesale, so each file repeats the whole
  `windows[0]` object. The window-state plugin is registered without
  `StateFlags::DECORATIONS`, otherwise its restore (which runs after
  `setup`) overrides the platform config's decorations.
- ✅ CI (`.github/workflows/ci.yml`): a macOS job — rustfmt, clippy
  `-D warnings`, `cargo test --workspace` (real PTYs), typecheck, ESLint,
  Vitest, production build — plus a Windows job (rustfmt, clippy, `cargo test
  -p podium-core` only) so `cfg(windows)` code is compiled, linted and tested.
  A PR-only `changelog` job requires an `## [Unreleased]` entry, and a
  `release` job turns every merge to `main` into a tagged GitHub Release.

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
├─ scripts/{next-version,changelog}.sh  # CI release helpers (+ *.test.sh)
├─ crates/podium-core/src/
│  ├─ orchestrator.rs          # Orchestrator — THE public entry point (all mutation goes through it)
│  ├─ project.rs               # project open/reload, cwd resolution (traversal-guarded)
│  ├─ config.rs                # podium.yml serde types (deny_unknown_fields)
│  ├─ process/                 # ProcessKind/Status/Spec, pty.rs (engine), scrollback.rs, supervisor.rs
│  ├─ agent/                   # AgentAdapter trait + claude.rs (ClaudeCodeAdapter) + auggie.rs (AuggieAdapter), McpConnectInfo, settings.rs (agents.json), session_memory.rs, cleanup.rs (exited-agent sweep), folder_trust.rs (pre-accepted trust for the assistant workspace)
│  ├─ mcp/                     # built-in MCP server (mod.rs) + the 38 tools (tools.rs) + stdio bridge (bridge.rs)
│  ├─ assistant.rs             # the personal assistant: headless Claude Code child, stream-json in/out, autonomy gate + proposals, transcript/memory store, nudge queue
│  ├─ subagent.rs              # the assistant's sub-agents: kinds, phases, cap + queue (SubagentBook), launch restrictions, nudge texts
│  ├─ responses.rs             # the assistant's Responses list (ResponseItem + persistent ResponseStore)
│  ├─ proactivity.rs           # EventBus watcher turning agent exits / stuck agents / lingering to-dos / a new day into assistant nudges
│  ├─ issues.rs                # the Git board: issues + PRs/MRs of a project's origin repo in Kanban columns (GitHub via `gh`, temporarily self-hosted GitLab)
│  ├─ boards.rs                # persistent Git-board cache (BoardStore → boards.json, CachedBoard)
│  ├─ worktree.rs              # Podium-managed git worktrees (create/list/remove under .podium/worktrees/)
│  ├─ platform.rs              # the OS gap: login shell, quoting, probe, git command, WSL-backend switch
│  ├─ platform/wsl_path.rs     # pure Windows <-> WSL path translation
│  ├─ backend_settings.rs      # backend.json (WSL distro + native-drive opt-in), read once at startup
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

Each project's sidebar shows or hides its To-do's / Terminals / Worktrees /
Scratchpads / Git sections via `ProjectInfo.sections`
(`ProjectSections`, all on by default; partial JSON loads).
Presentation only: hiding never stops processes, deletes data or affects MCP.
Agents and Processes are never hidden. Persisted per project in
`workspace.json` (`WorkspaceEntry.sections`, `None` = defaults) and
re-applied in `project_open`.

Sidebar projects are a **persistent workspace**: `project_open` adds the
project's root to `workspace.json` (app data dir) and `project_close` removes
it; at startup the frontend calls `workspace_list` and re-opens every path via
`project_open`. A path that fails to open (folder moved/deleted, drive not yet
mounted) is *not* auto-pruned — a transient failure must not silently drop the
project — but its error toast offers a "Remove from workspace" action wired to
`workspace_remove` for permanent failures. Each entry is a `{ path, name?,
color? }` object — the optional `name` is a user-set display-name override (set
via `project_rename`), `color` the badge colour (`#rrggbb`, set via
`project_set_color`, validated in core since it lands in a CSS property) and
list position is the sidebar order (set via `project_reorder`); legacy
bare-path-string files are migrated on load. `project_open` re-applies a stored
name override and colour so a renamed/recoloured project comes back that way
after a restart. Opening the same folder concurrently (e.g. React StrictMode's
double-mount firing `restoreWorkspace` twice) dedupes to one project: the
"already open" check and the id reservation happen under one lock acquisition,
atomically, before the async `podium.yml` load.

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
  `AgentLaunchCtx` (project root and its backend, prompt, MCP connect info,
  config `extra_args`) into a `LaunchPlan { command, env }`. Availability is
  probed via the login shell (`command -v`) of the backend the agent would
  run on — cached once per backend per run, and `adapters_list` takes the
  project — so nvm/homebrew PATH edits are honoured. Adding an agent CLI = new module + registry entry; no IPC or
  frontend change. Max **8 active agents per project** (recursion guard —
  agents can spawn agents over MCP).
- **Agent session memory** (`agent/session_memory.rs`, setting
  `remember_sessions` in `agents.json`, default off) builds on _Restart
  resumes the session_ (see To-dos below): `session_memory.json` in the app
  data dir, keyed by project root. `Orchestrator::shutdown()` is the only
  place a session is captured: each still-running agent with an `AgentSession`
  is remembered under that session's **current id** — the Podium-assigned
  `--session-id`, or the one its last restart moved it to — with its name, the
  to-dos/scratchpads it owns, its per-spawn `args_override` and its
  `worktree`/`worktree_name` request (`ManagedProcess.spawn_*`, so the
  restored identity prompt matches), in spawn order (`ManagedProcess.seq`, a
  per-run insertion counter a restart keeps) so a restore keeps the sidebar
  order, all in one write. Before that, each id follows a `/clear` (see
  _Restart resumes the session_); read outside the lock and within 200 ms, and
  no process starts on that exit path. An agent without an `AgentSession` is
  **not** remembered: Auggie (no session flags), and an agent whose own
  `extra_args` pick the session (`--resume`, `--continue`, …) — a restore
  would put Podium's `--resume` next to the user's flag.

  The next `open_project` on that root, with the setting on, restores the
  entries one at a time, oldest first: `front` peeks at the oldest, and only
  once its outcome is known — restored, or skipped — does `remove` drop
  exactly that entry (matched on the whole entry, so a fresh capture of the
  same session survives) in one write. A failed save is logged and never
  cancels the restore. An app that exits part-way (a quit, a crash) therefore
  still has the entry in flight and the rest next time, and `remember_all`
  dedupes on the session id, so a leftover cannot turn into a duplicate. Each
  is respawned through the restart path: `spawn_agent_inner` with
  `AgentSpawn.resume_session_id` sets the new process's `AgentSession.id` to
  the remembered id and hands `restart_launch(…, stored = true)` — the
  `--resume` line, no prompt — to `do_start`, so a resume that crashes within
  `RESUME_FAILURE_WINDOW` gets `resume_failed` and its next start is fresh,
  exactly as for a restart. Before that, an entry is skipped when it is older
  than 14 days (`session_memory::MAX_AGE`, for a project that kept failing to
  open), is not a hyphenated UUID (the file is user-writable and the id lands
  unquoted on the shell line), or `AgentAdapter::session_exists` (disk, or the
  WSL distro's login shell — `Orchestrator::session_stored`, shared with
  `start_process`) no longer finds it: a pruned session would only come back
  as a fresh agent without context or prompt.

  A `worktree` request whose named worktree already exists under
  `.podium/worktrees/` (`worktree::exists`) is dropped, and so is the "suggest
  worktree" offer (`AgentSpawn.suggest_worktree = false`), so the agent
  neither makes a `-2` copy nor asks about one. `exists` matches the slug
  only: a worktree `create` de-duplicated to `<slug>-2`, or another agent's
  under the same slug, is taken as is. An unnamed request cannot be matched
  and is passed on. After the spawn the agent is re-assigned the
  to-dos/scratchpads that still exist, are not done/archived and no live agent
  claimed (re-assigned, not handed over: a composed prompt would land in the
  resumed conversation as a new message).

  `shutdown()` sets `Inner.shutting_down` first, under the lock. From then on
  `spawn_agent_inner` (before it inserts a row) and `do_start` refuse with
  `CoreError::ShuttingDown` under that same lock, so nothing — a restore in
  flight, a supervised restart — launches after the stop sweep; a start
  refused after the insert removes its row again, so no never-started row is
  left. The restore loop stops at the flag. A spawn refused with
  `ShuttingDown`, `ProjectNotFound` or `AgentLimitReached` stops it too,
  leaving that entry and the rest on the list for the next open. With the
  setting off, opening a project drops its list (`clear`), and turning the
  setting off clears every root. The MCP server is started synchronously in
  `setup` so a restored agent never spawns before the MCP connect info (its
  `--mcp-config`, `-n` and identity) is set.
- **Built-in MCP server** (`mcp/`): rmcp streamable-HTTP nested in axum on
  `127.0.0.1:0` (ephemeral port) behind a **per-run bearer token**. Tools:
  `list_projects`, `list_processes`, `get_process_status`,
  `get_process_output` (tail, ANSI-stripped, default 100 / max 2000 lines),
  `spawn_agent` (its `args` param sets the model/reasoning effort for that
  agent, e.g. `["--model", "opus"]`), `start_process`, `stop_process`,
  `restart_process`, `remove_process`,
  `list_todos` (active only; `include_archived: true` appends the archive,
  read after the listing's auto-archive so nothing drops out of both), `add_todo`, `complete_todo`, `update_todo`, `comment_todo`
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
  other nudge), `get_board` / `move_board_item` (the Git board, see _Git board_),
  `verify_sender` (see _Cross-session messaging_), and
  `add_response` (the assistant filing a Responses item — see _Personal
  assistant_; only the assistant route lists or runs it), and
  `spawn_subagent` / `list_subagents` / `answer_subagent` (the assistant's
  sub-agents — see _Sub-agents_; assistant route only, like `add_response`),
  and `ask_assistant` (a sub-agent's question to the assistant; every route
  but the assistant's, run only for a valid sub-agent caller key): 37 tools
  on the assistant route, 34 on the agents' route, 11 on the lookup route. `spawn_subagent { kind, task, name?, args? }` is
  gated by its `kind` (`assistant::requires_proposal_for`): `lookup` never,
  `full` like `spawn_agent` (a missing/unknown kind too), and `args` may only
  be `--model`/`--effort` with a plain value (`subagent::validate_args`).
  `list_subagents` is read-only. `answer_subagent { process_id, answer }` is
  an ordinary state-changing tool (gated only under `propose-all`).
  `ask_assistant { question }` takes no process id: the caller is resolved
  from its key (an `RequestContext` argument, `PodiumTools::caller`), and a
  call without one is refused.
  The same surface is served a second time at **`/mcp-assistant`**
  (`mcp::ASSISTANT_PATH`, same bearer token) for the personal assistant: that
  route is its identity, and `PodiumTools::call_tool` puts calls arriving there
  through the autonomy gate (see _Personal assistant_). A third route,
  **`/mcp-lookup`** (`mcp::LOOKUP_PATH`, same token), is a *lookup*
  sub-agent's: `PodiumTools::new_lookup` lists and runs only
  `subagent::unprompted_podium_tools` (the read-only tools without
  `add_response`/`list_subagents`, plus `report_to_assistant` and
  `ask_assistant`), and
  `call_tool` refuses anything else before dispatch, so a lookup's "no writes"
  holds on the server, not only through the tool names the CLI hides.
  **Caller identity**: the bearer token is shared, so each sub-agent's MCP
  config also carries a per-sub-agent secret as `X-Podium-Caller`
  (`mcp::CALLER_HEADER`; `SubagentRecord.caller_key`, generated at spawn,
  resolved by `Orchestrator::subagent_for_caller` in constant time).
  `call_tool` (`refuse`, before the gate, so a refusal is never a proposal)
  refuses `report_to_assistant` for a sub-agent id unless the call carries
  that sub-agent's own key — on the lookup route any report but the caller's
  own — and, on the agents' and lookup routes, `stop_process` of a sub-agent by
  anyone but itself (the assistant's gated stop and the user's are the ways).
  Any other call carrying a sub-agent's key (bar `ask_assistant`) also counts
  as that sub-agent going on: a `waiting` one is `running` again
  (`Orchestrator::subagent_resumed`).
  The key lives in the sub-agent's 0600 config file under `{app_data_dir}/mcp`,
  which a lookup is denied reading; a project agent with a shell could read it,
  like the bearer token — the OS user stays the trust boundary.
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
    (`agent::peer_inbound_settings`). Without it the CLI decides per message
    from the two sessions' permission-mode classes, and the assistant
    (`bypassPermissions`) never matches an agent (prompting): a message to an
    agent is held for approval, and one to the assistant expires after
    `dialogExpiry` because a `-p` session has no TTY to show the dialog on.
    A `--settings` the user put in `extra_args` wins over Podium's. The
    assistant's sub-agents get neither this nor the rules below: the assistant
    never messages them, and they are denied `SendMessage`.
  - **Go-ahead rules.** A Claude Code agent's `--settings`
    (`agent::agent_session_settings`, built on the same inbound setting)
    also allows `git push`, `gh pr create` and `glab mr create`, so a
    go-ahead the assistant relays by message can run in the existing
    session — no peer message can approve a prompt. `GO_AHEAD_DENY_RULES`
    (deny beats allow) block force, `+refspec`, delete, mirror/prune/all,
    `--receive-pack`/`--exec`, submodules, push options, `--no-verify`, tag
    pushes, URL/`@` destinations, main/master as the destination and any
    quoting, `$`/backtick/brace expansion or a literal `*` (glob refspec)
    in a push, any of the three fed through `xargs` (a prefix allow also
    matches `xargs <prefix> …`, with the arguments out of sight on stdin),
    plus `-R`/`--repo` on `gh`/`glab`, wherever the flag sits (mid-pattern
    `*`; long git options cut to their shortest unambiguous prefix), and
    every way to read a local file into the PR/MR — `gh --body-file`/`-F`,
    `glab --description-file`/`--attach`, `gh --attach` — which would
    publish e.g. an ssh key without a prompt (the prompt's retry is
    `--fill`, or `--body` inline), plus glab's other-repository flags
    `-H`/`--head` and `--target-p…` (not in its docs; denied anyway), and
    glab's `-s`/`--source-branch` (also quoted/escaped): glab
    `--fill`/`--push` pushes `HEAD:<source-branch>` itself, around the git
    push rules. They
    match command text only:
    `git -C … push`, aliases, some flag clusters, an scp-style `host:path`
    destination with no user (`example.com:other.git`, an ssh-config
    alias), an explicit push to another non-main branch (`HEAD:develop`)
    and an implicit push of a checked-out
    main slip past (see the code comment); the identity prompt has the agent
    check it is not on main/master before pushing. The test's matcher
    mirrors CLI 2.1.292 — re-check it on a CLI upgrade. With the user's own
    `--settings` (either form) none of this applies, and the identity
    prompt drops the go-ahead. The assistant's fresh fallback agent is a
    `claude-code` spawn with no `--allowedTools`: Podium's settings already
    carry the allow and deny rules, and with the user's own `--settings` an
    `--allowedTools` would grant a push with no deny list behind it. When
    those `--settings` come from the default agent args or a project's
    `agents.extra_args` (merged in by `spawn_agent`), the fallback has no
    rules either: the push waits for approval in its terminal, and the
    assistant is told to say so.
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
  it. An agent that **exits** keeps its to-do/scratchpad links (only removing
  the process drops them) but loses its colour (`spec.color` cleared in the
  exit handler and kept as `last_color`, to-do row tinted neutral grey); the
  next start (`do_start`) takes `last_color` back if no live agent holds it,
  else picks one the way a spawn does (`pick_session_color`). A link
  to an exited agent no longer counts as **owning**: `AssignedAgent.active`
  (`ManagedProcess::is_active`, also in the `list_todos` MCP output) is
  `false`, the row gets its spawn button back and can join a multi-select
  again, and spawning on it (or `assign_todo`) hands it to the new agent.
  **Exited agents are removed automatically** (`agent/cleanup.rs`, setting
  `remove_exited_agents_after_minutes` in `agents.json` — on disk
  `removeExitedAgentsAfterMinutes`, on `AgentSettingsDto`
  `removeExitedAfterMinutes` — default 5, missing = 5, `0` = never, at most
  1440, refused above that in core): a sweep task (`cleanup::spawn`, started
  in `setup` next to the proactivity watcher, weak orchestrator ref, every
  15 s) reads the setting fresh each tick and calls
  `Orchestrator::remove_exited_agents`. That picks, under one `inner` lock,
  every `Agent`-kind process whose status is `Exited` with `at` (wall clock,
  so sleep counts; a clock that went back reads as not due) at least that
  long ago and no supervised restart pending — never one of the assistant's
  sub-agents (`inner.subagents`), which the sub-agent hub expires itself
  after its linger, keeping the result for the assistant — and takes each
  out with `take_process` — the same path `remove_process` uses (links cleared,
  restart cancelled), then publishes `ProcessRemoved` plus
  `TodosChanged`/`ScratchpadsChanged` per affected project. Terminals and
  services are never touched; a restart puts the agent back to running, and
  its next exit writes a new `at`, so the clock runs from the latest exit.
  The exit nudge is published with the exit, long before the removal; a
  Responses item keeps the names it resolved at add time, so a removed
  process id there breaks nothing (a later `get_process_output` or
  `respawn_agent` on it answers `ProcessNotFound`). The frontend's
  `applyRemoved` already disposes the terminal and clears the focus, so an
  open agent's work area falls back to the empty state. A removed agent can
  no longer be restarted from the sidebar, so its conversation is not
  resumable from Podium after that. `AgentCleanupConfig { tick, minute }` is
  injectable (tests use millisecond minutes).
  **Restart resumes the session**: Claude Code agents launch with
  `--session-id <process id>`, and the adapter's `LaunchPlan.restart`
  (`RestartCommands`, the launch line in parts) spells the two commands a
  manual start of an exited agent can run instead, for whichever session id
  is current (`AgentSession` on the process; `spec.command` is never
  swapped). `restart_launch` picks per start: `resume(<current id>)` (no
  initial prompt) when `AgentAdapter::session_exists` finds the CLI's
  transcript, else `fresh(<new UUID>)` — the original launch under a *new*
  `--session-id`, since the CLI refuses an id it already used and `--resume`
  fails without a transcript; the new id becomes current, so the agent stays
  resumable. The chosen launch (`Relaunch`) is handed to `do_start`, which
  makes its id current under the same lock only once the PTY spawned — a
  concurrent start that loses with `ProcessAlreadyRunning` cannot overwrite
  it. A resumed run that crashes within `RESUME_FAILURE_WINDOW` (5 s) marks
  the id `resume_failed` in the exit handler, so the next start is fresh
  instead of crash-looping on `--resume`. The transcript is `projects/*/<id>.jsonl` under
  `$CLAUDE_CONFIG_DIR` or `~/.claude` — read from disk natively, and on a WSL
  project checked inside the distro through its login shell
  (`run_shell_ok_on`, off the async runtime), since that is where the CLI
  runs.
  **`/clear` is followed**: after `/clear` the CLI goes on under a new session
  id it picks itself, so `AgentSession.id` would still name the pre-clear
  conversation. `AgentAdapter::live_session` reads Claude Code's live
  registry, `<config>/sessions/<pid>.json`, for the PTY child (`LiveProcess`:
  pid, spawn time, cwd) and trusts the record only when its own `pid` matches,
  its `startedAt` is no earlier than the spawn (less 5 s of slack; the spawn
  time is `ManagedProcess.started_wall`, a wall-clock stamp, since an
  `Instant` does not count a laptop's sleep) and its `cwd`, when present, is
  the agent's — so a stale record from a reused pid is ignored. The PTY child
  is the CLI only when the login shell execs it: a lone command, which zsh and
  bash do (verified against a running Podium); not behind a wrapper
  `command_override`, a shell function or fish, nor on native Windows
  (`cmd.exe` is the child) or WSL (Podium holds `wsl.exe`'s pid) — there
  `live_session` finds nothing (or answers `None` outright) and the agent
  keeps the pre-clear id. `read_live_session` takes the new id only when it is
  a valid UUID other than the current one, and only once Claude Code has
  stored the new conversation (`session_exists`; one not yet written to would
  only make the next start fresh); there is no newest-file guess. The read
  happens outside the lock: `live_probe` collects pid, spawn time, cwd,
  adapter, backend and the current id under it, and `apply_live_session` moves
  the session under it again only if the same run (pid) is still on the same
  id, clearing `resumed`/`resume_failed`. It is asked while the CLI is still
  there: in `stop_process` before the signal (a clean exit removes the
  record), in the exit handler on the wait thread, after the resume-failure
  check (a crash or kill leaves the record behind), and in the shutdown
  capture — stop and capture read off the runtime, all agents at once, giving
  up after `LIVE_SESSION_TIMEOUT` (200 ms): a read over 200 ms is skipped
  (logged at debug) and the id kept. The exit handler has no such deadline — a
  hung file system holds back only that agent's `Exited`. An agent that exits
  cleanly on its own (`/exit`) therefore resumes its pre-clear conversation.
  A session flag the user put in `extra_args` (`--session-id`, `--resume`,
  `--continue`, `--fork-session`, `--from-pr`, `=`-forms included) disables
  all of it. The same `AgentSession` is what _Agent session memory_ carries
  across an app restart. Sidebar rows distinguish being **open** in the work area (a subtle
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
  runs `platform::git_command(wsl)` (the project's backend) — natively a git path resolved once via
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

- **Git board** (`issues.rs`): the project row's **Git** header button
  (`GitButton.tsx`, a pill at the header's far end — the one action visible
  at rest, so the hover-only ones reveal to its left without moving it — with
  the open-card count, an accent fill while the board is open and a warning
  dot on error; per-project section toggle "Git board", key `issues`, on by
  default) opens a Kanban of the repo behind the project's `origin`
  remote in the work area (`GitBoard.tsx`, `layoutStore.openBoard`, exclusive
  with the process/to-do/scratchpad). `issues::board` returns a `Board {
  provider, items, columns, source, hint, prCounts }`: issues and PRs/MRs as
  `BoardItem`s, each already in a `Column { id, title, color }` (`BoardItem.
  column` = a column id; `color` is a GitLab hex or a lowercase GitHub option
  name, which the UI maps to `--board-<name>` tokens). **Columns** come from
  the project's own board when the platform has one (`source` = its name),
  else the defaults `open` / `inProgress` (open issue with a linked open PR —
  GitHub `closingIssuesReferences`, GitLab `merge_requests_count > 0` — or an
  in-progress label, plus draft PRs/MRs) / `review` / `done` (closed/merged
  within `DONE_DAYS` = 14). Fixed system ids are reused either way: `open`
  (lead: unplaced issues), `review` (unplaced open PR/MR, shown only when
  non-empty with a project), `done`. *GitLab*: the `/boards` request (a 6th
  parallel call; first board with a label list only, lists by `position`,
  label-less lists skipped) gives `label:<name>` columns; an open card sits in
  the first list whose label it carries. Any `/boards` failure or no usable
  board → defaults, never an error. *GitHub*: a 6th `gh api graphql` call reads
  the repo's first Project v2 (first 100 items) and its `Status` options as
  `status:<optionId>` columns, items matched by number; it needs the
  `read:project` scope, and when stderr says it is missing the board falls back
  to defaults with `hint` = the `gh auth refresh -s read:project` command
  (stderr is only inspected). Any other failure there → defaults, no hint.
  `mine` (the
  default) narrows issues to assigned-to-me and PRs/MRs to authored-by-me;
  the counts (`prCounts`, all-time open/merged in the same scope) come from a
  GraphQL `search … issueCount` on GitHub and `X-Total` on GitLab (`None` when
  GitLab omits it). Each board is seven parallel calls (`std::thread::scope`),
  lists capped at 100. The repo is detected via `detect` → `parse_remote`
  (ssh/`ssh://`/https, `.git` stripped). Credentials come from **setups**
  (`AgentSettings.issues.setups`, Settings → Issues): each an `IssueSetup
  { id, name, provider: github|gitlab, url, account? }`, `url` normalized to
  `scheme://host[:port][/path]` with the scheme **kept** (`http://` for an
  instance without TLS; `https` when omitted) and the path kept minus a
  trailing slash (a GitLab under `/gitlab`); `gitlab_target` cuts that path
  off an HTTPS remote's project path (SSH remotes never carry it). A URL
  copied from the browser is cut at GitLab's `/-/` (and loses `?`/`#`) on
  save; a project path still left in it is not guessed away — the fetch's 404
  says to check the setup's URL. GitLab calls never follow a
  redirect (the token header would go along to the new host); a 3xx is a
  "check the setup's URL" error. A project can pin one setup from the
  board's picker (`issues.projects`, project root → setup id, app-side);
  otherwise `pick` takes the first setup whose host matches the remote, and a
  pinned setup for another host is `IssuesHostMismatch`. There is **no
  zero-config**: with no setup at all `board`/`move_item`/`unassign_item`
  fail with `IssuesNotConfigured` before spawning anything (not even the `git
  remote` lookup), and `issueStore.refresh` does not even call the backend
  then (the Git button shows no warning dot for it) — so a user who never set
  up a board never pays for `git`/`gh` calls on every refresh; github.com
  without a matching setup is `IssuesUnsupportedHost` like any other host.
  GitHub goes through the user's own `gh` login
  (`gh issue|pr list --repo host/owner/repo --search …`, `gh api graphql`,
  binary resolved once by `platform::gh_binary()` and run via
  `platform::gh_command()` + `run_gated` — in WSL-backend mode the distro's
  `gh` (`wsl.exe -e <path>`, `GH_TOKEN` passed through `WSLENV` by
  `platform::backend_env`), so probe and call never use different `gh`
  logins); a setup's `account` picks a
  logged-in `gh` account via `gh auth token --user`, handed to those `gh` calls
  as `GH_TOKEN` — Podium stores no GitHub token. Self-hosted GitLab is fetched
  from the setup's URL (`/api/v4/projects/:path/{issues,merge_requests}`,
  `PRIVATE-TOKEN`, blocking reqwest + rustls; closed/merged lists filtered by
  `updated_after`, then by the real `closed_at`/`merged_at`). **Temporary** —
  Connect still runs on GitLab; removing it is `IssueProvider::Gitlab`, its
  arm in `issues::board` and the `// ponytail: temporary` blocks. **Token
  handling:** a GitLab token lives only in the OS keychain (`keyring`, service
  `com.podium.app.gitlab`, account = setup id; on Linux the Secret Service
  over D-Bus, so building there needs `libdbus-1-dev`); `issues_settings_get` reports
  just `tokenSet`, the token is never returned over IPC, logged, or put in an
  error, and the Settings field is never prefilled. Failures are typed
  (`IssuesNotConfigured` / `IssuesNoRemote` / `IssuesGhUnavailable` / `IssuesUnsupportedHost` /
  `IssuesHostMismatch` / `IssuesGitlabNotConfigured` / `IssuesGitlabAuth` /
  `IssuesFetch`, the last also for rate limits) and never carry `gh`/`git`
  stderr or a GitLab response body; the setup kinds link the board to Settings
  → Issues (`settingsRequestStore`), as does its gear. Refresh is a frontend
  concern (`issueStore`, which also caches the setups overview and refetches
  every shown project when it changes, and holds the per-project Mine/All
  scope — a switch mid-fetch refetches once the old answer lands): one
  in-flight fetch per project, the stale board is kept on error, and
  `startAutoRefresh` (mounted with the Git button, so its "N open" —
  every card not in `done` — tooltip stays live) fetches immediately, every 5 min, and on window focus
  when the last attempt is over 60 s old; errors show inline, never as a
  toast. Those automatic fetches share one pace across projects — starts at
  least 15 s apart (`AUTO_FETCH_SPACING_MS`), since a GitHub board costs ~6
  of GitHub's 30 searches a minute — while a refresh the user asks for skips
  it. A queued auto refresh is not `loading` (it waits *before* that), so a
  manual refresh still runs and the queued one lapses; closing the project
  drops it, and a project stuck on a local error (no remote, unsupported
  host, …) takes no slot, except on a setups change, which paces every shown
  project. A setups change during a fetch reruns it once that fetch is done,
  and a section shown again refetches when its cache predates the last setups
  change. A card click opens a read-only drawer (`IssueDetailPane`) beside the
  columns; browser, "start agent" (`NewAgentModal` prefilled via
  `initialPrompt`) and "turn into to-do" (`todoAdd` → `todoUpdate` →
  `todoAddLink`, hidden while an active to-do already links the URL) are card
  actions. GitLab MRs are referenced as `!n`, everything else `#n`.

  **Cache** (`boards.rs`, `BoardStore`, like `TodoStore`): `Orchestrator::
  issue_board` stores every successful board in one `boards.json` (app data
  dir, atomic writes — serialised under the store lock, written after it is
  released, an older snapshot never over a newer one — keyed by project
  root, per scope `mine`/`all`) as a
  `CachedBoard { board, fetchedAt }`; `cached_issue_board` /
  `issues_board_cached` read it without network, so the UI paints at once. The
  cache also keeps **columns stable**: when GitLab's `/boards` request *fails*
  (not: answers without a label-list board) `fetch_gitlab` places cards in the
  cached board's `label:` columns instead of the defaults. Project boards
  always carry `review` and `done`. A fetch that started before a move or
  unassign of that project landed is not stored (`BoardStore`'s per-root edit
  counter, taken before the fetch); `issue_board` then serves the cache, which
  has the edit, with its real `fetchedAt` (what `get_board` reports).
  Closed issues and merged PRs are always in `done`, also on a GitHub Project
  with a Status set, so `done` means closed on both providers.

  **Drop rules** ride on the wire: `Column.accepts` (`("issue"|"pr")[]`) —
  default board `open` [issue], `inProgress` [], `review` [], `done` [issue];
  project board `open` [issue], every `label:*`/`status:*` [issue, pr],
  `review` [pr], `done` [issue] — and `BoardItem.movable` (false for a PR/MR in
  `done`). **PRs/MRs that close an issue on the same board are folded**: the
  card is dropped and rides on the issue as `BoardItem.linked: LinkedPr[]`
  (`{ number, title, url, draft, merged }`); a PR whose issue is not on the
  board stays a card. GitHub uses `closingIssuesReferences`; GitLab makes no
  extra calls and matches the MR description against the default closing
  pattern (`closes|fixes|resolves|implements` + `#N` / `group/project#N`) plus
  a source-branch segment that is `N`, optionally one letter, then `-…` or
  nothing (`gl_closing`, a heuristic — upgrade path
  `/merge_requests/:iid/closes_issues`).

  **Moves** (`Orchestrator::move_board_item` → `issues::validate_move` then
  `move_item`, blocking): validated against the cached board — of the scopes
  holding the card, the most recently fetched, since its `done` decides
  close/reopen (item there, column exists and accepts the kind, movable;
  without a project board `open` is only reachable from `done`; already in
  that column is a no-op success that syncs both scopes), refusals are
  `CoreError::IssuesMove` (`issuesMove`, Podium-owned text). GitLab: one `PUT`
  on the issue/MR — `add_labels`/`remove_labels` for `label:*` (other list
  labels removed), all list labels removed for `open`/`review`,
  `state_event=close` for `done`, plus `state_event=reopen` out of `done`.
  GitHub: `gh issue close|reopen`; on a Project, GraphQL looks up the first
  project's Status field and the card's item, adds it when missing, then
  `updateProjectV2ItemFieldValue` (or `clearProjectV2ItemFieldValue` for
  `open`/`review`); a missing token scope becomes the `gh auth refresh -s
  project` message. On success both cached scopes are updated (`column`,
  GitLab `labels`; `linked` untouched) and `PodiumEvent::BoardChanged` →
  `board:changed`. MCP: `get_board` (cache if under 5 min old and not
  `refresh`; no bodies; classified read-only) and `move_board_item` (same
  path as the IPC command; **destructive** for the autonomy gate, since it
  closes/reopens issues other people see).

  **Unassign** (`Orchestrator::unassign_board_item` → `issues::
  validate_unassign` then `unassign_item`): each board also carries `me` (the
  signed-in login — a 7th parallel call, `gh api user` / GitLab `/user`, whose
  failure only hides the action) and every `BoardItem.assignees`; the card and
  drawer show "Unassign me" when `me` is among them. GitHub: `gh api --method
  DELETE repos/:repo/issues/:n/assignees` (PRs too). GitLab: a fresh `GET` of
  the issue/MR, then a `PUT` with the remaining `assignee_ids[]` (`0` = none),
  so a co-assignee added since the fetch is kept. On success `me` leaves the
  item's assignees in both cached scopes, an *issue* leaves the `mine` board
  outright, and `board:changed` is published. IPC only, no MCP tool.

  **Approvals**: open PRs/MRs (cards and linked pills) carry `approvals:
  { by, required }`, shown as a `✓ n/required` badge (`approvalBadge`,
  accented when `me` is among `by`). GitHub: `latestReviews` in state
  `APPROVED` (`required` unknown → `None`). GitLab: one GraphQL call after the
  lists (`/api/graphql`, same `PRIVATE-TOKEN`, `mergeRequests(iids:)` →
  `approvedBy` + `approvalsRequired`), whose failure only hides the badges.
  The **search** field (Cmd/Ctrl+F focuses it, Escape clears) filters every
  column by ref, title, labels, author, assignees and linked PRs.
  MR ↔ issue linking also takes a branch segment that is the issue number
  (`issue/12661`, `issue/12661b`, `42-fix`), Connect's convention — not
  `2fa-login`, `release/2026.10` or `hotfix/3.4.1`.

  **GitLab connection retries** (`gitlab_send`): a request that fails before
  any HTTP status (VPN/Wi-Fi not back yet after sleep — the focus refresh
  fires right then — or a dropped connection) is retried twice (500 ms, 1.5 s);
  the error names the reason (`timed out` / `connection failed`), never
  reqwest's text. An HTTP status is never retried.

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
  `exited` / `queueChanged` / `queueNotice` / `queueItemDelivered` /
  `responsesChanged` / `subagentsChanged`) published
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
  did. It is also told to stay a **pure orchestrator** that delegates almost
  everything: anything beyond a reply from what it already knows (a lookup, a
  check, shell/file work belonging to no single project) goes to
  `spawn_subagent`, then it answers in one line and **ends its turn** — the
  result comes back as a `[Podium]` message; project code work still goes to
  `spawn_agent`, routing (to-dos, messages, proposals) stays its own, and it
  never uses the CLI's own `Agent` tool (`Agent` and its old name `Task` are in
  its `--disallowedTools`, beside `Edit,Write,NotebookEdit,Bash`). It is told
  to keep agent updates to warnings and done-ness, how to stop
  agents safely (only the project in question; check the output for follow-up
  work and complete the to-do first), and how to pass on a push go-ahead (a
  message to an existing Claude Code agent, which accepts it once
  `verify_sender` says the sender is the assistant, pushes, opens the PR/MR
  and reports back with its URL; a fresh `claude-code` agent (no
  `--allowedTools`) if it refuses, never reports a push, or is not Claude
  Code).
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
  nothing; `confirm-destructive` gates the stopping/archiving/removing tools
  and `move_board_item`;
  `propose-all` gates everything that is not read-only (an *unknown* tool counts
  as state-changing, so a tool added later cannot slip through). Proposals live
  in memory (never expiring, but lost on restart) and surface as
  `assistant:event` `proposal` / `proposalResolved`; the user's Confirm/Reject
  in the chat calls `assistant_resolve_proposal`, which runs the stored action
  and tells the assistant the outcome as a `[Podium]` turn so it can carry on.
  A failed tool call is never silent: `tool_result` blocks flagged `is_error`
  become a `toolFailed` event, stored under its own `TranscriptRole::ToolFailed`
  (rows from before that replay as `Notice`). Lines carrying a string
  `parent_tool_use_id` are the calls of a CLI-internal `Agent`-tool subagent
  (not one of Podium's _Sub-agents_, which are processes of their own) and
  yield no events — the parent's `Agent` row stands for them. A `result` with `is_error` reports
  its `result` text, not the `success` subtype: a usage limit reads `<CLI
  text>: resumes at HH:MM` (the old `<text with "limit">|<epoch seconds>`
  form, local time; `Fri HH:MM` when not today; anything else stays verbatim)
  or the CLI's own wording; the CLI's synthetic text block carrying that old
  form is dropped, the `result` line reporting it; the store always prefixes
  it, so the panel shows `The turn failed: Claude AI usage limit reached:
  resumes at HH:MM`. The panel folds
  each run of consecutive `tool` / `toolFailed` / `nudge` rows into one
  collapsed `<details>` line (render-only, the transcript is unchanged; a
  `nudge` starts a new run unless the current one already holds one, so a
  user turn's actions are never relabelled as Podium's, and a run with a nudge
  reads fainter as "Podium check ·"). It only follows the tail while scrolled
  to the bottom (opening a run keeps a pinned reader pinned, and sending
  scrolls down directly), else shows a `New activity ↓` button — only when
  a row other than an action/nudge was appended, cleared when the transcript
  shrinks. The gate only sees Podium's own MCP tools,
  so the CLI's own `SendMessage` — the way a push go-ahead is relayed — is
  refused outright under `propose-all` instead: that mode adds it to the
  session's `--disallowedTools`. A CLI flag cannot change in a running
  session, so moving into or out of `propose-all`
  (`Orchestrator::set_assistant_autonomy` →
  `AssistantManager::set_send_message_blocked`) replaces the session,
  resuming the conversation: at once when it is idle (dropped, respawned by
  the next turn), or at the running turn's `TurnEnd` — never mid-turn — via
  the output task, which carries a `SpawnCtx` to start its own replacement
  before the next waiting item goes out (if that spawn fails, the waiting
  items are reported undelivered and `exited` is published, as for any exit
  nobody asked for). So the block applies from the next turn boundary: a
  turn already running when `propose-all` is chosen can still `SendMessage`
  until it ends. The mode loaded at startup is
  applied with `preset_send_message_blocked`. The prompt, which never knows
  the mode, says: if `SendMessage` is unavailable, use the fresh-agent route
  (a `spawn_agent`, which gets the card).

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
  started for itself — the external-sources poll, a nudge — leaves a `nudge`
  row (`PendingKind::Poll` / `Nudge`) but publishes nothing that starts or ends
  the panel's `busy` state or announces the turn as the user's, so `busy` reads
  idle while the session is busy.
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
  of order. A nudge's or sweep's `nudge` row is recorded and published there
  too, since the panel groups a turn's tool rows by the nudge row ahead of
  them. It is idempotent, since a failed write re-queues the same item.

  **Responses** (`responses.rs`) are the short "needs you" / "FYI" points the
  assistant files via the `add_response` MCP tool, only for what the user
  actually needs (the identity prompt's rules, pinned by unit tests — see
  _Desktop notifications come from the Responses list only_ above); Podium itself files one per
  parked proposal; the detail stays in its chat reply. One workspace-wide list, newest first, persisted in
  `assistant-responses.json` (atomic writes, like `todos.json`) so items
  survive a restart *and* the blank conversation every launch starts with —
  they are the user's to clear. Agent, project and to-do names (and the
  agent's colour) are resolved when the item is added, so an item about an
  exited agent still reads right; a live agent's own project wins over a
  mismatched `project_id`, and a context that is already gone (agent exited,
  project closed, to-do removed) is dropped from the item rather than
  refusing it. Agent and project names are capped at 40 chars, the to-do at
  60, the source at 40. A newer item with the same `processId` or
  `todoId` replaces the open one — except that an FYI never replaces an
  open "needs you" (both stay), and an item about an agent is only replaced
  by another agent's item (the same agent, or one on the same to-do, as
  after `respawn_agent`), never by an agent-less one such as the sweep's; an
  item with neither key never replaces anything. `add_response` is disabled in the agent route's tool router, so
  it is neither listed nor runnable there (the list is the assistant's to
  curate) — a guard against a confused agent, not a security
  boundary: both routes share one bearer token, so an agent that deliberately
  posts to `/mcp-assistant` gets through, exactly as it could reach every
  other assistant-route tool; the OS user stays the trust boundary. It is
  classified read-only, so the gate never parks it as a proposal, and capped
  (newest 100 items, 500-char text) since the assistant writes it from
  content it read. The nudges Podium sends name the agent's process id (and
  the lingering-to-do sweep each to-do's and project's id) so the assistant
  can pass them as the supersede key. Every change publishes a full
  `responsesChanged` snapshot, never recorded in the transcript; nor is the
  `add_response` call itself — `parse_stream_line` drops the exact
  `mcp__podium__add_response` tool event, so neither the transcript nor the
  chat shows an action row for it (the list already shows its result). The user either ticks an item
  (`assistant_response_dismiss`, it is just removed) or replies
  (`assistant_response_reply`): the reply is an ordinary user turn through
  `send`'s path (`AssistantManager::send_reply`, so it queues like any
  message) carrying the answered `ResponseItem` itself. The session sees it as
  `[Reply to your Responses item — "<label>": "<item text>"]` + the reply
  (label and text JSON-quoted, since an agent name or to-do text is
  agent-controlled and must not be able to close the bracket), the
  transcript stores it as a `reply` row (`"<label>\n<reply>"`), and a waiting
  reply exposes `replyTo` (the label) on `QueuedMessage` and on
  `queueItemDelivered`; editing a waiting reply changes only its text. The
  item is taken off the list before the send (so a double submit cannot send
  twice) and put back if the send fails — or if a queued reply never goes
  out: dropped by the queue cap, withdrawn by the user
  (`assistant_queue_remove`), or lost with the session (new conversation or
  crash, both also named in the undelivered notice; at app shutdown
  `AssistantManager::shutdown` restores them itself before killing the
  child, since nothing after it is sure to run). That is why the
  `ResponseStore` lives on the `AssistantStore`: the queue code needs it.
  `restore` skips an item a *newer* one has since replaced (an older item
  beside it never counts) and keeps the 100-item cap by dropping the oldest
  other item, never the restored one. `add_response` refuses a `process_id`
  that names a live terminal or service rather than an agent.
  Snapshots are published under the store's lock, so they arrive in the order
  the changes happened, and a write that fails leaves the list (and the
  published snapshot) unchanged. The label is the non-empty parts of agent ·
  project · to-do · source, or `Responses item`; it is computed in Rust
  (`ResponseItem::label`) and added as `label` only at the wire boundary
  (`LabelledResponse`, used by `assistant_responses_list`, the
  `responsesChanged` event and `add_response`'s answer) — never written to
  `assistant-responses.json` — so the frontend never re-derives it. (The
  row shows the same parts with the agent styled apart, dropping blank ones
  as `label()` does; its `nowrap` collapses whitespace inside a part.) The panel's `loadResponses` drops its result when a
  snapshot landed while it was in flight. A half-typed reply is filed under
  its item's id; when that item is replaced, the draft moves to the newest
  item that replaced it (the backend's rule mirrored in `supersedes`), so
  two items side by side never share one. The reply box takes focus only
  when its Reply button is clicked.

  **Sub-agents** (`subagent.rs`, `Orchestrator::spawn_subagent`): work the
  assistant hands off so its turn ends at once. Each is an ordinary interactive
  PTY Claude Code agent process in the assistant's own **scope**
  (`AssistantScope`, `Orchestrator::set_assistant_workspace`, called from
  `setup`): a Podium-internal project rooted at `{app_data_dir}/assistant-workspace`
  that is deliberately kept **out of `Inner.projects`**, so every project call
  (close, rename, sections, to-dos, scratchpads, worktrees, Git board,
  `spawn_agent`, `add_process`, session memory) answers `ProjectNotFound` for
  it, and `open_project` refuses that folder (root compared canonical). It
  runs on the assistant's backend (`platform::wsl()`). `spawn_subagent` is
  refused until the scope is set and while there is no MCP server (the result
  only comes back over MCP).
  - **Kinds and launch** (`subagent::launch_args`, after the user's default
    args so the restrictions win; `args` is `--model`/`--effort` only — both
    flags, the `--effort` levels and the CLI's `Task`→`Agent` alias checked
    against Claude Code 2.1.301). Both kinds load **only the user's own
    settings** (`--setting-sources user`): no project or local settings files
    (hooks, permissions), no project/local `CLAUDE.md` or `.claude/rules` (the
    CLI's walk up the parent dirs included) and no settings-based approval of
    a project `.mcp.json`. *lookup*: `--permission-mode bypassPermissions` (it
    runs in the background, a prompt for an MCP read would only block it), its
    Podium tools over the **lookup route** (above), and `--disallowedTools`
    Bash, Edit, MultiEdit, Write, NotebookEdit, SendMessage, Agent, Task,
    AskUserQuestion plus
    `mcp__podium__<tool>` for **every tool not on the lookup route**, generated
    from the router (`PodiumTools::all_tool_names` minus
    `unprompted_podium_tools`) — a second layer under the route, so a tool
    added later is refused until classified read-only. Its `--settings` also
    deny `Read(//{app_data_dir}/mcp/**)` (the CLI's absolute-path rule
    spelling, `agent::absolute_path_rule`; Read rules cover Grep/Glob, and the
    shell is denied): the token files and every caller key. A lookup **can
    still call the user's own other MCP servers** (Slack, Teams, a ticket
    system) **without a prompt**, as the assistant itself can — it is told to
    change nothing, but Podium cannot enforce that for servers that are not
    its own. *full*: the normal permission prompts, `--disallowedTools
    SendMessage,Agent,Task,AskUserQuestion`, the agents' route (the CLI's own
    question dialog would sit in a terminal nobody watches; questions go
    through `ask_assistant`, below). Both kinds' `--settings` allow
    `ask_assistant` and `report_to_assistant` unprompted. A sub-agent never gets the
    go-ahead push rules or `crossSessionInbound`, has its own identity prompt
    (no project; do the task, then call `report_to_assistant` with two lines
    and whether follow-up is needed; ask via `ask_assistant` only when truly
    blocked on a decision that changes the outcome and cannot be reversed
    cheaply, then end the turn and wait — else pick the sensible default and
    state it in the report; no peer-message note) and no session
    (`AgentSession`): never restarted, never remembered by session memory.
    **A `--settings` in the user's own default args** (Settings → Agents)
    replaces Podium's for sub-agents too (`has_flag`, as for agents): then a
    lookup shows Claude Code's one-time bypass-mode warning in its terminal
    (no `skipDangerousModePermissionPrompt`) and has no deny rule on the MCP
    dir, and a full one asks before its report (no allow rules). The flags in
    `launch_args` still apply.
  - **Shared workspace.** All sub-agents run in the one
    `{app_data_dir}/assistant-workspace`, and before every start
    (`Hub::scrub_workspace` → `subagent::scrub_workspace`) Podium removes what
    an earlier sub-agent may have left at its root that would shape the next
    session — `.claude/`, `.mcp.json`, `CLAUDE.md`, `CLAUDE.local.md` (a
    symlink is removed, never followed). Only a sub-agent can have put them
    there; `--setting-sources user` already ignores all but `.mcp.json`
    (approved anyway when the user's settings carry
    `enableAllProjectMcpServers`), so the scrub is the second layer. One
    folder for all of them keeps the folder trust (below) to one entry.
  - **Cap and phases** (`SubagentBook`, pure bookkeeping): at most
    `MAX_RUNNING_SUBAGENTS` = 4 run, further ones are `queued` (a `notStarted`
    row, never refused) and start oldest-first as slots free; a newcomer never
    jumps the queue. Phases `queued` / `running` / `waiting` / `done` / `failed` /
    `stopped`; an ended phase frees the slot and sticks (a stop after a report
    stays `done`). Admission happens under the same lock as the row insert.
    Ended rows linger 5 min (`SubagentConfig::linger`, held by the book) and
    are then removed with their process; a driver task (`run_subagent_driver`,
    started lazily on the first spawn, weak reference) acts on exits handed
    over by the exit handler and sweeps on a 10 s tick (also reconciles a
    missed exit), publishing the snapshot without them by itself. The linger
    counts on **either clock** (`SubagentBook::expired`): `Instant` stops
    during a macOS sleep, so a row ended before the lid closed would otherwise
    stay up to five awake minutes after the wake; the wall clock alone could
    be set back. Each `SubagentInfo` carries `expiresAt` (`endedAt` + linger),
    and the frontend lets a row go at that moment on its own (below), since a
    snapshot can reach a webview late.
    Timings and the adapter are injectable (`SubagentConfig`).
  - **Ending.** `report_to_assistant` from a sub-agent
    (`report_subagent_result`) → `done` (the result kept on the row up to 16 KiB, a memory cap
    only; `list_subagents` / `assistant_subagents_list` return it whole), a
    `finished_nudge` (carries the result up to 4000 chars via `quote_line`; only
    when clipped it adds a pointer to `list_subagents` / `get_process_output`), the session stopped after a 5 s grace so the tool call
    can finish; a second report changes nothing. An exit without a report is
    `failed` (a `failed_nudge` with the reason); an exit Podium caused is
    `stopped`. A queued one handed a slot by `fill_slots` is started outside
    the lock; a stop landing in between ends it (`subagent_unstarted`), and
    `do_start` re-checks the phase under the lock and refuses an ended
    sub-agent, so no untracked process is left. **Who is told:** the *user's* stop (`assistant_subagent_stop`, and
    IPC `process_stop`, which calls `Orchestrator::user_stop_process` and so
    routes a sub-agent through `stop_subagent`) nudges the assistant
    ("stopped by the user, no summary needed"); the assistant's own
    `stop_process` (gated destructive) is quiet, as is a new conversation that
    clears them. A queued one stopped is cancelled and never runs. These
    nudges go through `quote_line` (name, task and result are agent-written:
    whitespace runs collapse to one space and the text is JSON-quoted, so
    nothing can start a line that reads like Podium speaking; the other agent
    nudges use it too) and are **not** behind the proactivity switch — the
    result is what the user asked for.
  - **`lasting` nudges.** Sub-agent nudges go out via
    `AssistantManager::nudge_lasting`; `new_conversation(keep_lasting)` hands a
    waiting `lasting` item to the new conversation instead of reporting it
    undelivered (when the sub-agents were kept). `assistant_new_conversation
    { stopSubagents }`: with stop, `stop_all_subagents` first marks each ended
    `stopped` under the lock (so a racing report finds it ended and nudges
    nothing), then signals the processes — and runs once more after the old
    session is gone, for a spawn that raced the reset; the `setup` call passes
    `false` (nothing runs yet). Known limit: a `lasting` nudge already written
    into the turn the reset kills is not re-sent (the old session got it, the
    new one never hears of it).
  - **Questions** (`ask_assistant` → `Orchestrator::ask_subagent_question`;
    `answer_subagent` → `answer_subagent`). A running sub-agent that asks
    becomes `waiting` (`SubagentBook::ask`: the question on the row, capped
    at `MAX_QUESTION_CHARS` = 2000, a second ask replaces it; refused for a
    queued or ended one). `waiting` holds its slot (`SubagentPhase::is_active`,
    counted by `running()`), never expires (it is not ended), counts as live
    for the quit guard, the keep/stop dialog and `isSubagentLive`, and is
    skipped by proactivity's stuck checks (the assistant already has the
    question). The tool answers at once that the answer arrives as the next
    message, so the sub-agent ends its turn. The assistant gets a `lasting`
    nudge (`subagent::question_nudge`, the question via `quote_line`) worded
    **answer-first**: answer it yourself with `answer_subagent` whenever the
    task, context, memory or to-dos settle it, and mention it only in the
    usual short report; relay only a decision that is genuinely the user's
    (preference, risk, money, people, anything it cannot infer) as a needsYou
    `add_response` keyed by the process id with a recommended answer — that
    item is what pings the desktop, so a question does not by default. The
    identity prompt says the same (pinned by tests).
    `answer_subagent` is refused unless the sub-agent is `waiting`; it types
    the answer in through the PTY: control characters dropped (no escape can
    steer the terminal), `\r\n`/`\r` → `\n`, a multi-line answer wrapped in
    bracketed paste (`ESC[200~ … ESC[201~`, which Claude Code enables; a bare
    `\n` would insert a line break in its own input box), then `\r` after
    `ANSWER_ENTER_DELAY` (150 ms) — verified on Claude Code 2.1.303 on a PTY
    (both lines land in one message). Then `running`, question cleared. **The
    user answering in the terminal** instead: `write_stdin` carrying a `\r`
    to a waiting sub-agent resumes it, and so does any later tool call with
    its key. A stale `waiting` never blocks anything: a report, a stop or an
    exit ends it as from `running`.
  - **Folder trust.** Podium pre-accepts Claude Code's folder-trust dialog
    for the workspace (`agent::folder_trust`), so no sub-agent sits on it.
    Claude Code records an accepted dialog in its global config file —
    `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`
    (`.claude-custom-oauth.json` with `CLAUDE_CODE_CUSTOM_OAUTH_URL`; read
    from Podium's own environment, like `claude_config_dir`) — as
    `projects["<abs path>"].hasTrustDialogAccepted = true`, keyed on the
    session's physical cwd (`/` separators on Windows) and checked up the
    parent dirs to a git root (Claude Code 2.1.301; verified by starting the
    CLI on a PTY with a temp `CLAUDE_CONFIG_DIR`). `src-tauri` hands the path
    in (`Orchestrator::set_claude_config`, before `set_assistant_workspace`;
    unset, as in every test, nothing is touched). `set_assistant_workspace`
    writes it once and `Hub::scrub_workspace` checks again before every start,
    for a key that went missing. `ensure_trusted` is a read-modify-write of
    that one key for the canonical workspace path: a lock-free read first (key
    already true → nothing written), then under the CLI's own lock (the
    `<file>.lock` directory proper-lockfile makes; tried 5 × 40 ms, never taken
    over) a re-read, the key set (`projects` and the entry created when
    missing), and an atomic replace (temp file beside the real one — a
    symlinked config keeps its link — with its permissions, then a rename).
    serde_json's `preserve_order` + `float_roundtrip` and the CLI's own layout
    (two-space indent, the original's trailing newline or none) write every
    other key back byte for byte. Never created: a missing file means the CLI
    never ran with that config (or Podium resolved another one), and the CLI
    restores a missing config from its backups, which a file holding only
    Podium's key would pre-empt. A missing file, malformed JSON, an
    unexpected shape, a held lock or a failed write leaves the file alone and
    logs a warn without content; a start never fails on it. **WSL-backend
    mode** skips it (the CLI's config is inside the distro); there, and
    whenever the write did not happen, the detection below still applies.
    *Fallback detection:* for `trust_window` (30 s) after a start Podium
    watches its output (`shows_trust_prompt`: header, "Yes, I trust this
    folder" and "No, exit", no permission prompt, session not yet started)
    and reports it like a prompt seen on screen (`report_agent_waiting`), so
    proactivity raises it at once. Podium never answers it: the dialog
    focuses "No, exit" and ignores an early keypress, an Enter would end the
    session. The stuck-sub-agent nudge says the user answers it once, from
    the sub-agent list.
  - **Proactivity.** A sub-agent exit is not announced as an agent that
    "finished" (`is_subagent` early return); a stuck one (prompt, silence,
    escalation) gets its own wording (`stuck_subagent_text`): no project, no
    `respawn_agent` (refused for sub-agents) — stop it and start a fresh
    `spawn_subagent` carrying the answer; the user can answer in its terminal.
    `processes_with_subagents` feeds the dedupe so stuck reports don't restart
    from scratch.
  - **Visibility.** `ProcessInfo.subagent` (`SubagentMeta { kind, task, phase,
    endedAt? }`) is set by `process_info_in`/`process_info`. `process_list` /
    `list_processes` never return them (`list_subagents` /
    `assistant_subagents_list` do, oldest first, as `SubagentInfo`), but
    `process:added/updated/status/removed` events still fire for them with the
    internal project id, and `process_attach`/`process_write`/`get_process_*`
    work on their ids. `start_process`/`restart_process`/`remove_process`/
    `respawn_agent` are refused (Podium starts a queued one and removes ended
    ones itself). Every change publishes a full `subagentsChanged` snapshot
    (never recorded). The quit guard (`has_active_agents_or_terminals`) counts
    queued and running ones, and `shutdown()` stops them. `verify_sender`
    answers `{ fleet: true, kind: "subagent", session }` for one — no project,
    and no process id an agent could act on.

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

  That record then **escalates once, and stops**: an agent still blocked
  `escalate_after` later (default 10 minutes) is raised a second time, with
  wording that asks the assistant to say which answer it would pick and why —
  something the user can wave through in a word — and, once that answer is
  agreed, to call `respawn_agent` (nothing can type into an agent's
  terminal). That is the cap, `MAX_RAISES = 2` per question: raising it every
  `escalate_after` for as long as the agent lived once produced a "no change"
  reply every ten minutes for hours. The record is kept **for as long as the
  agent lives**, not only while it looks stuck: an agent that prints one line
  and falls silent again is waiting on the same unanswered question, and
  re-reporting it from scratch is exactly the repetition this prevents. A
  **new question cycle** starts when the user types into the agent's terminal
  after the last raise: `Orchestrator::write_stdin` stamps
  `ManagedProcess.last_input` (`last_user_input`), and a tick that finds it
  newer than `Waiting.last` drops the record, so the next stall is raised
  from scratch. A write counts as the user typing unless it is a short
  escape sequence (at most 16 bytes starting with ESC — xterm.js's own focus
  reports and cursor/device-attribute replies, which would otherwise end the
  question on every window focus change); a lone arrow key is skipped too.
  Otherwise only the agent exiting (or vanishing) clears the record.

  **Viewing** is what keeps the assistant quiet about the agent the user is
  looking at. The frontend's activity monitor reports it (`process_report_viewing`
  → `Orchestrator::set_viewing`, in memory only, at most one process, never a
  non-agent; the flag lives on `ManagedProcess.viewing`, so a removed process
  drops it by itself) and `ProcessInfo.viewing` carries it to
  `list_processes`/`get_process_status`. Proactivity skips the viewed agent:
  no finished/crashed nudge, and no idle/waiting nudge — its question is
  recorded as already raised instead (`Watcher::mark_seen`: one raise toward
  the cap, escalation clock restarted every tick it stays in view), so
  leaving the agent never fires a nudge at once. `add_response` for a
  `process_id` that is the viewed agent stores nothing and answers with a
  short "skipped, the user is viewing that agent" text (not an error;
  `Orchestrator::add_assistant_response` returns `Option`). A
  `report_to_assistant` from the viewed agent is still delivered, with one
  line appended telling the assistant not to mention it or file an item. The
  identity prompt says the same of `viewing: true`; reading the agent's
  output stays fine.

  Each raise becomes one `[Podium] …` turn plus a `notice` event, so the chat says why the assistant is speaking
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
  delivered as `PendingKind::Poll`: unlike a proposal-outcome `Notice` it
  leaves the same `Notice` row a nudge does (`record_before_write`, stored as a
  `nudge` row), so the panel folds the sweep and its actions into one faint
  "Podium check" line rather than mixing them with the user's own. The old
  concern, a row every 10 minutes piling up, is gone because the panel folds
  consecutive Podium rows into a single line. Unlike nudges, polls are not
  merged or deduped in the queue.

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

  **Desktop notifications come from the Responses list only.** A chat
  reply, a proposal card and a `[Podium]` nudge never ping; a **new
  Responses item** does (see _Terminal & activity_ for the frontend half).
  So what the assistant may file *is* the notification policy, and the
  identity prompt spells it out. `needsYou` only for an agent blocked on a
  question only the user can answer (one item per question, replaced when
  the question changes) and an external message (Slack/Teams/mail) that
  directly asks the user something — never a manual step the user already
  knows about (merging a PR, `just install-signed`, testing a build).
  `fyi` only for finished work the user asked for that has something to look
  at (a result, a PR/MR URL, a posted review, the answer), an agent that
  crashed or failed, an external heads-up that asks nothing, and an agent
  the assistant stopped or respawned on its own. Never: "no change"/"still
  waiting" reminders or a repeat of an open item, a routine check that found
  nothing, lookup or sub-agent results it used itself, an agent that
  finished with nothing left for the user, or anything about the viewed
  agent. A nudge that brings nothing new about an agent already reported
  gets no new chat reply either. Unit tests pin the key phrases.
  The old `ACTION_REQUIRED` marker and `TurnEnd.notify` (with the
  `turn_from_user` bookkeeping that fed it) are gone. `take_action_required`
  stays, only to strip a stray marker an assistant writes out of habit
  before anything is recorded or published; a reply that is *only* the
  marker is dropped like a swallowed one.

  **Proposals file their own item.** When the autonomy gate parks a proposal,
  `AssistantManager::propose` adds a `needsYou` Responses item — "Confirm or
  reject: `<tool> <summary>`", `source` `"proposal"`, no process/to-do — and
  keeps its id on the `PendingProposal`. Resolving (`resolve_proposal`),
  invalidating (`invalidate_proposals_for_project`) or dropping it
  (`new_conversation`) takes the item off again. Proposals are memory-only
  but the list is on disk, so `new_conversation` (every launch runs one)
  also sweeps every item whose source is `proposal`, which is how one a
  restart orphaned goes away. The assistant must not file one itself.

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
  **Turning the assistant off** (`enabled` in the same `assistant` block,
  default on; `Orchestrator::set_assistant_enabled`, IPC
  `assistant_enabled_get/set`): the session is killed deliberately (no
  `exited`), proposals and their Responses items are dropped, the queue is
  dropped with its undelivered messages reported via `queueNotice`, and all
  sub-agents are stopped quietly. From then on `assistant_send`,
  `assistant_day_start` and `assistant_response_reply` fail with
  `CoreError::AssistantDisabled` (`assistantDisabled`), nudges and the
  external sweep are dropped per call (`assistant_nudge` / `assistant_poll`
  and the manager's `deliver_own`), and `report_to_assistant` /
  `spawn_subagent` over MCP answer plain text ("the assistant is turned
  off; nothing was delivered"), so no session is ever spawned. Transcript,
  memory and the Responses list stay on disk. Turning it on spawns nothing
  until the next send or nudge. The manager mirrors the setting in an
  atomic, preset when the settings path is set, so startup honours it too.

  The watcher holds
  only a weak reference, so it ends with the orchestrator. **Settings →
  Assistant** holds this toggle plus the autonomy mode (the chat header keeps
  its own autonomy picker — both mirror the one stored value).

  A pending proposal is also dropped when **its project closes**
  (`invalidate_proposals_for_project`, keyed off the call's `project_id`
  argument): confirming it later could only fail, so the card resolves itself
  with the reason and the assistant is told.

### WSL-backend mode (Windows)

`backend.json` (`backend_settings.rs`) holds an optional WSL distro plus
the `nativeDriveProjects` opt-in (default off). Setup
reads it **first thing** (`init_backend` in `src-tauri/src/lib.rs`) and fixes
`platform::init_wsl` before the MCP server, the assistant or any project can
spawn; changing it needs a restart. `platform::wsl()` is the switch:
`Some(&WslBackend)` in WSL mode, always `None` off Windows (`init_wsl`
ignores a configured distro there).

**Per project, decided once.** By default every project runs on the
configured backend, a Windows-drive one through `/mnt/<drive>`. With the
`nativeDriveProjects` opt-in (`WslBackend::native_drive_projects`, Settings →
General → Backend), `platform::wsl_for(root)` returns the backend only for a
root inside a distro (`wsl_path::is_wsl_path` — `\\wsl.localhost\…`,
`\\wsl$\…`) and `None` for a Windows-drive project, which then runs natively
(cmd.exe, Windows git, Windows `claude`). The orchestrator resolves it **once,
at project open** (`ProjectHandle::wsl`) and passes that one value down —
`AgentLaunchCtx::wsl` for `build_launch` quoting and the `--mcp-config`
spelling, `PtyProcess::spawn` for the shell and the direct agent launch,
`wsl_launch_guard_for` for the gate, the `worktree::*` functions for git,
`AdapterRegistry::infos` for the availability probe — so a launch quoted for
one shell is never run by another. `ProjectInfo.nativeInWslMode` tells the
frontend, which then leaves the terminal-shell setting (meant for the
distro's shell) out of that project's terminals, and the default terminal
command follows the same flag. The explicit-backend helpers are the
`_in`/`_on(wsl, …)` variants (`login_shell_in`, `login_shell_on`,
`quote_arg_in`, `backend_path_in`, `from_backend_path_in`, `run_shell_ok_on`,
`command_exists_query_on`, `git_command`); the plain ones use the global
`wsl()` and are for workspace-wide work only — the assistant and the MCP
install. The launch gate is only taken for a `wsl.exe` start
(`run_gated` checks the program, `do_start` the project's backend), so a
native launch never waits behind a slow distro start.

- **Not behind `cfg(windows)`.** Call sites branch at runtime on `wsl()`, so
  macOS/Linux CI compiles, lints and tests the WSL code. Pure helpers take
  the mode as a parameter (`wsl_login_shell`, `worktree_path`,
  `WslBackend::for_tests`) so both modes are unit-testable anywhere; tests
  never call `init_wsl` (it is process-global).
- **Shell, quoting and probe.** `login_shell()` is `wsl.exe -d <distro> -e bash
  -lic`; `login_shell_in(wsl, cwd)` adds `--cd <linux path>`. `quote_arg` is POSIX
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
  under the orchestrator's `inner` lock**: `do_start` reads the project's
  backend under a short lock of its own, takes the gate, then locks `inner`
  and hands the gate to `PtyProcess::spawn` by reference, so it drops after
  `inner`.
  New code that starts `wsl.exe` must use one of these. A no-op natively.
- **No console flash.** A console-subsystem child (git, `wsl.exe`, `cmd.exe`,
  bash) spawned via a plain `Command`/`tokio::process::Command` from
  release-mode Podium (a GUI-subsystem app, no console of its own) gets its
  own new console window on Windows unless told otherwise — visible for the
  call's lifetime. `platform::hide_console_window` sets `CREATE_NO_WINDOW`;
  every non-ConPTY Windows spawn in this crate uses it (`run_shell_ok`,
  `run_shell_stdout`, `git_command`, the WSL networking check and distro
  listing, the assistant's CLI child — a tokio `Command`, via `as_std_mut()`
  — Warp's `cmd /C start`, and the Git board's `gh` calls via
  `gh_command`, several per project on every board refresh). ConPTY-attached spawns (`process::pty`) are already windowless
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
  cwd/env, a Windows-drive project running in the distro by default, argv
  round-trip, stop, config paths, the localhost reachability prerequisite,
  worktrees and interop. `tests/wsl_native_drives.rs` is its twin with the
  `nativeDriveProjects` opt-in on (a separate binary, since the backend is
  process-wide): a drive project's service runs under cmd.exe while a distro
  project's still runs in WSL, and an agent's `build_launch` quoting
  survives the real launch path on both (Windows direct launch, bash in the
  distro). Run it the same way with `--test wsl_native_drives`.

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
| `project_set_color`     | `{ projectId, color? }` (`#rrggbb`, blank clears) | `ProjectInfo` |
| `project_reorder`       | `{ ordered: string[] }` (project ids)       | `ProjectInfo[]`   |
| `project_set_sections`  | `{ projectId, sections: ProjectSections }`  | `ProjectInfo`     |
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
| `process_report_viewing` | `{ processId? }` (the agent open on screen; omitted = none; in memory only) | – |
| `process_activity`      | –                                           | `Record<ProcessId, OutputActivity>` (`{ quietMs, burstMs }` per running process that printed) |
| `process_rename`        | `{ processId, name }`                       | `ProcessInfo`     |
| `process_start`         | `{ processId }`                             | –                 |
| `process_stop`          | `{ processId }`                             | –                 |
| `process_restart`       | `{ processId }`                             | –                 |
| `process_write`         | `{ processId, dataB64 }`                    | –                 |
| `process_resize`        | `{ processId, cols, rows }`                 | –                 |
| `process_attach`        | `{ processId, channel: Channel<TermEvent> }`| –                 |
| `backend_settings_get`  | –                                           | `BackendSettingsDto` (`{ wslDistro, activeWslDistro, nativeDriveProjects, activeNativeDriveProjects }`, empty distro = native) |
| `backend_settings_set_wsl_distro` | `{ distro? }` (blank = native; must be installed — checked against `wsl.exe -l -q`; applies on restart) | `BackendSettingsDto` |
| `backend_settings_set_native_drive_projects` | `{ enabled }` (WSL mode: run Windows-drive projects natively; applies on restart) | `BackendSettingsDto` |
| `backend_wsl_networking_mode` | – (WSL mode only; the app warns when not `mirrored`) | `string \| null` |
| `adapters_list`         | `{ projectId? }` (probe on that project's backend) | `AdapterInfo[]`   |
| `agent_spawn`           | `{ projectId, adapterId?, name?, prompt?, todoIds?, scratchpadIds?, worktree?, worktreeName?, args? }` | `ProcessInfo` |
| `agent_settings_get`    | –                                           | `AgentSettingsDto` |
| `agent_settings_set_adapter` | `{ adapterId, command?, defaultArgs }` | `AgentSettingsDto` |
| `agent_settings_set_default_adapter` | `{ adapterId? }` (blank clears) | `AgentSettingsDto` |
| `agent_settings_set_merge_mode` | `{ mode }`                       | `AgentSettingsDto` |
| `agent_settings_set_suggest_worktree` | `{ enabled }`             | `AgentSettingsDto` |
| `agent_settings_set_remember_sessions` | `{ enabled }`            | `AgentSettingsDto` |
| `agent_settings_set_remove_exited_after` | `{ minutes }` (0 = never, max 1440) | `AgentSettingsDto` (`removeExitedAfterMinutes`) |
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
| `issues_board`          | `{ projectId, mine }`                       | `Board`           |
| `issues_board_cached`   | `{ projectId, mine }`                       | `CachedBoard \| null` (`{ board, fetchedAt }`, no network) |
| `issues_board_move`     | `{ projectId, kind: "issue"\|"pr", number, column }` | – (refusals: `issuesMove`; UI follows `board:changed`) |
| `issues_board_unassign` | `{ projectId, kind: "issue"\|"pr", number }` | – (removes the signed-in user from the assignees; refusals: `issuesMove`; UI follows `board:changed`) |
| `issues_settings_get`   | –                                           | `IssuesOverview` (`{ setups, projects }`, never a token) |
| `issues_setup_save`     | `{ id?, name, provider, url, account?, token? }` (token omitted = keep, blank = delete) | `IssuesOverview` |
| `issues_setup_remove`   | `{ id }`                                    | `IssuesOverview`  |
| `issues_project_setup_set` | `{ projectId, setupId? }` (omitted = match by host) | `IssuesOverview` |
| `worktree_list`         | `{ projectId }`                             | `WorktreeInfo[]`  |
| `worktree_remove`       | `{ projectId, name, force }`                | `WorktreeInfo[]` (refreshed list) |
| `worktree_open_in_warp` | `{ projectId, name }`                       | –                 |
| `assistant_send`        | `{ text, attachments }` (`Attachment[]`)    | `bool` — delivered, not queued (reply streams as `assistant:event`) |
| `assistant_day_start`   | –                                           | `bool` — as `assistant_send` |
| `assistant_transcript`  | –                                           | `TranscriptEntry[]` |
| `assistant_new_conversation` | `{ stopSubagents }` (false = sub-agents keep running into the new conversation) | – |
| `assistant_subagents_list` | –                                         | `SubagentInfo[]` (oldest first; changes stream as `subagentsChanged`) |
| `assistant_subagent_stop` | `{ id }` (process id; ungated user stop, queued = cancelled; the assistant is told) | –   |
| `assistant_memory_get`  | –                                           | `string`          |
| `assistant_memory_set`  | `{ content }` (blank clears)                | –                 |
| `assistant_autonomy_get` | –                                          | `AutonomyMode`    |
| `assistant_autonomy_set` | `{ mode }`                                 | `AutonomyMode`    |
| `assistant_enabled_get` | –                                           | `boolean`         |
| `assistant_enabled_set` | `{ enabled }` (off: session killed quietly, sub-agents stopped, input refused with `assistantDisabled`) | `boolean` |
| `assistant_proactivity_get` | –                                       | `boolean`         |
| `assistant_proactivity_set` | `{ enabled }`                           | `boolean`         |
| `assistant_polling_get` | –                                           | `AssistantPolling` |
| `assistant_polling_set` | `{ intervalMinutes, slack, teams }` (0 = off) | `AssistantPolling` |
| `assistant_resolve_proposal` | `{ id, approved }`                     | – (outcome streams as `assistant:event`) |
| `assistant_queue_edit`  | `{ id, text }`                              | –                 |
| `assistant_queue_send_now` | `{ id }` (waiting user message)          | –                 |
| `assistant_queue_remove` | `{ id }`                                   | –                 |
| `assistant_interrupt`   | –                                           | – (queue drain streams as `assistant:event`) |
| `assistant_responses_list` | –                                        | `ResponseItem[]` (newest first) |
| `assistant_response_dismiss` | `{ id }` (unknown id: `responseNotFound`) | –              |
| `assistant_response_reply` | `{ id, text }`                           | `bool` — as `assistant_send` |
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
`worktree:changed` / `board:changed` `{projectId}`, plus `assistant:event`
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
state ("working…"/"idle" is noise for a plain shell). The monitor also
covers the assistant's sub-agents, which are never in `processStore`:
`agentActivityStore` adds the running ones from `assistantStore.subagents`
(through `subagentProcess`) to its input, so one stuck on a prompt is reported
at once — **once its terminal has been opened** (the prompt is read off the
xterm viewport). A sub-agent whose terminal was never opened is caught by the
backend's 5-minute silence timer instead; only its folder-trust dialog the
backend spots on its own (see _Sub-agents_).

**Sub-agents (frontend).** `SubagentsSection` (rendered by `AssistantPane`
above the chat log, hidden when empty; collapsible) lists
`assistantStore.subagents` — loaded in `App.tsx` at startup (the activity
monitor and quit warning need it with the panel closed), kept current by
`subagentsChanged` snapshots plus `applySubagentStatus` from `process:status`
in between. The store also drops an ended row at its `expiresAt` by itself
(`pruneExpiredSubagents`, one timer re-armed on every list change), and
`App.tsx` re-pulls the list on window focus / `visibilitychange`, so the panel
never waits on a snapshot a hidden webview was slow to run. Per row name/task, kind, a live elapsed clock and phase (the
result or error is only the phase label's tooltip, never a line of the row); stop (`assistant_subagent_stop`, its click does not propagate) while
queued/running. The row's name/task area itself opens a started one
(`role="button"`, Enter/Space, a focus outline; a queued one is not
clickable) — there is no separate open button. A `waiting` row reads
"Needs input" (warning colour, the question as the phase tooltip), counts in
the header summary (`N needs input`) and stays live (stop, open, ticking
clock, `isSubagentLive`, `TerminalPane`'s stop, the quit warning's "needs
input" tag). Open =
`layoutStore.openSubagent` (`openSubagentInWorkArea`):
fills the work area with a `TerminalPane` on a synthesized process, exclusive
with the focused process, to-do, scratchpad and board (a sub-agent is never in
the process store, so it cannot use that focus); the pane shows an "Assistant
sub-agent · kind" label and a stop button, and its stop goes through the
sub-agent path. `CloseWarningModal` lists live sub-agents (queued marked)
beside processes. **Settings → Assistant**'s reset: with live sub-agents the
button shows an inline Keep running / Stop them / Cancel choice before calling
`assistant_new_conversation { stopSubagents }`; the choice is withdrawn when the
live count drops to 0 while it is open, and the next Reset just resets.

`notifyAgentWaiting` (`src/lib/notify.ts`) has a sibling, `notifyAssistant`,
for the same toast+OS-notification+sound treatment on the personal assistant.
It fires from one place only: `assistantStore`'s handling of a
`responsesChanged` snapshot (`notifyNewResponses`). A chat reply, a turn end,
a `proposal` card and a nudge never pings — the Responses list is the only
thing that reaches the desktop (a proposal gets there through the
`needsYou` item the backend files for it). A module-level `Set` holds the id
of every item this app run has seen, filled from every snapshot and from
`loadResponses` (also called once from `App` at launch, so a closed pane
still has a baseline; nothing pings before that first load completes). Only
an id not in the set is news, so what the list held at launch never pings
and a reply that gets restored (same id) never pings twice. The ping is
skipped when `isAssistantVisible()` (focused window **and**
`useLayoutStore.getState().assistantOpen`), and per item when its `processId`
is `viewedAgentId()`. Several new items in one snapshot make one
notification: title "Needs you" when any is `needsYou`, else "FYI"; body the
newest item's text plus "(+N more)".

**Viewing** (`agentActivityStore.ts`): `viewedAgentId()` is the one
definition — window focused, the active process is an agent, and no to-do,
scratchpad or Git board pane covers the work area (`isViewingAgent`, the
"needs input" gate, shares its base and so also treats an open scratchpad as
covering the terminal). The once-a-second monitor calls
`processReportViewing` (→ `process_report_viewing`) only when the value
changed, retries a failed report on the next tick, and also reports on window
`focus`/`blur`; stopping the monitor reports "none".

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
just version 0.2.0  # bump + sync version across all manifests (emergencies only; CI bumps on merge)
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
`main`. `main` stays releasable; every merged PR is released automatically
(see _Versioning & CI_). We do **not** open a GitHub issue per change:
the PR is the unit of work (title + body capture the what and why).
Conversations that touch no code (questions, exploration) need no branch.

1. **Branch** — from an up-to-date `main`, named `short-description`
   (e.g. `agent-activity-fix`). No issue-number prefix.
2. **Implement, commit, push** the branch. Commit subjects are free-form,
   imperative (Keep-a-Changelog voice). Per the global rule, **never add a
   `Co-Authored-By` trailer** (a `commit-msg` hook also strips Anthropic ones).
3. **PR** — `gh pr create --base main --fill` (or with an explicit title/body).
   The PR must add its entry under `## [Unreleased]` in `CHANGELOG.md` (CI
   fails without one). `ci.yml` runs on the PR and must be green.
4. **Merge = release** — merging a PR to `main` is the release; nobody
   bumps the version or tags by hand. Before merging, set a `minor` or `major`
   label if the change warrants more than a patch (`gh pr edit N --add-label
   minor`). Merge with `gh pr merge N --squash`; the branch auto-deletes. Get
   explicit user confirmation first, because merging publishes a release.

## Versioning & CI

- **SemVer.** The `[workspace.package]` version in the root `Cargo.toml` is
  the single source of truth; `scripts/sync-version.sh` propagates it to
  `tauri.conf.json` and `package.json` (`just version-check` detects drift).
- **`CHANGELOG.md`** follows _Keep a Changelog_ and lists **only functional
  (user-facing) changes**. Every PR adds its entry under `## [Unreleased]`;
  the PR-only `changelog` job in `ci.yml` fails unless the PR touches
  `CHANGELOG.md` and that section holds at least one `- ` entry
  (`scripts/changelog.sh unreleased`).
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
  packages the NSIS `.exe`. Runs on `workflow_dispatch` and pushes to `main`,
  uploading each OS's installer as an artifact; on a tag ref (the release
  job's dispatch) it also attaches the `.dmg`/`.exe` to that tag's GitHub
  Release. There is deliberately **no tag trigger** (see below).
- **Releasing is automatic: every merge to `main` is a release.** The
  `release` job in `ci.yml` runs after the test jobs on each push to `main`
  (its own non-cancelling `release` concurrency group, and the workflow never
  cancels runs on `main`, so no merge is left without a version). It checks
  out `main` itself, not the pushed sha, so a queued second run sees the
  first one's bump; an empty `## [Unreleased]` means already released, and
  it exits. Otherwise: the version is the latest `v*` tag bumped by
  `scripts/next-version.sh` — **patch by default**, `minor`/`major` when the
  merged PR carries that label (`major` wins). It renames `## [Unreleased]`
  to `## [X.Y.Z] - date` (`scripts/changelog.sh release`), runs
  `sync-version.sh --set` and `cargo update --workspace` (Cargo.lock), commits
  `Release vX.Y.Z [skip ci]` to `main` as `github-actions[bot]`, pushes it
  with the annotated tag `vX.Y.Z` (`--atomic`), creates the GitHub Release
  with the section as notes, and dispatches `build.yml` on the tag — the one
  installer build per release. **Never bump the version or create a tag by
  hand**; `just version` is for emergencies only.
- **Release push = deploy key.** `main`'s ruleset (PR required, linear
  history, no force-push/deletion) can only be bypassed by the admin role and
  `DeployKey`; GitHub refuses the Actions app as a bypass actor on a
  user-owned repo, so a `GITHUB_TOKEN` push is rejected with `GH013`. The
  release job therefore checks out with `ssh-key: secrets.RELEASE_DEPLOY_KEY`
  (the private half of a write-enabled deploy key; the owner creates both)
  and pushes over SSH. Its first step fails with an `::error::` naming the
  setup commands when the secret is missing. A deploy-key push, unlike a
  `GITHUB_TOKEN` one, *does* trigger workflows: `[skip ci]` keeps the release
  commit from re-running `ci.yml` (and `build.yml`'s `main` trigger), and
  since GitHub's docs don't say whether it also covers the tag in the same
  push, `build.yml` has no tag trigger at all — the explicit dispatch is the
  only build, so a release gets exactly one. Rotating the key = new deploy
  key + `gh secret set RELEASE_DEPLOY_KEY`, then delete the old key.
  The script tests (`scripts/*.test.sh`) run in the `changelog` job.
- **Not yet set up:** signed/notarized releases.

## Roadmap

All the originally planned phases are shipped, plus post-v1.0 work
(scratchpads, the app-quit close guard, the Auggie adapter, the retro theme).
This file and the code are the source of truth for current behaviour.
