//! The small OS gap Podium has to bridge: wrapping a command *string* in a
//! login shell, asking that shell whether a binary is on `PATH`, and quoting
//! a single argument for inclusion in that command string. Every other
//! platform difference is handled by portable-pty (ConPTY on Windows).
//!
//! Keeping these helpers in one place means the shell idiom (`$SHELL -lc` vs
//! `cmd /C`) is written once, not re-derived at each of the PTY-spawn /
//! adapter-probe / MCP-install call sites.

pub mod wsl_path;

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::{CoreError, CoreResult};

/// `CREATE_NO_WINDOW`: suppresses the console window Windows otherwise
/// allocates for a console-subsystem child (git, wsl.exe, cmd.exe, bash)
/// spawned from a GUI-subsystem process (release-mode Podium has no console
/// of its own) — without it, every such call flashes a window for its
/// lifetime. Never touches ConPTY-attached spawns (`process::pty`), which
/// are already windowless by construction; only plain `Command`/
/// `tokio::process::Command` calls need it.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Sets [`CREATE_NO_WINDOW`] on Windows; a no-op elsewhere. Apply to every
/// plain (non-ConPTY) `Command`/`tokio::process::Command` this crate spawns.
#[cfg(windows)]
pub fn hide_console_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_console_window(_cmd: &mut std::process::Command) {}

/// WSL-backend mode: on Windows, run everything Podium executes (services,
/// terminals, agents, the assistant, git) inside this WSL distro instead of
/// natively. Set once at startup from the persisted backend settings (see
/// [`crate::backend_settings`]); changing it needs an app restart.
///
/// Deliberately not behind `cfg(windows)`: call sites branch at runtime on
/// [`wsl()`], so every platform compiles, lints and tests that code. Only
/// [`init_wsl`] is platform-dependent, and it never enables WSL mode off
/// Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslBackend {
    distro: String,
    /// Opt-in (`backend.json`'s `nativeDriveProjects`, default off): run a
    /// project on a Windows drive natively instead of in the distro (see
    /// [`wsl_for`]). Off, every project runs in the distro via `/mnt/<drive>`.
    native_drive_projects: bool,
}

impl WslBackend {
    /// The WSL distro name, as `wsl.exe -d` takes it (e.g. `Ubuntu`).
    pub fn distro(&self) -> &str {
        &self.distro
    }

    /// Whether Windows-drive projects run natively (see [`wsl_for`]).
    pub fn native_drive_projects(&self) -> bool {
        self.native_drive_projects
    }

    /// A backend for unit tests of code that takes an `Option<&WslBackend>`
    /// (the real one is only ever built by [`init_wsl`]).
    #[cfg(test)]
    pub(crate) fn for_tests(distro: &str) -> Self {
        Self {
            distro: distro.to_string(),
            native_drive_projects: false,
        }
    }

    /// [`Self::for_tests`] with Windows-drive projects running natively.
    #[cfg(test)]
    pub(crate) fn for_tests_native_drives(distro: &str) -> Self {
        Self {
            native_drive_projects: true,
            ..Self::for_tests(distro)
        }
    }
}

static WSL: OnceLock<Option<WslBackend>> = OnceLock::new();

/// Decide the backend from the configured distro. Pure, so the decision is
/// testable on every platform: WSL mode needs Windows and a non-blank distro.
fn wsl_backend_for(
    distro: Option<&str>,
    native_drive_projects: bool,
    windows: bool,
) -> Option<WslBackend> {
    let distro = distro.map(str::trim).filter(|d| !d.is_empty())?;
    windows.then(|| WslBackend {
        distro: distro.to_string(),
        native_drive_projects,
    })
}

/// Fix the process-wide backend for this run. Call once, at startup, before
/// anything spawns (the MCP server, the orchestrator reopening projects).
/// Off Windows a configured distro is ignored (a settings file synced from a
/// Windows machine must not change macOS behaviour). `native_drive_projects`
/// is the opt-in described at [`wsl_for`]. Later calls are no-ops.
pub fn init_wsl(distro: Option<&str>, native_drive_projects: bool) {
    let backend = wsl_backend_for(distro, native_drive_projects, cfg!(windows));
    if backend.is_none() && distro.is_some_and(|d| !d.trim().is_empty()) {
        tracing::info!("ignoring configured WSL distro: WSL mode is Windows-only");
    }
    if WSL.set(backend).is_err() {
        tracing::warn!("init_wsl called more than once; keeping the first backend");
    }
}

/// The active WSL backend, or `None` for the native one (always `None` off
/// Windows, and before [`init_wsl`] runs).
pub fn wsl() -> Option<&'static WslBackend> {
    WSL.get().and_then(Option::as_ref)
}

