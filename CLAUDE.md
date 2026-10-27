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
[WSL-backend mode](docs/architecture/wsl-backend.md)). Linux is kept in mind, not yet packaged.

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
- ✅ **Tauri IPC layer** (`src-tauri`): **101 commands**, per-attach terminal
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
  IPC wrappers over all 101 commands.
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
- **Casing:** wire casing is fixed (see [the IPC contract](docs/architecture/tauri-ipc-contract.md)); keep
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
    calls in `podium-core`; update the IPC contract table in `docs/architecture/tauri-ipc-contract.md`.
  - **Frontend component/store** → a Vitest + jsdom + RTL file beside it
    (`Foo.test.tsx`), mocking `@tauri-apps/*` per file and seeding Zustand
    via `setState(..., true)` (see `WelcomeScreen.test.tsx` /
    `TodoDetailPane.test.tsx`); assert user-visible behaviour and that store
    actions are called with the right args.
  - Use **only fictitious** sample data, and run the relevant suite
    (`just test` / `just test-core` / `pnpm vitest run`) before reporting
    done. Only skip adding a test when it is genuinely infeasible (e.g. a
    pure canvas/xterm path jsdom can't exercise) — say so explicitly.
- **Documentation (always-on):** this file and `docs/architecture/` are the contributor-facing
  documentation and [`README.md`](./README.md) the user-facing one. A change
  that alters what a user can **see or do** — a feature, a keyboard/mouse
  interaction, a setting, a `podium.yml` key, an MCP tool — updates the README
  **in the same change**, not as a follow-up. Its _Feature tour_, _Settings_
  and _Things you'd otherwise never find_ sections go stale first. Its
  screenshots/GIFs live in `docs/media/` and are the maintainer's to record —
  when a change makes one of them wrong, or a new feature is better shown
  than described, say so rather than replacing the file yourself. Internal-only
  changes (layering, test harnesses, build plumbing) stay in `docs/architecture/` alone (update the matching file, add one if none fits, and its index line here).

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
