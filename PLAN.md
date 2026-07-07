# Podium — Agent Orchestration Workspace

## Context

New greenfield app in `~/code/podium` (currently empty). Podium is an agent-orchestration workspace inspired by Solo (soloterm.com, closed source — we build from concept): open projects (folders), spawn AI coding agents and dev-server processes, watch and control them in embedded terminals. **Claude Code only in v1**, but agent support goes through an adapter abstraction so Codex/Gemini CLI/Aider fit later.

Decisions made with the user:
- **Stack:** Tauri 2 + React 19 + TypeScript (same as Selene and Solo itself), Rust backend.
- **MVP:** projects + agents + processes + embedded terminals **+ built-in MCP server** so agents inside Podium can control Podium (list processes, read output, spawn agents).
- **Agent model:** interactive Claude Code CLI in a PTY per agent.
- **UI:** reuse Selene's island theme and theme selector 1-op-1 (Dark/Light/Retro).
- Also: set up `CLAUDE.md`.

Reference project: `~/code/Selene` (same author, Tauri 2 + React + CSS Modules + Zustand — conventions and visual system are copied from here).

## Repo layout

```
podium/
├─ Cargo.toml                 # [workspace] members = ["src-tauri", "crates/*"]; workspace.package = version source of truth
├─ rust-toolchain.toml, justfile, scripts/sync-version.sh   # copy from Selene
├─ package.json, vite.config.ts, tsconfig.json, index.html, eslint/prettier configs  # mirror Selene (port 1420, strictPort)
├─ CLAUDE.md
├─ crates/podium-core/        # UI-agnostic core: ZERO tauri dependency, #![forbid(unsafe_code)]
│  └─ src/
│     ├─ lib.rs, error.rs (CoreError/thiserror), ids.rs (ProjectId/ProcessId uuid newtypes)
│     ├─ config.rs            # podium.yml serde types
│     ├─ project.rs           # load/validate podium.yml
│     ├─ events.rs            # PodiumEvent + tokio broadcast bus
│     ├─ orchestrator.rs      # single public API surface (Tauri commands AND MCP tools both call this)
│     ├─ process/{mod,pty,scrollback,supervisor}.rs
│     ├─ agent/{mod,claude}.rs
│     └─ mcp/{mod,tools}.rs
└─ src-tauri/                 # thin IPC adapter, no domain logic (lib name podium_lib)
   └─ src/{main,lib,state,error,events}.rs + commands/{project,process,agent,mcp}.rs
```

Key crates: `portable-pty 0.9` (PTY), `serde_yaml_ng` (yml), `rmcp` pinned (official MCP Rust SDK, features `server,macros,transport-streamable-http-server`), `axum`, `shlex`, `strip-ansi-escapes`, `nix` (unix signal), `tokio`, `thiserror`, `uuid`, `chrono`.

Tauri plugins: `dialog` (folder picker), `log`, `window-state`.

## Visual system — port from Selene

