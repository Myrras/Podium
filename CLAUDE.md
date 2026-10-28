# CLAUDE.md — Podium

Guidance for AI agents (and humans) working in this repository.

## What Podium is

Cross-platform **desktop cockpit for AI coding agents**, built in **Rust + Tauri**. Open a project folder and Podium runs its dev servers (`podium.yml`), ad-hoc terminals and AI coding agents (Claude Code; more adapters later), each in its own PTY with lifecycle supervision. A built-in **MCP server** lets agents inspect and orchestrate those processes, including spawning more agents. macOS and Windows are built and packaged (ConPTY on Windows); Windows has an optional [WSL-backend mode](docs/architecture/wsl-backend.md). Linux is kept in mind, not packaged.

> Naming: product and repo are **Podium**. Core crate `podium-core`; bundle id `com.podium.app`.

## Boilerplate (shared skeleton)

The shared skeleton (workspace shape, `justfile`, `scripts/sync-version.sh`, CI, config files, design tokens, layering/IPC conventions) is a standalone template: <https://github.com/RobbieMinderhoud/rust-application-boilerplate>. It is the **upstream** of the skeleton; Podium adds its domain on top.

- **Maintainer rule (internal):** a change to the shared skeleton or conventions (not Podium domain code) also gets a **PR against the boilerplate**. App-specific code does not flow back. Outside contributors can ignore this.

## Status

Shipped and current (v1.1.x). The counts are the source of truth: keep them in sync when you add a command or MCP tool.

- ✅ **`podium-core`**: the whole domain, with the built-in MCP server (axum + rmcp streamable-HTTP, bearer auth, 38 tools). Zero Tauri dependency; unit and real-PTY/MCP integration tests on plain `cargo test`.
- ✅ **Tauri IPC layer** (`src-tauri`): **101 commands**, per-attach terminal `Channel` streaming, global-event forwarder, locked-down CSP.
- ✅ **Frontend** (`src/`): sidebar, work-area views, xterm.js terminals outside React, assistant chat panel, Zustand stores, typed IPC wrappers over all 101 commands.

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

Notes live in `docs/architecture/`, not here. Read the matching file before changing an area. Changed architecture? Update that file (add one if none fits) and its line below.

- `docs/architecture/overview.md` — crate layout and file tree; read first
- `docs/architecture/core-contract-and-workspace.md` — Orchestrator entry point, sidebar sections, workspace.json
- `docs/architecture/processes-and-agents.md` — PTY processes, supervision, agent adapters, session memory
- `docs/architecture/mcp-server.md` — built-in MCP server, auth, tools; read when adding a tool
- `docs/architecture/cross-session-messaging.md` — messages between assistant/agent sessions
- `docs/architecture/todos.md` — per-project to-dos
- `docs/architecture/scratchpads-and-worktrees.md` — shared notes, managed worktrees
- `docs/architecture/git-board.md` — issues and PRs/MRs board, cache
- `docs/architecture/assistant.md` — assistant chat, autonomy, persistence, queue, Responses
- `docs/architecture/assistant-subagents.md` — sub-agent kinds, launch, cap, nudges, questions
- `docs/architecture/assistant-memory-and-proactivity.md` — assistant memory, proactivity, notifications, proposals
- `docs/architecture/wsl-backend.md` — Windows WSL-backend mode, path translation
- `docs/architecture/tauri-ipc-contract.md` — Tauri commands, events, wire casing; read when adding a command
- `docs/architecture/podium-yml.md` — `podium.yml` format
- `docs/architecture/terminal-and-activity.md` — frontend terminal, agent activity, window chrome

## Build, run, test

Prefer `just` (run `just` to list recipes). The Tauri CLI is the local dev-dependency: use `pnpm tauri …`, no global install.

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

Every recipe exports `CARGO_TARGET_DIR` at the **main checkout's** `target/`, so worktrees share one build cache. Cargo locks it: builds in different worktrees serialise. Release builds use cargo's default codegen (fat LTO + `codegen-units = 1` cost ~110s of link time for ~3.5 MB).

`just dev` ad-hoc-signs with a fresh identity on every rebuild, so the macOS keychain re-prompts each run. The `*-signed` recipes pass `src-tauri/tauri.dev-signed.conf.json` (sets only `bundle.macOS.signingIdentity`): one stable identity, so "Always Allow" sticks. They build a bundle, so **no hot reload**. Use `just dev` to iterate, `just dev-signed` when the keychain prompt is in the way, `just install-signed` to install into `/Applications`. Only `just build` produces the `.dmg` (`scripts/make-dmg.sh`, the CI release path); the signed recipes skip it (`--bundles app`).

