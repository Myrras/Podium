# CLAUDE.md — Podium

Guidance for AI agents (and humans) working in this repository.

## What Podium is

Podium is a cross-platform **desktop cockpit for AI coding agents**, built in
**Rust + Tauri**. You open a project folder and Podium runs its dev servers
(from `podium.yml`), ad-hoc terminals, and AI coding agents (Claude Code
today; more adapters later) — each in its own PTY with a real terminal view,
lifecycle supervision, and a built-in **MCP server** so agents can inspect
and orchestrate the very processes Podium manages (including spawning more
agents). macOS-first; Windows/Linux kept in mind (the PTY layer is Unix-only
today).

> Naming: product and repo are **Podium**. Core crate `podium-core`; bundle id
> `com.podium.app`.

> Lineage: architecturally patterned after the user's earlier app **Selene**
> (a desktop SQL editor, same stack) — same Cargo workspace shape (a `*-core`
> crate with zero Tauri dependency, a thin `src-tauri` IPC layer, a
> presentation-only React frontend), the same `justfile`/version-sync
> conventions, and this file mirrors Selene's `CLAUDE.md` structure. See
> `../Selene/CLAUDE.md` for the sibling project's conventions and prior art.

## Boilerplate (shared skeleton)

Podium's shared skeleton — the Cargo workspace shape (a `*-core` crate with
zero Tauri dependency + a thin `src-tauri` IPC layer + a presentation-only
React frontend), the `justfile`, `scripts/sync-version.sh`, CI, the shared
config files, the design tokens, and the layering/IPC conventions — is
extracted into a standalone template:

- **Repo:** <https://github.com/RobbieMinderhoud/rust-application-boilerplate>
  (`../rust-application-boilerplate` locally). It is the **upstream** source of
  truth for the skeleton; Podium carries its domain (processes, PTYs, agents,
  MCP) on top.
- **Rule:** when a change here touches the **shared skeleton or conventions**
  (not Podium-specific domain code), also open a **PR against the boilerplate**
  so it stays current. App-specific code does not flow back.

## Status — v1.0

- ✅ **`podium-core`**: the whole domain — project open/close + `podium.yml`
  config (strict serde, broken config still opens the project), PTY process
  engine (spawn via `$SHELL -lc`, process groups, `killpg` SIGTERM→SIGKILL
  stop), ring-buffer scrollback with per-chunk `seq`, supervision (exponential
  backoff + circuit breaker), agent adapters (Claude Code, Auggie), per-project
  to-dos (persisted, shared with agents), and the built-in MCP server (axum +
  rmcp streamable-HTTP, bearer auth, 15 tools). Zero Tauri dependency; unit
  tests plus real-PTY/MCP integration tests on plain `cargo test`.
- ✅ **Tauri IPC layer** (`src-tauri`): **34 commands**, per-attach terminal
  `Channel` streaming (16ms/64KiB batching), a global-event forwarder for
  lifecycle events, persistent recents + workspace list, window-state
  persistence, dialog + log plugins, locked-down CSP.
- ✅ **Frontend** (`src/`): project switcher + sidebar (Agents / Processes /
  Terminals / To-dos), xterm.js terminals living **outside React** (registry
  keyed by processId; scrollback survives tab switches and StrictMode),
  snapshot+seq attach protocol with lag re-attach, new-agent modal, agent
  activity ("working…"/idle) heuristic, a to-do detail view that fills the
  work area (description edit + comment thread, mutually exclusive with the
  focused process), settings + theme (dark/light) + toasts, Zustand stores,
  typed IPC wrappers over all 28 commands.
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
│  ├─ mcp/                     # built-in MCP server (mod.rs) + the 15 tools (tools.rs) + stdio bridge (bridge.rs)
│  ├─ todo.rs                  # per-project to-dos (TodoInfo + persistent TodoStore)
│  ├─ events.rs                # PodiumEvent + broadcast EventBus
│  ├─ ids.rs                   # ProjectId / ProcessId / TodoId (UUID newtypes)
│  └─ error.rs                 # CoreError (messages are Podium-owned text, never output/secrets)
├─ src-tauri/src/
│  ├─ lib.rs                   # Tauri builder, plugins, MCP startup, command registration, shutdown
│  ├─ state.rs                 # AppState: Arc<Orchestrator> + McpServer handle
│  ├─ events.rs                # PodiumEvent → global Tauri events ("process:status", …)
│  ├─ error.rs                 # IpcError { kind, message } ← CoreError
│  └─ commands/{project,recents,workspace,process,agent,mcp,todo}.rs
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
it; at startup the frontend calls `workspace_list` and re-opens every path
via `project_open` (pruning dead paths with `workspace_remove`). Each entry is
a `{ path, name? }` object — the optional `name` is a user-set display-name
override (set via `project_rename`) and list position is the sidebar order
(set via `project_reorder`); legacy bare-path-string files are migrated on
load. `project_open` re-applies a stored name override so a renamed project
comes back named after a restart.

