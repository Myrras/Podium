# CLAUDE.md — Podium

<!-- rust-base:start -->
<!-- Synced from rust-base. Do not edit this block here; change it in rust-base. -->
## Shared CI and recipes (rust-base)

CI, release and installer builds come from
<https://github.com/RobbieMinderhoud/rust-base>. `.github/workflows/` holds thin
callers (`ci.yml`, `pr-checks.yml`, `build.yml`, `integration.yml`) of its
reusable workflows, pinned `@v2`. The callers and the shared config files
(`.gitattributes`, `.editorconfig`, `rust-toolchain.toml`, `.prettierrc.json`,
`.prettierignore`, `eslint.config.js`, `tsconfig*.json`,
`.github/dependabot.yml`) are synced from rust-base: **change them there**,
not here. Behaviour is documented in the rust-base README.

App-specific CI settings (runner, Windows test scope, extra recipe, release
title prefix, integration packages and env) live in `.github/rust-base.json`.
That file is owned by this repo.

- **Required checks on `main`:** `ci / ci-ok`, `pr-checks / Changelog entry`,
  `pr-checks / Release label`. Don't require `ci / frontend`, `ci / rust` or
  `ci / windows`. The labels `release:patch`, `release:minor`, `release:major`,
  `no-release` and `windows` must exist.
- **Release labels:** every PR carries exactly one of `release:patch`,
  `release:minor` or `release:major`, enforced by `pr-checks.yml`. Use
  `no-release` for CI, config or docs-only PRs. Merging to `main` releases:
  the version is the latest `v*` tag bumped by the highest label across the PRs
  merged since. Never bump the version or create a tag by hand; `just version`
  is for emergencies only.
- **Changelog gate:** every PR adds an entry under `## [Unreleased]` in
  `CHANGELOG.md` (user-facing changes only, _Keep a Changelog_). `pr-checks.yml`
  fails unless the PR touches `CHANGELOG.md` and that section holds at least one
  `- ` entry. The heading must stay exactly `## [Unreleased]`. `no-release` PRs
  need no entry.
- **`packageManager`:** don't add one to `package.json`. The shared workflows
  set pnpm 10; a second version fails setup.
- **Shared recipes** (`dev`, `dev-signed`, `build`, `build-signed`,
  `install-signed`, `check`, `test-core`, `format`, `version`, `version-check`,
  `rust`, `frontend`, `ci`, `sweep`) come from `ci/common.just` in rust-base.
  The justfile imports it with `import? '~/.rust-base/ci/common.just'`; link
  your clone with `ln -s /path/to/rust-base ~/.rust-base`. Don't redefine those
  names (just rejects duplicates) and never add `set allow-duplicate-recipes`:
  change the shared recipe in rust-base instead. Recipes that CI calls
  (`test-integration`) must not depend on shared recipes: in CI the optional
  import resolves to nothing.

### When to add the `windows` label

Windows CI runs on pushes to `main` and on PRs that carry the `windows` label
(adding the label re-runs CI for the PR). It gates the release. Add `windows`
to a PR when it:

- touches `cfg(windows)`, `cfg(unix)` or `target_os` code, or platform-specific modules
- changes file paths, path separators, drive letters or case sensitivity; process spawning, PTY or shell quoting; environment variables; WSL
- changes the Windows settings in `tauri.conf.json` (bundle, NSIS) or installer scripts
- adds or upgrades a dependency with platform-specific code (`tauri`, `windows-rs`, `portable-pty`, …) or bumps the Rust toolchain
- fixes a Windows bug

Skip it for frontend-only, docs-only and purely cross-platform logic changes. If unsure, add it: `main` runs Windows after the merge anyway, and a failure there blocks the release.
<!-- rust-base:end -->

Guidance for AI agents (and humans) working in this repository.

## What Podium is