Core integration tests (`crates/podium-core/tests/`) use **real PTYs** and a real MCP server on ephemeral ports, in plain `cargo test` (no Docker), on macOS and Windows. Tests that need Unix-only commands/shell syntax or process-group semantics are `#[cfg(unix)]` or `#[cfg_attr(windows, ignore = "…")]`, with a comment why. `tests/wsl_backend.rs` is the opt-in Windows + WSL suite (see [WSL-backend mode](docs/architecture/wsl-backend.md)). Frontend tests: Vitest + jsdom + React Testing Library (`src/test/setup.ts`); Tauri APIs are mocked per file.

## Conventions

- **Rust:** `#![forbid(unsafe_code)]` in every crate; `cargo clippy -- -D warnings` must pass; `cargo fmt`. Comment the _why_ of non-obvious logic.
- **Layering (always-on):** all orchestration logic lives in `podium-core`; `src-tauri` only (de)serializes and forwards. A command handler that grows logic moves down. The core must compile without Tauri.
- **Security (always-on):**
  - The MCP **bearer token** is never logged, never crosses the Tauri IPC bridge (`mcp_status` is token-free), and reaches agents and the stdio bridge only via 0600 config files wiped on every start. `McpConnectInfo` has a manual `Debug` impl so `{:?}` cannot leak it; the bridge never writes it to its client-facing stdio stream.
  - **Never log** terminal output or process command lines above `DEBUG`/`TRACE`. `IpcError`/`CoreError` messages are Podium-owned text.
  - `cwd` for new processes is validated against the project root (no absolute paths, no `..` escape).
  - Stops kill the whole **process group** (`killpg`) so children cannot outlive their process.
  - Tests/fixtures use **only fictitious** sample data.