/// The backend for a project rooted at `root`. By default that is simply
/// [`wsl`]: in WSL-backend mode every project runs in the distro, one on a
/// Windows drive through `/mnt/<drive>`. With the `nativeDriveProjects`
/// opt-in, only a project *inside* a distro (`\\wsl.localhost\…`) does; one
/// on a Windows drive runs natively (cmd.exe, Windows git, Windows `claude`).
///
/// Decide this **once per project** (the orchestrator does, at open) and
/// pass the result down, rather than re-deriving it from each process cwd:
/// one project must never mix backends. Workspace-wide work with no project
/// (the assistant, MCP install) uses [`wsl`].
pub fn wsl_for(root: &std::path::Path) -> Option<&'static WslBackend> {
    backend_for(wsl(), root)
}

/// [`wsl_for`] with the configured backend passed in, so it is testable
/// without [`init_wsl`].
fn backend_for<'a>(wsl: Option<&'a WslBackend>, root: &std::path::Path) -> Option<&'a WslBackend> {
    wsl.filter(|w| !w.native_drive_projects || wsl_path::is_wsl_path(root))
}

/// Serializes `wsl.exe` launches in WSL-backend mode. `wsl.exe` does not
/// survive a burst of concurrent starts: measured on WSL 2 (Windows 11), with
/// three or more launched at once all but two hang ~30 s and then fail
/// (`Wsl/Service/E_UNEXPECTED`, or a connection timeout), while the same six
/// started ~150 ms apart all succeed. Podium fires exactly such a burst at
/// startup (adapter probes, git for every open project, auto-start
/// services), and a failed adapter probe is cached for the whole run. The
/// limit is on starting, not on running: many long-lived `wsl.exe` sessions
/// coexist fine.
static WSL_LAUNCH_GATE: Mutex<()> = Mutex::new(());
/// How long a launch keeps the gate after spawning, so its `wsl.exe`
/// handshake is done before the next one starts.
const WSL_LAUNCH_SETTLE: Duration = Duration::from_millis(150);
/// How long a launch waits for the gate before going ahead without it. A
/// holder is bounded (spawn + [`WSL_LAUNCH_SETTLE`]), but one `wsl.exe` that
/// hangs in its spawn must not hold up every launch behind it for good —
/// risking the burst failure beats a frozen app.
const WSL_LAUNCH_GATE_TIMEOUT: Duration = Duration::from_secs(10);
const WSL_LAUNCH_GATE_POLL: Duration = Duration::from_millis(25);

/// Run a short-lived backend command to completion, capturing its output
/// like [`std::process::Command::output`] — except that stdio the caller
/// did not set is inherited, so set `stdout`/`stderr` to `piped()` for
/// whatever should be read back. In WSL-backend mode only the *start* goes
/// through the launch gate (see [`WSL_LAUNCH_GATE`]): the limit is on
/// starting, and a slow login shell (nvm, `claude mcp get`) must not make
/// every launch behind it wait for it to finish.
pub fn run_gated(cmd: &mut std::process::Command) -> std::io::Result<std::process::Output> {
    // Only a `wsl.exe` start needs the gate: a native git or `cmd.exe` call
    // (a Windows-drive project with `nativeDriveProjects`) must not queue
    // behind a slow distro launch.
    let gate = if launches_wsl(cmd) {
        wsl_launch_guard()
    } else {
        None
    };
    let child = cmd.spawn();
    drop(gate);
    child?.wait_with_output()
}

/// Holds the WSL launch gate for a long-lived launch; dropping it waits
/// [`WSL_LAUNCH_SETTLE`] and then lets the next launch through. Drop it once
/// the process can make progress — for a PTY child that is after its reader
/// thread runs, since ConPTY holds the child until the reader answers the
/// startup cursor query. Never hold it across a lock other code takes on a
/// UI-facing path: the settle sleep runs in its `Drop`.
pub struct WslLaunchGuard {
    /// `None` when the gate timed out and this launch went ahead anyway.
    gate: Option<std::sync::MutexGuard<'static, ()>>,
}

impl Drop for WslLaunchGuard {
    fn drop(&mut self) {
        if self.gate.is_some() {
            std::thread::sleep(WSL_LAUNCH_SETTLE);
        }
    }
}

/// Whether `cmd` starts `wsl.exe`, i.e. whether it needs the launch gate.
fn launches_wsl(cmd: &std::process::Command) -> bool {
    std::path::Path::new(cmd.get_program())
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("wsl.exe") || n.eq_ignore_ascii_case("wsl"))
}

/// Take the WSL launch gate for a launch (WSL-backend mode); `None`
/// natively. Waits at most [`WSL_LAUNCH_GATE_TIMEOUT`].
pub fn wsl_launch_guard() -> Option<WslLaunchGuard> {
    wsl_launch_guard_for(wsl())
}