Cross-platform **desktop cockpit for AI coding agents**, built in **Rust + Tauri**. Open a project folder and Podium runs its dev servers (`podium.yml`), ad-hoc terminals and AI coding agents (Claude Code; more adapters later), each in its own PTY with lifecycle supervision. A built-in **MCP server** lets agents inspect and orchestrate those processes, including spawning more agents. macOS and Windows are built and packaged (ConPTY on Windows); Windows has an optional [WSL-backend mode](docs/architecture/wsl-backend.md). Linux is kept in mind, not packaged.

> Naming: product and repo are **Podium**. Core crate `podium-core`; bundle id `com.podium.app`.

## Boilerplate (shared skeleton)

The shared skeleton (workspace shape, `justfile`, the CI callers, config files, design tokens, layering/IPC conventions) is a standalone template: <https://github.com/RobbieMinderhoud/rust-application-boilerplate>. It is the **upstream** of the skeleton; Podium adds its domain on top.

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
just dev            # shared: run the app (Vite + Tauri, hot reload)
just dev-signed     # shared: debug .app, stable signing identity, no hot reload (macOS)
just build          # shared: release bundle .app + .dmg
just build-signed   # shared: signed release .app, opens Finder on it (macOS)
just install-signed # shared: signed release, atomic install into /Applications (no Finder)
just check          # cargo check --workspace
just test-core      # podium-core tests only
just test-integration # opt-in WSL tests (Windows + a WSL distro only; run nightly by CI)
just ci             # shared: rust (fmt, clippy, test) + frontend (prettier, eslint, vitest, build)
just format         # rustfmt + prettier
just version 0.2.0  # bump + sync version across all manifests (emergencies only; CI bumps on merge)
just version-check  # verify versions are in sync
```

`rust`, `frontend`, `ci`, `build-signed` and `sweep` come from the shared `common.just` of `RobbieMinderhoud/ci-workflows`, imported via `import? '~/.ci-workflows/common.just'`. Locally, symlink your clone: `ln -s /path/to/ci-workflows ~/.ci-workflows`. Don't redefine those names here. `build`, `version` and `version-check` call the shared scripts in `~/.ci-workflows/scripts/`. `test-integration` must not depend on `common.just` (CI has no import).

Every recipe exports `CARGO_TARGET_DIR` at the **main checkout's** `target/`, so worktrees share one build cache. Cargo locks it: builds in different worktrees serialise. Release builds use cargo's default codegen (fat LTO + `codegen-units = 1` cost ~110s of link time for ~3.5 MB).

`just dev` ad-hoc-signs with a fresh identity on every rebuild, so the macOS keychain re-prompts each run. The `*-signed` recipes pass `src-tauri/tauri.dev-signed.conf.json` (sets only `bundle.macOS.signingIdentity`): one stable identity, so "Always Allow" sticks. They build a bundle, so **no hot reload**. Use `just dev` to iterate, `just dev-signed` when the keychain prompt is in the way, `just install-signed` to install into `/Applications`. Only `just build` produces the `.dmg` (shared `make-dmg.sh`, the CI release path); the signed recipes skip it (`--bundles app`).

Core integration tests (`crates/podium-core/tests/`) use **real PTYs** and a real MCP server on ephemeral ports, in plain `cargo test` (no Docker), on macOS, Windows and Linux (the CI Rust job runs on ubuntu). Tests that need Unix-only commands/shell syntax or process-group semantics are `#[cfg(unix)]` or `#[cfg_attr(windows, ignore = "…")]`, with a comment why. `tests/wsl_backend.rs` and `tests/wsl_native_drives.rs` are the opt-in Windows + WSL suite (`just test-integration`, nightly `integration.yml`; see [WSL-backend mode](docs/architecture/wsl-backend.md)). Frontend tests: Vitest + jsdom + React Testing Library (`src/test/setup.ts`); Tauri APIs are mocked per file.

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
  - Use **only fictitious** sample data. Run the relevant suite (`just ci` / `just test-core` / `pnpm vitest run`) before reporting done. Skip a test only when genuinely infeasible (e.g. a canvas/xterm path jsdom can't exercise) and say so.
- **Documentation (always-on):** this file and `docs/architecture/` are the contributor docs; [`README.md`](./README.md) is the user-facing one. A change to what a user can **see or do** (feature, interaction, setting, `podium.yml` key, MCP tool) updates the README **in the same change**. Its _Feature tour_, _Settings_ and _Things you'd otherwise never find_ sections go stale first. Screenshots/GIFs in `docs/media/` are the maintainer's to record: if a change makes one wrong, or a feature is better shown than described, say so instead of replacing the file. Internal-only changes (layering, test harnesses, build plumbing) go in `docs/architecture/` alone (update the matching file, add one if none fits, plus its index line here).

## Git workflow

The repo is on GitHub (`RobbieMinderhoud/Podium`); use the **`gh` CLI**. **Every code change goes through a branch → PR**, never straight to `main`, which stays releasable. No GitHub issue per change: the PR (title + body: what and why) is the unit of work. Conversations that touch no code need no branch.

1. **Branch** from an up-to-date `main`, named `short-description` (e.g. `agent-activity-fix`), no issue-number prefix.
2. **Implement, commit, push.** Commit subjects are free-form, imperative (Keep-a-Changelog voice). **Never add a `Co-Authored-By` trailer** (a `commit-msg` hook also strips Anthropic ones).
3. **PR:** `gh pr create --base main --fill` (or explicit title/body). Add its entry under `## [Unreleased]` in `CHANGELOG.md` (CI fails without one). `ci / ci-ok` must be green.
4. **Merge = release.** Nobody bumps the version or tags by hand. Every PR needs exactly one release label, `release:patch`, `release:minor` or `release:major` (`gh pr edit N --add-label release:minor`), enforced by `pr-checks.yml`. CI, config or docs-only PRs get `no-release` instead. Merge with `gh pr merge N --squash`; the branch auto-deletes. Get explicit user confirmation first, since merging publishes a release.

## Versioning & CI

- **SemVer.** The `[workspace.package]` version in the root `Cargo.toml` is the source of truth; the shared `sync-version.sh` propagates it to `tauri.conf.json` and `package.json` (`just version-check` detects drift).
- **`CHANGELOG.md`** follows _Keep a Changelog_ and lists **only user-facing changes**. Every PR adds an entry under `## [Unreleased]`; `pr-checks.yml` fails unless the PR touches `CHANGELOG.md` and that section holds at least one `- ` entry.
- **CI** is a set of thin callers in `.github/workflows/` of the reusable workflows in `RobbieMinderhoud/ci-workflows@v1` (behaviour lives there; see its README): `ci.yml` (checks, then release on push to `main`), `pr-checks.yml` (changelog entry + release label), `build.yml` (installers, dispatch-only: the release dispatches it on the tag) and `integration.yml` (nightly `just test-integration` on Windows with WSL, `Ubuntu-24.04`). Don't add a `packageManager` field to `package.json` (the shared workflows set pnpm 10; a second version fails setup).
- **Checks.** Rust checks (rustfmt, clippy `-D warnings`, `cargo test --workspace`) run on **ubuntu**, plus the frontend job (prettier, ESLint, Vitest, `pnpm build`). Windows (`cargo test -p podium-core`, clippy on the whole workspace) runs on pushes to `main` and on PRs that carry the `windows` label (adding the label re-runs CI for the PR). macOS is only built by `build.yml`, so macOS-only breakage shows up at release, not in a PR. Required checks: `ci / ci-ok`, `pr-checks / Changelog entry`, `pr-checks / Release label`; don't require `ci / frontend`, `ci / rust` or `ci / windows`. The `windows` label must exist in the repo.
- **Releasing is automatic: every merge to `main` is a release.** The `release` job in `ci.yml` runs after `ci` (Windows included) on each push to `main`. An empty `## [Unreleased]` means already released. Otherwise the version is the latest `v*` tag bumped **patch by default**, `minor`/`major` by the highest `release:*` label across the PRs merged since the tag. It commits `Release vX.Y.Z [skip ci]`, tags, creates the GitHub Release ("Podium vX.Y.Z") and dispatches `build.yml` on the tag, which builds the macOS `.dmg` and Windows NSIS installers and attaches them. `build.yml` must keep that name; a hand-pushed tag builds with `gh workflow run build.yml --ref vX.Y.Z`. **Never bump the version or create a tag by hand**; `just version` is for emergencies.
- **Release push = deploy key.** `main`'s ruleset (PR required, linear history, no force-push/deletion) is bypassable only by the admin role and `DeployKey`; GitHub refuses the Actions app as a bypass actor on a user-owned repo, so a `GITHUB_TOKEN` push fails with `GH013`. The release job checks out with `ssh-key: secrets.RELEASE_DEPLOY_KEY` (private half of a write-enabled deploy key; the owner creates both) and pushes over SSH. The caller sets `require-deploy-key: true`, so a missing secret fails the release up front instead of dying on `GH013`. A deploy-key push *does* trigger workflows: `[skip ci]` stops the release commit re-running `ci.yml` and `build.yml`'s `main` trigger. Whether it also covers the tag is unclear, hence no tag trigger. Rotate = new deploy key + `gh secret set RELEASE_DEPLOY_KEY`, then delete the old key.
- **Dependabot** (`.github/dependabot.yml`) opens weekly grouped PRs for GitHub Actions, Cargo and npm.
- **Not yet set up:** signed/notarized releases.

### When to add the `windows` label

Add `windows` to a PR when it:

- touches `cfg(windows)`, `cfg(unix)` or `target_os` code, or platform-specific modules
- changes file paths, path separators, drive letters or case sensitivity; process spawning, PTY or shell quoting; environment variables; WSL
- changes the Windows settings in `tauri.conf.json` (bundle, NSIS) or installer scripts
- adds or upgrades a dependency with platform-specific code (`tauri`, `windows-rs`, `portable-pty`, …) or bumps the Rust toolchain
- fixes a Windows bug

Skip it for frontend-only, docs-only and purely cross-platform logic changes. If unsure, add it: `main` runs Windows after the merge anyway, and a failure there blocks the release.

Podium has real Windows code. The Windows-sensitive spots:

- `crates/podium-core/src/platform.rs` and `platform/wsl_path.rs`: the WSL backend switch, `cmd`/PowerShell command quoting (`quote_arg_windows`, `parse_windows_argv`), Windows-vs-Unix paths, `open_in_warp`
- `crates/podium-core/src/process/pty.rs`: ConPTY vs Unix PTY, process-group kill (`killpg`) vs the Windows Job Object
- `crates/podium-core/src/backend_settings.rs` (`backend.json`), `agent/mod.rs` (agent command planning), `mcp/install.rs`, `worktree.rs` (no `\\?\` verbatim paths), `agent/folder_trust.rs`, `sweep.rs`
- `src-tauri/src/commands/process.rs` (default terminal: PowerShell vs `$SHELL`), `src-tauri/src/main.rs` (`windows_subsystem`), `src-tauri/tauri.windows.conf.json` (frameless window), the `bundle` block in `src-tauri/tauri.conf.json` and the NSIS build in `build.yml`
- tests: the `#[cfg_attr(windows, ignore = "…")]` / `#[cfg(unix)]` tests in `podium-core` (`orchestrator.rs`, `tests/mcp_server.rs`, `tests/pty_engine.rs`, `tests/supervision.rs`, `tests/agent_cleanup.rs`, `tests/assistant.rs`) and the nightly-only `tests/wsl_backend.rs` / `tests/wsl_native_drives.rs`

Windows CI only runs `cargo test -p podium-core`; the WSL suites need a real distro and run in `integration.yml`, not on PRs.

## Roadmap

All originally planned phases are shipped, plus post-v1.0 work (scratchpads, app-quit close guard, Auggie adapter, retro theme). This file and the code are the source of truth for current behaviour.