- **Motion:** timing comes from the `--dur-*` / `--ease-*` tokens in `src/styles/tokens.css`, no raw values in components. Animate only `transform`/`opacity` in hot paths. Keep the reduced-motion guard in `global.css` working (the pulsing activity dot is opacity-only for this reason).
- **Casing:** wire casing is fixed (see [the IPC contract](docs/architecture/tauri-ipc-contract.md)). Keep `src/ipc/types.ts` in lockstep with the Rust serde derives.
- **Testing (always-on):** add or update tests **in the same change** as any behaviour change. Match existing conventions:
  - **Core logic** → Rust unit tests next to the code (`#[cfg(test)] mod tests`); PTY/MCP/supervision behaviour → integration tests in `crates/podium-core/tests/` (injectable timings, so they finish in ms). New MCP tools get a `tools/list` + round-trip assertion. For Windows-unreliable tests prefer `#[cfg_attr(windows, ignore = "…")]` over `#[cfg(unix)]`: it still compiles and lints on Windows.
  - **New Tauri command** → thin forwarder; cover the logic in `podium-core`; update the IPC contract table in `docs/architecture/tauri-ipc-contract.md`.
  - **Frontend component/store** → a Vitest + jsdom + RTL file beside it (`Foo.test.tsx`), mocking `@tauri-apps/*` per file and seeding Zustand via `setState(..., true)` (see `WelcomeScreen.test.tsx` / `TodoDetailPane.test.tsx`). Assert user-visible behaviour and store action args.
  - Use **only fictitious** sample data. Run the relevant suite (`just test` / `just test-core` / `pnpm vitest run`) before reporting done. Skip a test only when genuinely infeasible (e.g. a canvas/xterm path jsdom can't exercise) and say so.
- **Documentation (always-on):** this file and `docs/architecture/` are the contributor docs; [`README.md`](./README.md) is the user-facing one. A change to what a user can **see or do** (feature, interaction, setting, `podium.yml` key, MCP tool) updates the README **in the same change**. Its _Feature tour_, _Settings_ and _Things you'd otherwise never find_ sections go stale first. Screenshots/GIFs in `docs/media/` are the maintainer's to record: if a change makes one wrong, or a feature is better shown than described, say so instead of replacing the file. Internal-only changes (layering, test harnesses, build plumbing) go in `docs/architecture/` alone (update the matching file, add one if none fits, plus its index line here).

## Git workflow

The repo is on GitHub (`RobbieMinderhoud/Podium`); use the **`gh` CLI**. **Every code change goes through a branch → PR**, never straight to `main`, which stays releasable. No GitHub issue per change: the PR (title + body: what and why) is the unit of work. Conversations that touch no code need no branch.

1. **Branch** from an up-to-date `main`, named `short-description` (e.g. `agent-activity-fix`), no issue-number prefix.
2. **Implement, commit, push.** Commit subjects are free-form, imperative (Keep-a-Changelog voice). **Never add a `Co-Authored-By` trailer** (a `commit-msg` hook also strips Anthropic ones).
3. **PR:** `gh pr create --base main --fill` (or explicit title/body). Add its entry under `## [Unreleased]` in `CHANGELOG.md` (CI fails without one). `ci.yml` must be green.
4. **Merge = release.** Nobody bumps the version or tags by hand. Before merging, set a `minor` or `major` label if more than a patch is warranted (`gh pr edit N --add-label minor`). Merge with `gh pr merge N --squash`; the branch auto-deletes. Get explicit user confirmation first, since merging publishes a release.

## Versioning & CI

- **SemVer.** The `[workspace.package]` version in the root `Cargo.toml` is the source of truth; `scripts/sync-version.sh` propagates it to `tauri.conf.json` and `package.json` (`just version-check` detects drift).
- **`CHANGELOG.md`** follows _Keep a Changelog_ and lists **only user-facing changes**. Every PR adds an entry under `## [Unreleased]`; the PR-only `changelog` job fails unless the PR touches `CHANGELOG.md` and that section holds at least one `- ` entry (`scripts/changelog.sh unreleased`). Script tests (`scripts/*.test.sh`) run in that job.
- **CI** (`.github/workflows/ci.yml`), on every PR and push to `main`: a `macos-latest` job (rustfmt, clippy `-D warnings`, `cargo test --workspace`, typecheck, ESLint, Vitest, `pnpm build`) and a `windows-latest` job (rustfmt, clippy, `cargo test -p podium-core` only). Both lint the **whole workspace**, so `cfg(windows)` code is linted. Both stub `dist/` first so `tauri::generate_context!` compiles. Windows-ignored tests still run on macOS.
- **Bundle build** (`.github/workflows/build.yml`): macOS and Windows matrix, each native. macOS packages `.app` + APFS `.dmg` (`scripts/make-dmg.sh`); Windows the NSIS `.exe`. Runs on `workflow_dispatch` and pushes to `main`, uploading installers as artifacts; on a tag ref (the release job's dispatch) it also attaches them to that tag's GitHub Release. Deliberately **no tag trigger**.
- **Releasing is automatic: every merge to `main` is a release.** The `release` job in `ci.yml` runs after the test jobs on each push to `main` (own non-cancelling concurrency group; the workflow never cancels runs on `main`). It checks out `main`, not the pushed sha, so a queued run sees the previous bump. An empty `## [Unreleased]` means already released: it exits. Otherwise the version is the latest `v*` tag bumped by `scripts/next-version.sh`: **patch by default**, `minor`/`major` by PR label (`major` wins). It renames `## [Unreleased]` to `## [X.Y.Z] - date` (`scripts/changelog.sh release`), runs `sync-version.sh --set` and `cargo update --workspace`, commits `Release vX.Y.Z [skip ci]` as `github-actions[bot]`, pushes it with the annotated tag `vX.Y.Z` (`--atomic`), creates the GitHub Release from the section, and dispatches `build.yml` on the tag: the one installer build per release. **Never bump the version or create a tag by hand**; `just version` is for emergencies.
- **Release push = deploy key.** `main`'s ruleset (PR required, linear history, no force-push/deletion) is bypassable only by the admin role and `DeployKey`; GitHub refuses the Actions app as a bypass actor on a user-owned repo, so a `GITHUB_TOKEN` push fails with `GH013`. The release job checks out with `ssh-key: secrets.RELEASE_DEPLOY_KEY` (private half of a write-enabled deploy key; the owner creates both) and pushes over SSH. A missing secret fails the first step with an `::error::` naming the setup commands. A deploy-key push *does* trigger workflows: `[skip ci]` stops the release commit re-running `ci.yml` and `build.yml`'s `main` trigger. Whether it also covers the tag is unclear, hence no tag trigger. Rotate = new deploy key + `gh secret set RELEASE_DEPLOY_KEY`, then delete the old key.
- **Not yet set up:** signed/notarized releases.

## Roadmap

All originally planned phases are shipped, plus post-v1.0 work (scratchpads, app-quit close guard, Auggie adapter, retro theme). This file and the code are the source of truth for current behaviour.