/// [`wsl_launch_guard`] for a launch on backend `wsl` (see [`wsl_for`]):
/// `None` for a native launch, even in WSL-backend mode, so starting a
/// service in a natively-run Windows-drive project never waits on a slow
/// distro start.
pub fn wsl_launch_guard_for(wsl: Option<&WslBackend>) -> Option<WslLaunchGuard> {
    wsl?;
    let gate = lock_with_deadline(&WSL_LAUNCH_GATE, WSL_LAUNCH_GATE_TIMEOUT);
    if gate.is_none() {
        tracing::warn!(
            "wsl.exe launch gate still held after {:?}; launching without it",
            WSL_LAUNCH_GATE_TIMEOUT
        );
    }
    Some(WslLaunchGuard { gate })
}

/// `mutex.lock()`, giving up after `timeout` (`std`'s `Mutex` has no timed
/// lock, so this polls `try_lock`). A poisoned lock is still taken: the gate
/// guards no data.
fn lock_with_deadline<T>(
    mutex: &Mutex<T>,
    timeout: Duration,
) -> Option<std::sync::MutexGuard<'_, T>> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return None;
        }
        std::thread::sleep(WSL_LAUNCH_GATE_POLL.min(deadline - now));
    }
}

/// Start a long-lived backend process through the WSL launch gate: `f`
/// spawns it, and the gate is held for the spawn plus
/// [`WSL_LAUNCH_SETTLE`]. Natively it is just `f()`.
pub fn spawn_gated<T>(f: impl FnOnce() -> T) -> T {
    let _gate = wsl_launch_guard();
    f()
}

/// The login shell plus the args that precede a command *string*, so that
/// `Command::new(program).args(prefix).arg(cmd)` runs `cmd` through it.
///
/// Unix: `$SHELL -lic <cmd>` — login *and* interactive, so both
/// `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc` PATH/env edits (nvm,
/// Homebrew shellenv, Docker Desktop alternatives, …) apply — some installers
/// only add to the interactive-only file. Windows: `%COMSPEC% /C <cmd>`
/// (defaults to `cmd.exe`). WSL-backend mode: `wsl.exe -d <distro> -e bash
/// -lic <cmd>`, the same login + interactive idiom inside the distro.
///
/// Runs in the distro's default directory in WSL mode; use
/// [`login_shell_in`] where the command needs a working directory.
pub fn login_shell() -> (String, Vec<String>) {
    login_shell_on(wsl())
}

/// [`login_shell`] on backend `wsl` (see [`wsl_for`]).
pub fn login_shell_on(wsl: Option<&WslBackend>) -> (String, Vec<String>) {
    match wsl {
        Some(w) => wsl_login_shell(w, None),
        None => native_login_shell(),
    }
}

/// [`login_shell`] on backend `wsl` (see [`wsl_for`]) for a command that
/// must run in `cwd`. In WSL mode the directory is passed to `wsl.exe --cd`
/// as its Linux path (a `cwd` outside the backend distro is an error); the
/// caller should then give `wsl.exe` itself a plain local working directory,
/// not the `\\wsl.localhost` one. Natively this is the native login shell,
/// and the caller sets the process's working directory as before.
pub fn login_shell_in(
    wsl: Option<&WslBackend>,
    cwd: &std::path::Path,
) -> CoreResult<(String, Vec<String>)> {
    match wsl {
        Some(w) => {
            let linux_cwd = wsl_path::to_linux(w.distro(), cwd)?;
            Ok(wsl_login_shell(w, Some(&linux_cwd)))
        }
        None => Ok(native_login_shell()),
    }
}

fn native_login_shell() -> (String, Vec<String>) {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, vec!["-lic".to_string()])
    }
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        (shell, vec!["/C".to_string()])
    }
}

/// `wsl.exe -d <distro> [--cd <linux_cwd>] -e bash -lic`. `-e` execs bash
/// directly (no extra default-shell layer), so the command string is parsed
/// exactly once, by bash.
fn wsl_login_shell(wsl: &WslBackend, linux_cwd: Option<&str>) -> (String, Vec<String>) {
    let mut args = vec!["-d".to_string(), wsl.distro().to_string()];
    if let Some(cwd) = linux_cwd {
        args.extend(["--cd".to_string(), cwd.to_string()]);
    }
    args.extend(["-e", "bash", "-lic"].map(String::from));
    ("wsl.exe".to_string(), args)
}

/// `path` spelled the way the backend's processes see it: its Linux path
/// inside the distro in WSL-backend mode (`C:\Users\…` → `/mnt/c/Users/…`),
/// the path itself natively. For paths handed to a child as an argument
/// (config files, rules files) — a `--cd` goes through [`login_shell_in`].
pub fn backend_path(path: &std::path::Path) -> CoreResult<String> {
    backend_path_in(wsl(), path)
}

/// [`backend_path`] for a process on backend `wsl` (see [`wsl_for`]).
pub fn backend_path_in(wsl: Option<&WslBackend>, path: &std::path::Path) -> CoreResult<String> {
    match wsl {
        Some(w) => wsl_path::to_linux(w.distro(), path),
        None => Ok(path.to_string_lossy().into_owned()),
    }
}

