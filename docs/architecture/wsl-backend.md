# WSL-backend mode (Windows)

On Windows, an optional mode runs everything Podium executes inside a WSL
distro. The configuration is `backend.json` (`backend_settings.rs`); the
switch lives in `platform.rs`.

## Configuration and switch

`backend.json` (`backend_settings.rs`) holds an optional WSL distro plus the
`nativeDriveProjects` opt-in (default off). Setup reads it **first thing**
(`init_backend` in `src-tauri/src/lib.rs`) and fixes `platform::init_wsl`
before the MCP server, the assistant or any project can spawn. Changing it
needs a restart. `platform::wsl()` is the switch: `Some(&WslBackend)` in WSL
mode, always `None` off Windows (`init_wsl` ignores a configured distro there).

## Per project, decided once

By default every project runs on the configured backend, a Windows-drive one
through `/mnt/<drive>`. With the `nativeDriveProjects` opt-in
(`WslBackend::native_drive_projects`, Settings → General → Backend),
`platform::wsl_for(root)` returns the backend only for a root inside a distro
(`wsl_path::is_wsl_path` — `\\wsl.localhost\…`, `\\wsl$\…`) and `None` for a
Windows-drive project, which then runs natively (cmd.exe, Windows git, Windows
`claude`).

The orchestrator resolves it **once, at project open** (`ProjectHandle::wsl`)
and passes that one value down, so a launch quoted for one shell is never run
by another:

- `AgentLaunchCtx::wsl` for `build_launch` quoting and the `--mcp-config`
  spelling,
- `PtyProcess::spawn` for the shell and the direct agent launch,
- `wsl_launch_guard_for` for the gate,
- the `worktree::*` functions for git,
- `AdapterRegistry::infos` for the availability probe.

