# Overview

A Cargo **workspace**. `podium-core` holds all orchestration/PTY/MCP logic and
has **zero Tauri dependency** (plain-`cargo test`-able). `src-tauri` is a thin
IPC adapter. `src/` is the React app (presentation only).

## Layout

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

## Lineage

Podium is architecturally patterned after an earlier desktop app by the same
author: the same Cargo workspace shape (a `*-core` crate with zero Tauri
dependency, a thin `src-tauri` IPC layer, a presentation-only React frontend),
the same `justfile`/version-sync conventions and a mirrored `CLAUDE.md`
structure. That sibling app is a private repo; this is background only, nothing
depends on access to it.