/// In WSL-backend mode, the distro's networking mode as `wslinfo
/// --networking-mode` reports it (`mirrored`, `nat`, …); `None` natively or
/// when `wslinfo` is unavailable (older WSL). Agents inside WSL reach
/// Podium's MCP server on `127.0.0.1` only with mirrored networking. Runs
/// `wslinfo` directly (`wsl.exe -e`, like [`git_command`]), not through the
/// login shell, so a chatty `.bashrc` cannot end up in the answer. Blocks —
/// call from a blocking-friendly context.
pub fn wsl_networking_mode() -> Option<String> {
    let w = wsl()?;
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(["-d", w.distro(), "-e", "wslinfo", "--networking-mode"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    hide_console_window(&mut cmd);
    let output = run_gated(&mut cmd).ok()?;
    if !output.status.success() {
        return None;
    }
    parse_networking_mode(&String::from_utf8_lossy(&output.stdout))
}

/// The mode out of `wslinfo --networking-mode` output: its last non-empty
/// line, trimmed — should anything still print ahead of it.
fn parse_networking_mode(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_string)
}

/// The reverse of [`backend_path`]: a path printed by a backend process (e.g.
/// git), as a Windows path in WSL-backend mode, unchanged natively.
pub fn from_backend_path(path: &str) -> std::path::PathBuf {
    from_backend_path_in(wsl(), path)
}

/// [`from_backend_path`] for a process on backend `wsl` (see [`wsl_for`]).
pub fn from_backend_path_in(wsl: Option<&WslBackend>, path: &str) -> std::path::PathBuf {
    match wsl {
        Some(w) => wsl_path::to_windows(w.distro(), path),
        None => std::path::PathBuf::from(path),
    }
}

/// `existing` `WSLENV` plus `KEY/u` for each of `keys` not already listed, so
/// those variables set on the Windows side of `wsl.exe` reach the Linux side
/// (WSL passes nothing through otherwise). `/u` = only when launching WSL
/// from Windows; no `/p` path translation — callers translate paths
/// themselves where a value is one.
pub fn wslenv_with<'a>(existing: Option<&str>, keys: impl IntoIterator<Item = &'a str>) -> String {
    let mut entries: Vec<String> = existing
        .unwrap_or_default()
        .split(':')
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect();
    for key in keys {
        let listed = entries.iter().any(|e| e.split('/').next() == Some(key));
        if !listed {
            entries.push(format!("{key}/u"));
        }
    }
    entries.join(":")
}

/// A shell command line that exits 0 iff `binary` resolves on `PATH`.
/// Unix and WSL mode use `command -v`; native Windows uses `where`.
pub fn command_exists_query(binary: &str) -> String {
    command_exists_query_on(wsl(), binary)
}

/// [`command_exists_query`] for the shell of backend `wsl` (see [`wsl_for`]).
pub fn command_exists_query_on(wsl: Option<&WslBackend>, binary: &str) -> String {
    if wsl.is_some() {
        return format!("command -v {binary}");
    }
    #[cfg(unix)]
    {
        format!("command -v {binary}")
    }
    #[cfg(windows)]
    {
        format!("where {binary}")
    }
}

/// Quote a single argument for inclusion in a command line run through
/// [`login_shell`]: POSIX single-quote quoting for `$SHELL -lc` on Unix and
/// for bash in WSL-backend mode, Windows argv quoting (the C-runtime
/// backslash/quote escaping rules that `CommandLineToArgvW` reverses) for
/// `cmd.exe /C` on native Windows.
///
/// The two are not interchangeable: `cmd.exe` has no concept of POSIX
/// single-quote grouping, so a POSIX-quoted argument containing a `'` (e.g.
/// `You're …`) is not treated as one quoted token — `cmd.exe` still splits it
/// on whitespace, truncating the argument at the first space.
pub fn quote_arg(arg: &str) -> CoreResult<String> {
    quote_arg_in(wsl(), arg)
}

/// [`quote_arg`] for a command run on backend `wsl` (see [`wsl_for`]).
pub fn quote_arg_in(wsl: Option<&WslBackend>, arg: &str) -> CoreResult<String> {
    if wsl.is_some() {
        return quote_posix(arg);
    }
    quote_native(arg)
}

/// POSIX single-quote quoting, for a command string a POSIX shell parses.
pub fn quote_posix(arg: &str) -> CoreResult<String> {
    shlex::try_quote(arg)
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| CoreError::InvalidInput(format!("cannot quote argument: {e}")))
}

/// The native shell's quoting: [`quote_posix`] on Unix, argv quoting on
/// Windows.
fn quote_native(arg: &str) -> CoreResult<String> {
    #[cfg(unix)]
    {
        quote_posix(arg)
    }
    #[cfg(windows)]
    {
        if arg.contains('\0') {
            return Err(CoreError::InvalidInput(
                "cannot quote argument: contains a nul byte".to_string(),
            ));
        }
        Ok(quote_arg_windows(arg))
    }
}