- **Processes** are shell command lines run via `$SHELL -lc` in their own
  PTY + process group. Stop = SIGTERM to the group, SIGKILL after a grace
  period. A user stop is never counted as a crash.
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
  `spawn_agent`, `start_process`, `stop_process`, `restart_process`,
  `list_todos`, `add_todo`, `complete_todo`, `update_todo`, `comment_todo`,
  `add_todo_link` (pin an issue/PR URL to the top of a to-do),
  `assign_todo` (a running agent self-assigns a to-do via its
  `PODIUM_PROCESS_ID`, so the user sees who owns it).
  Spawned agents get the URL + token via 0600 per-agent config files under
  `{app_data_dir}/mcp` (wiped on every start); the same dir holds
  `server.json` (current URL + token, 0600) for the **stdio bridge**:
  external MCP clients configure `Podium mcp-bridge` (a subcommand of the
  app binary, `mcp/bridge.rs`) once — the bridge proxies stdio ↔ HTTP,
  re-reads `server.json`, and transparently replays the `initialize`
  handshake with backoff when the app restarts, so client config never goes
  stale even though port and token rotate every launch
  (`PODIUM_APP_DATA_DIR` overrides the app data dir). `mcp/install.rs`
  registers the bridge with external clients (Claude Code today:
  `claude mcp add --scope user --transport stdio podium -- <exe>
  mcp-bridge`, via the login shell) — surfaced in the Settings → MCP tab
  as a one-click Run/Copy card with an installed indicator.
- **To-dos** (`todo.rs`): each project has a shared to-do list, visible to
  the user (sidebar) and every agent (MCP). Persisted in one `todos.json`
  in the app data dir — **keyed by project root path**, not project id, so
  lists survive restarts; atomic writes (temp file + rename) keep the
  project folder clean and the file uncorrupted. Every mutation emits
  `TodosChanged`. To-dos can carry pinned issue/PR **links** (agents add
  them via `add_todo_link`, shown at the top of the detail pane) and can be
  **archived**: listing auto-archives any done to-do left over from an
  earlier day, and a to-do can be archived/unarchived manually — archived
  items drop out of the active list and show in the Archive modal.

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
| `recents_list`          | –                                           | `RecentProject[]` |
| `recents_remove`        | `{ path }`                                  | `RecentProject[]` |
| `workspace_list`        | –                                           | `string[]`        |
| `workspace_remove`      | `{ path }`                                  | `string[]`        |
| `process_add`           | `{ projectId, spec: NewProcess }`           | `ProcessInfo`     |
| `process_remove`        | `{ processId }`                             | –                 |
| `process_list`          | `{ projectId? }`                            | `ProcessInfo[]`   |
| `process_rename`        | `{ processId, name }`                       | `ProcessInfo`     |
| `process_start`         | `{ processId }`                             | –                 |
| `process_stop`          | `{ processId }`                             | –                 |
| `process_restart`       | `{ processId }`                             | –                 |
| `process_write`         | `{ processId, dataB64 }`                    | –                 |
| `process_resize`        | `{ processId, cols, rows }`                 | –                 |
| `process_attach`        | `{ processId, channel: Channel<TermEvent> }`| –                 |
| `adapters_list`         | –                                           | `AdapterInfo[]`   |
| `agent_spawn`           | `{ projectId, adapterId?, name?, prompt? }` | `ProcessInfo`     |
| `agent_settings_get`    | –                                           | `AgentSettingsDto` |
| `agent_settings_set_adapter` | `{ adapterId, command?, defaultArgs }` | `AgentSettingsDto` |
| `agent_settings_set_default_adapter` | `{ adapterId? }` (blank clears) | `AgentSettingsDto` |
| `agent_settings_set_merge_mode` | `{ mode }`                       | `AgentSettingsDto` |
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
`project:closed` / `todo:changed` `{projectId}`.

> ⚠️ **Field casing for the frontend:** everything on the wire is
> **camelCase** — DTO fields (`projectId`, `dataB64`, `configError`, …) and
> the serde tags: `ProcessKind` is tagged by `kind`
> (`service`/`terminal`/`agent {adapter}`), `ProcessStatus` by `state`
> (`notStarted`/`running {pid, since}`/`stopping`/`exited {code, crashed, …}`),
> `TermEvent` by `type` (`snapshot`/`data`/`lagged`). The one exception:
> `RestartPolicy` values are **kebab-case** (`never`/`on-crash`/`always`) —
> matching `podium.yml`'s `auto_restart`. Match these exactly in the
> hand-written types in `src/ipc/types.ts`.

### `podium.yml`

Optional, at the project root, **strict** (`deny_unknown_fields` — typos are
readable errors; a broken config still opens the project and surfaces via
`ProjectInfo.configError`). Snake_case keys:

```yaml
name: Webshop            # display name (default: folder name)
icon_initials: WS        # sidebar badge, max 2 chars
processes:
  - name: dev-server
    command: pnpm dev    # run via $SHELL -lc
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
extra IPC.

## Build, run, test

Prefer `just` (run `just` to list recipes). The Tauri CLI is the local
`@tauri-apps/cli` dev-dependency, so use `pnpm tauri …` (no global install).

```
just dev            # run the app (Vite + Tauri, hot reload)
just build          # production bundle
just check          # cargo check --workspace
just test           # cargo test --workspace + frontend vitest
just test-core      # podium-core tests only
just lint           # clippy -D warnings + eslint
just format         # rustfmt + prettier
just version 0.2.0  # bump + sync version across all manifests
just version-check  # verify versions are in sync
```

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
- **Bundle build** (`.github/workflows/build.yml`): builds the macOS bundle
  (the `.app` plus the APFS `.dmg` via `scripts/make-dmg.sh`) on
  `workflow_dispatch` and on any pushed `v*` tag, uploading the `.dmg` as an
  artifact. macOS only — the PTY layer is Unix-only and the app is macOS-first.
- **Releasing** is done via the **`release` skill** (see _Git workflow_).
  Do **not** create git tags or releases without explicit user approval
  (invoking the skill + its confirmation step is that approval).
- **Not yet set up:** Conventional Commits, git-cliff changelog automation,
  signed/notarized releases.

## Roadmap

Phases 0–7 of `PLAN.md` are shipped (v1.0). `PLAN.md` is the historical
build plan — consult it for design rationale, but this file and the code are
the source of truth for current behaviour.
