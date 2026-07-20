# Podium

A cross-platform **desktop cockpit for AI coding agents**, built in Rust +
Tauri. Open a project folder and Podium runs its dev servers, ad-hoc
terminals, and AI coding agents (Claude Code today) — each in its own PTY with
a real terminal view and lifecycle supervision. A built-in MCP server lets the
agents inside Podium inspect and orchestrate the very processes Podium manages,
including spawning more agents.

<!-- TODO: add a screenshot or GIF of the app here -->

## Requirements

- **macOS or Windows** (Linux runs but isn't packaged yet)
- **Rust** toolchain (stable; see `rust-toolchain.toml`)
- **Node.js** + **pnpm**
- An agent CLI on your `PATH` if you want to spawn agents — e.g. `claude`
  ([Claude Code](https://docs.claude.com/en/docs/claude-code))

## Build & run

Tasks run through [`just`](https://github.com/casey/just) (run `just` to list
recipes); the Tauri CLI is the local dev-dependency, so no global install.

```sh
just dev      # run the app (Vite + Tauri, hot reload)
just build    # production desktop bundle (.app + .dmg on macOS)
just test     # Rust workspace tests + frontend Vitest
just lint     # clippy -D warnings + eslint
```

## Configuring a project

Drop an optional `podium.yml` at a project root to declare dev-server
processes and agent defaults:

```yaml
name: Webshop
processes:
  - name: dev-server
    command: pnpm dev
    cwd: web
    auto_start: true
    auto_restart: on-crash   # never | on-crash | always
agents:
  default_adapter: claude-code
```

A folder without a `podium.yml` still opens fine.

## Pointing an external agent at Podium

Podium runs a bearer-authenticated MCP server and ships a stdio bridge
(`mcp-bridge`, a subcommand of the app binary) so external MCP clients keep
working across restarts even though the port and token rotate each launch.
Register it once — the Settings → MCP tab has one-click install cards, or by
hand for Claude Code:

```sh
claude mcp add --scope user --transport stdio podium -- /path/to/podium mcp-bridge
```

Agents Podium spawns itself get the URL + token automatically.

## Architecture

`podium-core` (zero Tauri dependency) holds the orchestration/PTY/MCP domain;
`src-tauri` is a thin IPC adapter; `src/` is the React frontend. See
[`CLAUDE.md`](./CLAUDE.md) for the full architecture, IPC contract, and
conventions.