/// Windows argv quoting: wraps `arg` in double quotes and escapes embedded
/// quotes/backslashes per the rules every Windows argv parser (the C
/// runtime's, `CommandLineToArgvW`) expects — see Microsoft's "Parsing C++
/// Command-Line Arguments". Only quotes when necessary so simple args stay
/// readable in the command string shown in the UI.
#[cfg(windows)]
fn quote_arg_windows(arg: &str) -> String {
    let needs_quotes = arg.is_empty() || arg.contains([' ', '\t', '"']);
    if !needs_quotes {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Resolve `program` to an absolute executable path by walking `PATH`
/// looking for a *file* — unlike portable-pty's own internal Windows PATH
/// search (used by `process::pty` to spawn agent binaries directly, bypassing
/// `cmd.exe`), which resolves a bare name to the first `PATH` entry that
/// merely `exists()`, directory or not. A same-named data/log folder earlier
/// in `PATH` than the real executable — e.g. Claude Code's own
/// `%LOCALAPPDATA%\claude\` ahead of `claude.exe` further down `PATH` — makes
/// portable-pty hand that *directory* to `CreateProcessW` as
/// `lpApplicationName`, which Windows rejects with `ERROR_ACCESS_DENIED`
/// (the process never even starts). Falls back to `program` unresolved (today's
/// behaviour, buggy PATH order and all) when no file match is found, so a
/// binary that truly isn't installed still fails the same way it always has.
#[cfg(windows)]
pub fn resolve_windows_executable(program: &str) -> String {
    if program.contains(['\\', '/']) {
        return program.to_string(); // already a path; nothing to search.
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return program.to_string();
    };
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    resolve_in_path(program, &path_var, &pathext)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string())
}

/// The testable core of [`resolve_windows_executable`]: `path_var`/`pathext`
/// injected rather than read from the environment, so tests don't have to
/// mutate real process-global env vars (racy under parallel test execution).
#[cfg(windows)]
fn resolve_in_path(
    program: &str,
    path_var: &std::ffi::OsStr,
    pathext: &str,
) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            let with_ext = dir.join(format!("{program}{ext}"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

/// Run `cmd` through the login shell, discarding all output; returns whether
/// it exited successfully. Output is never captured, so nothing the command
/// prints can leak into logs or errors.
pub fn run_shell_ok(cmd: &str) -> bool {
    run_shell_ok_on(wsl(), cmd)
}

/// [`run_shell_ok`] through the login shell of backend `wsl` (see
/// [`wsl_for`]).
pub fn run_shell_ok_on(wsl: Option<&WslBackend>, cmd: &str) -> bool {
    let (shell, prefix) = login_shell_on(wsl);
    let mut command = std::process::Command::new(shell);
    command
        .args(prefix)
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console_window(&mut command);
    run_gated(&mut command)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run `cmd` through the login shell, discarding stderr; returns its stdout
/// when the command exits successfully, otherwise `None`. Used only where a
/// command's non-secret stdout must be read (e.g. the Auggie installed-check's
/// JSON listing), not just its exit status.
pub fn run_shell_stdout(cmd: &str) -> Option<String> {
    let (shell, prefix) = login_shell();
    let mut command = std::process::Command::new(shell);
    command
        .args(prefix)
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    hide_console_window(&mut command);
    let output = run_gated(&mut command).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A [`std::process::Command`] that runs git, for the worktree operations
/// (args follow, e.g. `-C <root> worktree list`). Natively that is the
/// [`git_binary`], invoked directly; in WSL-backend mode it is `wsl.exe -d
/// <distro> -e git` — the distro's own git, run without a shell, so path
/// arguments must be in the backend's spelling ([`backend_path`]).
/// On backend `wsl` (see [`wsl_for`]).
pub fn git_command(wsl: Option<&WslBackend>) -> std::process::Command {
    let mut cmd = match wsl {
        Some(w) => {
            let mut cmd = std::process::Command::new("wsl.exe");
            cmd.args(["-d", w.distro(), "-e", "git"]);
            cmd
        }
        None => std::process::Command::new(git_binary()),
    };
    hide_console_window(&mut cmd);
    cmd
}

/// The native `git` binary to invoke directly (bypassing the login shell)
/// for worktree operations, which can spawn several git calls per action
/// (list/create/remove) — a login shell per call is slow on heavy shell
/// profiles. Resolved once via the login shell's `command -v git` so
/// shell-profile PATH edits (e.g. a Homebrew git) are still honoured, then
/// reused directly; falls back to the bare name if resolution fails.
/// Windows PATH is process-wide, so no shell round-trip is needed there.
fn git_binary() -> &'static str {
    static GIT_PATH: OnceLock<String> = OnceLock::new();
    GIT_PATH
        .get_or_init(|| {
            #[cfg(windows)]
            {
                "git".to_string()
            }
            #[cfg(unix)]
            {
                run_shell_stdout(&command_exists_query("git"))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "git".to_string())
            }
        })
        .as_str()
}

/// The `warp://` URI that opens a new Warp tab at `path` (percent-encoded).
pub fn warp_uri(path: &std::path::Path) -> String {
    let mut encoded = String::new();
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(b as char)
            }
            _ => encoded.push_str(&format!("%{b:02X}")),
        }
    }
    format!("warp://action/new_tab?path={encoded}")
}