`ProjectInfo.nativeInWslMode` tells the frontend, which then leaves the
terminal-shell setting (meant for the distro's shell) out of that project's
terminals, and the default terminal command follows the same flag.

The explicit-backend helpers are the `_in`/`_on(wsl, …)` variants
(`login_shell_in`, `login_shell_on`, `quote_arg_in`, `backend_path_in`,
`from_backend_path_in`, `run_shell_ok_on`, `command_exists_query_on`,
`git_command`). The plain ones use the global `wsl()` and are for
workspace-wide work only — the assistant and the MCP install. The launch gate
is only taken for a `wsl.exe` start (`run_gated` checks the program, `do_start`
the project's backend), so a native launch never waits behind a slow distro
start.

## Not behind `cfg(windows)`

Call sites branch at runtime on `wsl()`, so macOS/Linux CI compiles, lints and
tests the WSL code. Pure helpers take the mode as a parameter
(`wsl_login_shell`, `worktree_path`, `WslBackend::for_tests`) so both modes are
unit-testable anywhere. Tests never call `init_wsl` (it is process-global).

## Shell, quoting and probe

`login_shell()` is `wsl.exe -d <distro> -e bash -lic`; `login_shell_in(wsl,
cwd)` adds `--cd <linux path>`. `quote_arg` is POSIX (`quote_posix`), since
bash parses the string. `command_exists_query` is `command -v`. Agents skip the
Windows direct launch in `pty.rs` and take the shell path, like Unix.

## Paths

Podium keeps Windows `PathBuf`s everywhere (`\\wsl.localhost\<distro>\…`,
`C:\…`). Anything handed to a backend process goes through
`platform::backend_path` (`wsl_path::to_linux`: UNC → `/…`, `C:\x` →
`/mnt/c/x`). Anything a backend process prints goes back through
`from_backend_path` (`to_windows`). `git worktree list` paths are matched in the
Linux spelling and rebuilt under the project's own path, so a `\\wsl$\…`
project keeps its spelling.

## cwd and env

A PTY child's cwd rides on `--cd`, while `wsl.exe` itself starts in the local
temp dir (never the UNC path). Environment variables reach Linux **only** when
listed in `WSLENV`: `pty.rs` appends `KEY/u` for every `spec.env` key and
`TERM` (`platform::wslenv_with`). Add any new per-process variable to
`spec.env`, not straight to the command.

## One `wsl.exe` start at a time

`wsl.exe` can't take a burst of concurrent starts: with three or more at once,
all but two hang ~30 s and fail (`Wsl/Service/E_UNEXPECTED`), which the startup
burst of adapter probes and per-project git calls hit (and a failed probe is
cached for the run). So every launch goes through a process-wide gate, held
only for the *start* (spawn plus a 150 ms settle), never while a command runs:

- `platform::run_gated` for short commands (`run_shell_*`, git — the wait for
  output happens outside the gate),
- `wsl_launch_guard`/`spawn_gated` for long-lived ones (PTY, assistant, external sweep; held
  until the PTY reader runs).

Waiting for the gate gives up after 10 s with a warning and launches anyway, so
one hung `wsl.exe` cannot stall everything behind it. New code that starts
`wsl.exe` must use one of these. A no-op natively.

> **Never take the gate under the orchestrator's `inner` lock.** `do_start`
> reads the project's backend under a short lock of its own, takes the gate,
> then locks `inner` and hands the gate to `PtyProcess::spawn` by reference, so
> it drops after `inner`.

## No console flash

A console-subsystem child (git, `wsl.exe`, `cmd.exe`, bash) spawned via a plain
`Command`/`tokio::process::Command` from release-mode Podium (a GUI-subsystem
app, no console of its own) gets its own new console window on Windows unless
told otherwise — visible for the call's lifetime. `platform::hide_console_window`
sets `CREATE_NO_WINDOW`. Every non-ConPTY Windows spawn in this crate uses it:

- `run_shell_ok`, `run_shell_stdout`, `git_command`,
- the WSL networking check and distro listing,
- the assistant's and the external sweep's CLI children — tokio `Command`s, via `as_std_mut()`,
- Warp's `cmd /C start`,
- the Git board's `gh` calls via `gh_command`, several per project on every
  board refresh.

ConPTY-attached spawns (`process::pty`) are already windowless and need
nothing. New code that spawns a plain `Command` on Windows must use it too.

## git

git runs as `wsl.exe -d <distro> -e git` (`platform::git_command`), the
distro's git without a shell.

## Stop

Killing `wsl.exe` (the ConPTY child) takes the whole Linux session with it,
children included (`stopping_a_service_ends_its_linux_children`), so no
`setsid`/PGID bookkeeping is needed.

## MCP

The server still binds `127.0.0.1` on Windows; agents in WSL reach it through
**mirrored networking**. `backend_wsl_networking_mode` lets the app warn at
startup when it's off. The install registers `/mnt/c/…/Podium.exe mcp-bridge`
with the distro's CLIs, which start the bridge through interop.

## Tests

`crates/podium-core/tests/wsl_backend.rs` runs the real thing (ConPTY,
`wsl.exe`, a distro). It is Windows-only and opt-in: every test is `#[ignore]`d,
so CI compiles it and reports each test as *ignored* — CI never exercises WSL
itself. Run it on Windows with:

```
PODIUM_TEST_WSL_DISTRO=Ubuntu cargo test -p podium-core --test wsl_backend -- --ignored --test-threads=1
```

One thread, because a burst of concurrent `wsl.exe` starts fails. Without the
variable the run fails instead of passing vacuously, and the localhost test
skips with a message unless networking is mirrored. It covers cwd/env, a
Windows-drive project running in the distro by default, argv round-trip, stop,
config paths, the localhost reachability prerequisite, worktrees and interop.

`tests/wsl_native_drives.rs` is its twin with the `nativeDriveProjects` opt-in
on (a separate binary, since the backend is process-wide). A drive project's
service runs under cmd.exe while a distro project's still runs in WSL, and an
agent's `build_launch` quoting survives the real launch path on both (Windows
direct launch, bash in the distro). Run it the same way with `--test
wsl_native_drives`.