Copy **nearly verbatim** (rename `selene.` storage keys → `podium.`):
- `src/styles/tokens.css` — all design tokens, 3 themes on `:root[data-theme]`. **Extend each theme block with `--term-*` xterm palette** (bg/fg/cursor/selection + 16 ANSI colors as literal hex — xterm can't parse `color-mix()`). Dark→GitHub-dark ANSI, Light→GitHub-light, Retro→Gruvbox-light.
- `src/styles/global.css`, `src/lib/motion.ts` (MOTION, usePresence), `src/lib/platform.ts`
- `src/state/themeStore.ts` — View Transitions crossfade, localStorage, `data-theme` attribute
- `src/components/Modal.tsx`, `WindowControls.tsx`, `Toasts.tsx` + `toastStore.ts` (+ hun .module.css)
- `SettingsModal.module.css` + het **THEMES array + swatch-selector blok** uit `SettingsModal.tsx` (Dark/Light/Retro swatchknoppen met checkmark)
- Island-recept uit `Sidebar.module.css` / `App.module.css`: `.sidebar` island (radius-xl, border, shadow-md, `islandIn` animatie), section-collapse (grid-rows trick), status-dots (`.dotOn` breathe / `.dotConnecting` pulse), titlebar (40px), body = `--canvas` met `--gap-island` padding, sidebar-resizer (drag handler uit `App.tsx` regels ~85–103 hergebruiken)

Adapt: `App.tsx` (brand "Podium", body = Sidebar + resizer + work area of WelcomeScreen), `SettingsModal.tsx` (één General-panel: theme + reduce motion + terminal fontsize 11–18px), `icons.tsx` (icon()-factory houden; Bot, SquareTerminal, Play, Square, RotateCw, Plus, Folder, Settings, Check, …), `layoutStore.ts` (sidebarWidth clamp + collapsed sections), `settingsStore.ts` (deepMerge-skelet, nieuwe shape `{appearance:{reduceMotion}, terminal:{fontSize:13}}`).

New frontend files:
- `src/lib/terminalRegistry.ts` — **xterm instances live OUTSIDE React**, keyed by processId, reparentable host div (solves StrictMode double-mount; scrollback survives switching). Re-theme all live terminals on themeStore change.
- `src/lib/terminalTheme.ts` — `readTerminalTheme(): ITheme` via getComputedStyle over `--term-*` vars
- `src/components/TerminalView.tsx` (attach/detach + ResizeObserver → fit → `process_resize`), `TerminalPane.tsx` (island + header: naam, status dot, start/stop/restart), `Sidebar.tsx` (ProjectSwitcher bovenaan; secties Agents / Processes / Terminals met `+`-knoppen), `ProcessRow.tsx`, `WelcomeScreen.tsx` (open folder + recents), `NewAgentModal.tsx`
- `src/ipc/{types,commands,events}.ts` — typed invoke-wrappers (Selene-patroon), `src/state/{projectStore,processStore}.ts`

npm deps: react 19, zustand 5, `@xterm/xterm` + `@xterm/addon-fit`, lucide-react, @tauri-apps/api + plugin-dialog + plugin-log. Geen TanStack Query (state is push-based). Dev-deps mirror Selene (vite 7, vitest, eslint 9, prettier).

## Backend architecture

**PTY (`process/pty.rs`):** spawn via `$SHELL -lc "<command>"` (login shell → lost macOS GUI-PATH probleem op; `claude`/`pnpm`/nvm resolven). `TERM=xterm-256color`. Één blocking read-thread per proces → `ScrollbackBuffer` (ring, 2 MiB cap, raw bytes incl. ANSI, monotone `seq`) → `tokio::broadcast`. Wait-thread op `child.wait()` → supervisor. **Stop = `killpg` op de sessie-group** (SIGTERM → 5s grace → SIGKILL) — anders overleven grandchildren (dev servers). App-exit hook (`RunEvent::Exit`) killt alles.

**Process model:** `ProcessKind { Service, Terminal, Agent { adapter } }` — een agent ÍS een proces. `ProcessStatus { NotStarted, Running{pid,since}, Stopping, Exited{code,crashed,at} }`. `crashed` = non-zero exit én niet door de gebruiker gestopt (`user_stopped` flag). `RestartPolicy { Never, OnCrash, Always }` met exponential backoff (500ms→30s) + circuit breaker (5 restarts/60s). `Orchestrator` = enige publieke API (open/close project, list/start/stop/restart, spawn_agent, attach, write_stdin, resize, tail_text, subscribe, shutdown).

**`podium.yml`** (in project-root, optioneel — folder zonder yml is gewoon een project):
```yaml
name: Webshop            # default: mapnaam
icon_initials: WP        # default: afgeleid, max 2 chars
processes:
  - name: web
    command: pnpm dev
    cwd: apps/web        # relatief aan root, traversal-guard
    auto_start: true     # default false
    auto_restart: on-crash   # never|on-crash|always
    env: { PORT: "3000" }
agents:
  default_adapter: claude-code
  extra_args: []
```
`deny_unknown_fields`; parse-fout → project opent met 0 processen + fout in UI. Reload via expliciet command (geen file-watch in v1).

**Agent adapter (`agent/mod.rs`):** trait `AgentAdapter { id, display_name, binary, build_launch(ctx) -> LaunchPlan, is_available, state_hint }`. Alleen `ClaudeCodeAdapter` geregistreerd. `build_launch`: schrijft `{app_data}/mcp/agent-{id}.json` met `{"mcpServers":{"podium":{"type":"http","url":"http://127.0.0.1:<port>/mcp","headers":{"Authorization":"Bearer <token>"}}}}`, bouwt `claude [prompt] --mcp-config <path> [extra_args]` (shell-quoted via `shlex`), env `PODIUM_PROCESS_ID`/`PODIUM_PROJECT_ID`. Idle/working-detectie v1 = alleen `last_output_at` heuristiek.

**MCP server (`mcp/`):** `rmcp` StreamableHTTP op `127.0.0.1:0` (ephemeral port) achter axum, **bearer token per app-run** (anders kan elk lokaal proces agents spawnen). 8 tools, allemaal dunne calls naar Orchestrator: `list_projects`, `list_processes`, `get_process_status`, `get_process_output` (ANSI-stripped, default 100 regels), `spawn_agent`, `start_process`, `stop_process`, `restart_process`. Recursion-guard: max 8 concurrent agents per project. Token nooit in logs of `mcp_status` response; `mcp/`-dir gewiped bij startup.

## IPC contract (gereconcilieerd backend↔frontend)

Alle commands `Result<T, IpcError{message,kind}>`. **Terminal-data via per-attach `tauri::ipc::Channel`** (niet global events — per-webview, hoog volume, cleanup on drop):

- `process_attach { processId, onData: Channel<TermEvent> } → { snapshotB64, nextSeq }` — snapshot + subscribe atomair onder één lock (gap-vrij). Frontend schrijft snapshot, dropt events met `seq < nextSeq`. `TermEvent = {kind:"data",seq,dataB64} | {kind:"lagged"} | {kind:"eof"}`; bij `lagged` → re-attach (self-healing). Backend batcht ~16ms / max 64KiB per event; base64 (multibyte-safe).
- Overige commands: `project_open/close`, `projects_list`, `project_config_reload`, `recents_list/remove` (recents.json in app-config-dir, atomic writes, cap 20), `processes_list`, `process_start/stop/restart`, `process_write {processId,dataB64}`, `process_resize {processId,cols,rows}`, `agent_spawn {projectId,prompt?,name?,adapterId?}`, `adapters_list`, `mcp_status` (url, geen token).
- Global Tauri events (laag volume): `process:status`, `process:added/removed`, `project:opened/closed`.

## CLAUDE.md (nieuw, gemodelleerd naar Selene's)

Secties: wat Podium is + status; tech stack tabel; architectuur (workspace, podium-core UI-agnostisch, src-tauri dun); adapter-patroon (Claude nu, Codex/Gemini later); volledige IPC-contract-tabel + TermEvent/seq-protocol; veldcasing-nuance (envelope camelCase, domain types snake_case); build/run (`just dev|build|check|test|lint|format`); conventies: `#![forbid(unsafe_code)]`, `clippy -D warnings`, motion-tokens verplicht (geen hardcoded durations, alleen transform/opacity, prefers-reduced-motion), security (MCP-token nooit in logs/IPC, `killpg` bij stop, geen secrets in podium.yml aanraden), settings-discoverability (elke user-facing feature in SettingsModal); versioning (workspace.package + sync-version.sh); roadmap (Windows/Linux, meer adapters, todos/scratchpads/timers/locks à la Solo, idle-detectie, CPU/mem stats).

## Build-fasen (elke fase compileert, lint, demo'baar)

0. **Skeleton** — workspace scaffold, lege podium-core, Tauri-venster boot, justfile/toolchain/sync-version. Demo: `just dev` opent venster.
1. **Visual shell** (geen backend) — tokens/global/motion/themeStore/Modal/WindowControls/Toasts gekopieerd; App met titlebar, lege sidebar-island, resizer, lege work-island; SettingsModal met theme-selector. Demo: 3 themes wisselen met crossfade.
2. **Core PTY engine** — ids/error/scrollback/pty + minimale Orchestrator; unit tests tegen echte PTY's (`sh -c`, exit codes, seq, killpg). Demo: `cargo test -p podium-core`.
3. **Terminal in de app** — IpcError/AppState, project_open (folder, nog geen yml), attach/write/resize, terminalRegistry + TerminalView + `--term-*` tokens. Demo: `htop` in embedded terminal; remount overleeft via snapshot.
4. **Config & supervision** — podium.yml, auto_start, supervisor (auto-restart/backoff/breaker), status-events, recents, shutdown-hook. Demo: crashend proces restart zichtbaar; quit laat geen orphans achter (`pgrep`).
5. **Claude-agent (zonder MCP)** — adapter-trait, ClaudeCodeAdapter, agent_spawn, NewAgentModal. Demo: interactieve Claude-sessie in Podium.
6. **MCP-loop** — rmcp-server + bearer auth + 8 tools + adapter-injectie. Demo (headline): een door Podium gespawnde Claude draait `list_processes`, leest dev-server-output en spawnt een sibling-agent.
7. **Polish** — activity-indicator, output-batching tuning, agent-cap, window-state, CLAUDE.md afronden, CI.

## Verificatie

- Per fase: `just lint && just test` (clippy -D warnings, vitest, cargo test) + de demo hierboven.
- Fase 3: terminal-remount (proces wisselen en terug) → scrollback intact, geen dubbele output.
- Fase 4: `podium.yml` met `auto_restart: on-crash` + `sh -c 'sleep 1; exit 1'` → zichtbare restarts, breaker stopt na 5; app afsluiten → `pgrep -f 'sleep'` leeg.
- Fase 6: in Podium gespawnde Claude vragen "list all processes in this project and tail the web server output" → moet Podium-MCP-tools gebruiken; extern proces zonder token → 401.

## Risico's

- **rmcp 0.x API-churn** — versie pinnen, voorbeelden van de pinned tag volgen, alle rmcp-gebruik in `mcp/` isoleren.
- **xterm flooding** — batching + lagged→re-attach; echte flow control uitgesteld.
- **macOS PATH** — structureel opgelost via `$SHELL -lc`; `adapters_list.available`-probe toont ontbrekende `claude` vroeg.
- **Orphan processes** — killpg + exit-hook vanaf fase 4, niet later.
- **Claude CLI `--mcp-config` met `"type":"http"` + headers** — werkt in huidige CLI's; bij implementatie even verifiëren tegen geïnstalleerde versie.