/// Open a new Warp tab at `path` via the OS URI handler. A missing Warp
/// install surfaces as a launcher failure (nothing handles the scheme).
pub fn open_in_warp(path: &std::path::Path) -> CoreResult<()> {
    let uri = warp_uri(path);
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(&uri).status();
    #[cfg(windows)]
    let status = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", "", &uri]);
        hide_console_window(&mut cmd);
        cmd.status()
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(&uri).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        // The launcher's own output never reaches the message.
        _ => Err(CoreError::InvalidInput(
            "could not open Warp — is it installed?".to_string(),
        )),
    }
}

/// Reverses [`quote_arg_windows`]'s escaping, mirroring the documented
/// `CommandLineToArgvW` / C-runtime argv-parsing algorithm.
///
/// Used in production to un-quote an agent's `spec.command` before spawning
/// it directly (see `process::pty` — agent launches bypass `cmd.exe` on
/// Windows entirely, so the tokens this recovers are handed to
/// `CommandBuilder::arg` as-is, letting portable-pty apply the *one* correct
/// quoting pass instead of re-parsing an already-quoted string). Also used as
/// a dependency-free stand-in for actually invoking `cmd.exe` in the quoting
/// round-trip tests below, the same role `shlex::split` plays for Unix.
#[cfg(windows)]
pub(crate) fn parse_windows_argv(cmd: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut backslashes = 1;
                while chars.peek() == Some(&'\\') {
                    backslashes += 1;
                    chars.next();
                }
                if chars.peek() == Some(&'"') {
                    cur.extend(std::iter::repeat_n('\\', backslashes / 2));
                    if backslashes % 2 == 1 {
                        chars.next();
                        cur.push('"');
                    }
                } else {
                    cur.extend(std::iter::repeat_n('\\', backslashes));
                }
            }
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networking_mode_is_the_last_non_empty_line() {
        assert_eq!(
            parse_networking_mode("mirrored\n").as_deref(),
            Some("mirrored")
        );
        assert_eq!(
            parse_networking_mode("Welcome to Ubuntu\r\nnat\r\n\r\n").as_deref(),
            Some("nat")
        );
        assert_eq!(parse_networking_mode(" \n"), None);
    }

    fn ubuntu() -> WslBackend {
        WslBackend::for_tests("Ubuntu")
    }

    #[test]
    fn lock_with_deadline_takes_a_free_lock() {
        let mutex = Mutex::new(());
        assert!(lock_with_deadline(&mutex, Duration::ZERO).is_some());
    }

    #[test]
    fn lock_with_deadline_gives_up_on_a_held_lock() {
        let mutex = Mutex::new(());
        let _held = mutex.lock().unwrap();
        let start = std::time::Instant::now();
        assert!(lock_with_deadline(&mutex, Duration::from_millis(60)).is_none());
        assert!(start.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn lock_with_deadline_waits_for_a_lock_released_in_time() {
        let mutex = std::sync::Arc::new(Mutex::new(()));
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let holder = {
            let mutex = std::sync::Arc::clone(&mutex);
            std::thread::spawn(move || {
                let _held = mutex.lock().unwrap();
                locked_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(50));
            })
        };
        locked_rx.recv().unwrap();
        assert!(lock_with_deadline(&mutex, Duration::from_secs(5)).is_some());
        holder.join().unwrap();
    }

    #[test]
    fn lock_with_deadline_takes_a_poisoned_lock() {
        let mutex = std::sync::Arc::new(Mutex::new(()));
        let poisoner = std::sync::Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _held = poisoner.lock().unwrap();
            panic!("poison the gate");
        })
        .join();
        assert!(mutex.is_poisoned());
        assert!(lock_with_deadline(&mutex, Duration::ZERO).is_some());
    }

    #[test]
    fn run_gated_captures_piped_output() {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "echo hi"]);
            c
        } else {
            let mut c = std::process::Command::new("sh");
            c.args(["-c", "echo hi"]);
            c
        };
        cmd.stdout(std::process::Stdio::piped());
        let output = run_gated(&mut cmd).unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi");
    }

    #[test]
    fn wsl_login_shell_runs_bash_in_the_distro() {
        let (program, args) = wsl_login_shell(&ubuntu(), None);
        assert_eq!(program, "wsl.exe");
        assert_eq!(args, ["-d", "Ubuntu", "-e", "bash", "-lic"]);
    }

    #[test]
    fn wsl_login_shell_passes_the_cwd_before_the_command() {
        let (_, args) = wsl_login_shell(&ubuntu(), Some("/home/me/mijn project"));
        assert_eq!(
            args,
            [
                "-d",
                "Ubuntu",
                "--cd",
                "/home/me/mijn project",
                "-e",
                "bash",
                "-lic"
            ]
        );
    }

    #[test]
    fn posix_quoting_round_trips_through_a_posix_tokenizer() {
        // What WSL mode hands bash, on every host OS.
        for arg in [
            "You're the assigned agent for to-do 'Fix login bug'",
            r#"fix the "login" bug; $HOME `id` \ back"#,
            "multi\nline\nprompt",
            "",
        ] {
            let quoted = quote_posix(arg).unwrap();
            assert_eq!(
                shlex::split(&quoted).unwrap(),
                vec![arg.to_string()],
                "{arg:?}"
            );
        }
    }

    #[test]
    fn from_backend_path_is_the_path_itself_natively() {
        assert_eq!(
            from_backend_path("/home/me/app/.git"),
            std::path::PathBuf::from("/home/me/app/.git")
        );
    }

    #[test]
    fn backend_path_is_the_path_itself_natively() {
        let path = std::env::temp_dir().join("mcp").join("agent.json");
        assert_eq!(
            backend_path(&path).unwrap(),
            path.to_string_lossy().into_owned()
        );
    }

    #[test]
    fn wslenv_appends_each_key_once() {
        assert_eq!(
            wslenv_with(None, ["TERM", "PODIUM_PROCESS_ID"]),
            "TERM/u:PODIUM_PROCESS_ID/u"
        );
        // Existing entries (with or without flags) are kept and not repeated.
        assert_eq!(
            wslenv_with(Some("USERPROFILE/p:TERM"), ["TERM", "FOO"]),
            "USERPROFILE/p:TERM:FOO/u"
        );
        assert_eq!(wslenv_with(Some(""), ["A", "A"]), "A/u");
        assert_eq!(wslenv_with(Some("X/up"), []), "X/up");
    }

    #[test]
    fn the_native_backend_is_used_without_init() {
        // Tests never call init_wsl, so every helper takes the native path.
        assert!(wsl().is_none());
        assert_eq!(login_shell(), native_login_shell());
        let dir = std::env::temp_dir();
        assert_eq!(login_shell_in(None, &dir).unwrap(), native_login_shell());
    }

    #[test]
    fn drive_projects_stay_in_the_distro_unless_opted_in() {
        let in_distro = std::path::Path::new(r"\\wsl.localhost\Ubuntu\home\me\app");
        let on_drive = std::path::Path::new(r"C:\Users\me\app");
        // Default: every project runs in the distro, as before the opt-in.
        let w = WslBackend::for_tests("Ubuntu");
        assert!(backend_for(Some(&w), in_distro).is_some());
        assert!(backend_for(Some(&w), on_drive).is_some());
        // Opted in: only a project inside the distro does.
        let native = WslBackend::for_tests_native_drives("Ubuntu");
        assert!(backend_for(Some(&native), in_distro).is_some());
        assert!(backend_for(Some(&native), on_drive).is_none());
        assert!(backend_for(None, in_distro).is_none());
        // An opted-in drive project is quoted, spelled and probed natively.
        assert_eq!(
            quote_arg_in(backend_for(Some(&native), on_drive), "it's").unwrap(),
            quote_native("it's").unwrap()
        );
        assert_eq!(
            backend_path_in(backend_for(Some(&native), on_drive), on_drive).unwrap(),
            on_drive.to_string_lossy()
        );
        assert_eq!(
            login_shell_on(backend_for(Some(&native), on_drive)),
            native_login_shell()
        );
        assert_eq!(
            command_exists_query_on(backend_for(Some(&w), on_drive), "claude"),
            "command -v claude"
        );
    }

    #[test]
    fn only_a_wsl_launch_takes_the_gate() {
        assert!(launches_wsl(&std::process::Command::new("wsl.exe")));
        assert!(launches_wsl(&std::process::Command::new(
            r"C:\Windows\System32\WSL.EXE"
        )));
        assert!(!launches_wsl(&std::process::Command::new("git")));
        assert!(!launches_wsl(&std::process::Command::new("cmd.exe")));
        // A native launch never waits on the gate, even in WSL-backend mode.
        assert!(wsl_launch_guard_for(None).is_none());
    }

    #[test]
    fn wsl_backend_needs_windows_and_a_distro() {
        assert_eq!(
            wsl_backend_for(Some("Ubuntu"), false, true),
            Some(WslBackend {
                distro: "Ubuntu".to_string(),
                native_drive_projects: false,
            })
        );
        assert_eq!(
            wsl_backend_for(Some("  Ubuntu-24.04 "), true, true).map(|w| w.distro().to_string()),
            Some("Ubuntu-24.04".to_string())
        );
        assert!(wsl_backend_for(Some("Ubuntu"), true, true)
            .unwrap()
            .native_drive_projects());
        assert_eq!(wsl_backend_for(None, true, true), None);
        assert_eq!(wsl_backend_for(Some("   "), false, true), None);
        // Off Windows a configured distro is ignored.
        assert_eq!(wsl_backend_for(Some("Ubuntu"), true, false), None);
    }

    #[test]
    fn shell_wraps_a_command_string() {
        let (shell, prefix) = login_shell();
        assert!(!shell.is_empty());
        assert_eq!(prefix.len(), 1); // one flag before the command line
    }

    #[test]
    fn exists_query_targets_the_binary() {
        assert!(command_exists_query("podium").contains("podium"));
    }

    #[test]
    fn run_shell_ok_reflects_exit_status() {
        // A binary that never exists must probe false; the trivial success
        // command differs per OS.
        assert!(!run_shell_ok(&command_exists_query(
            "podium-nonexistent-binary-xyz"
        )));
        #[cfg(unix)]
        assert!(run_shell_ok("true"));
        #[cfg(windows)]
        assert!(run_shell_ok("exit 0"));
    }

    #[cfg(unix)]
    #[test]
    fn quote_arg_round_trips_through_a_posix_tokenizer() {
        let cases = [
            "claude",
            "fix the \"login\" bug",
            "You're the assigned agent",
        ];
        let quoted: Vec<String> = cases.iter().map(|a| quote_arg(a).unwrap()).collect();
        let line = quoted.join(" ");
        assert_eq!(shlex::split(&line).unwrap(), cases);
    }

    #[cfg(windows)]
    #[test]
    fn quote_arg_round_trips_through_windows_argv_parsing() {
        // The regression case: an embedded apostrophe (e.g. a to-do prompt
        // starting "You're …") must survive as one argument, not get split
        // apart by cmd.exe, which has no notion of POSIX single-quoting.
        let cases = [
            "claude",
            "You're the assigned agent for to-do X",
            "fix the \"login\" bug",
            r"trailing backslash \",
            r"C:\path with spaces\",
        ];
        let quoted: Vec<String> = cases.iter().map(|a| quote_arg(a).unwrap()).collect();
        let line = quoted.join(" ");
        assert_eq!(parse_windows_argv(&line), cases);
    }

    #[cfg(windows)]
    #[test]
    fn quote_arg_wraps_an_empty_argument_in_quotes() {
        assert_eq!(quote_arg("").unwrap(), "\"\"");
    }

    #[test]
    fn quote_arg_rejects_nul_bytes() {
        assert!(quote_arg("bad\0arg").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn resolve_in_path_skips_a_same_named_directory_and_finds_the_real_exe() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory that merely happens to share the program's name (e.g.
        // Claude Code's own %LOCALAPPDATA%\claude\ data folder) sits in an
        // earlier PATH entry than the real executable.
        let decoy_dir = tmp.path().join("decoy");
        std::fs::create_dir_all(decoy_dir.join("myprog")).unwrap();
        let real_dir = tmp.path().join("real");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("myprog.EXE"), "").unwrap();

        let path_var = std::env::join_paths([&decoy_dir, &real_dir]).unwrap();
        let resolved = resolve_in_path("myprog", &path_var, ".EXE").unwrap();

        assert_eq!(resolved, real_dir.join("myprog.EXE"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_in_path_finds_an_extensionless_match_directly() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("myprog"), "").unwrap();

        let path_var = std::env::join_paths([tmp.path()]).unwrap();
        let resolved = resolve_in_path("myprog", &path_var, ".EXE").unwrap();

        assert_eq!(resolved, tmp.path().join("myprog"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_in_path_returns_none_when_nothing_file_shaped_matches() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("myprog")).unwrap();

        let path_var = std::env::join_paths([tmp.path()]).unwrap();
        assert!(resolve_in_path("myprog", &path_var, ".EXE").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn resolve_windows_executable_falls_back_to_the_bare_name_when_unresolved() {
        assert_eq!(
            resolve_windows_executable("podium-nonexistent-binary-xyz"),
            "podium-nonexistent-binary-xyz"
        );
        // Already a path — passed through untouched, no PATH search.
        assert_eq!(
            resolve_windows_executable(r"C:\already\a\path.exe"),
            r"C:\already\a\path.exe"
        );
    }

    #[test]
    fn git_binary_resolves_to_something_nonempty() {
        // Either a resolved absolute-ish path or the "git" fallback — either
        // way it must be usable directly as a `Command::new` program name.
        let path = git_binary();
        assert!(!path.is_empty());
        // Cached: a second call returns the identical pointer/value.
        assert_eq!(git_binary(), path);
    }

    #[test]
    fn warp_uri_percent_encodes_the_path() {
        assert_eq!(
            warp_uri(std::path::Path::new("/Users/me/My Project")),
            "warp://action/new_tab?path=/Users/me/My%20Project"
        );
    }
}
