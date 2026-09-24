//! The small OS gap Podium has to bridge: wrapping a command *string* in a
//! login shell, asking that shell whether a binary is on `PATH`, and quoting
//! a single argument for inclusion in that command string. Every other
//! platform difference is handled by portable-pty (ConPTY on Windows).
//!
//! Keeping these helpers in one place means the shell idiom (`$SHELL -lc` vs
//! `cmd /C`) is written once, not re-derived at each of the PTY-spawn /
//! adapter-probe / MCP-install call sites.

pub mod wsl_path;

use std::sync::OnceLock;

use crate::error::{CoreError, CoreResult};

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
}

impl WslBackend {
    /// The WSL distro name, as `wsl.exe -d` takes it (e.g. `Ubuntu`).
    pub fn distro(&self) -> &str {
        &self.distro
    }
}

static WSL: OnceLock<Option<WslBackend>> = OnceLock::new();

/// Decide the backend from the configured distro. Pure, so the decision is
/// testable on every platform: WSL mode needs Windows and a non-blank distro.
fn wsl_backend_for(distro: Option<&str>, windows: bool) -> Option<WslBackend> {
    let distro = distro.map(str::trim).filter(|d| !d.is_empty())?;
    windows.then(|| WslBackend {
        distro: distro.to_string(),
    })
}

/// Fix the process-wide backend for this run. Call once, at startup, before
/// anything spawns (the MCP server, the orchestrator reopening projects).
/// Off Windows a configured distro is ignored (a settings file synced from a
/// Windows machine must not change macOS behaviour). Later calls are no-ops.
pub fn init_wsl(distro: Option<&str>) {
    let backend = wsl_backend_for(distro, cfg!(windows));
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

/// The login shell plus the args that precede a command *string*, so that
/// `Command::new(program).args(prefix).arg(cmd)` runs `cmd` through it.
///
/// Unix: `$SHELL -lic <cmd>` — login *and* interactive, so both
/// `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc` PATH/env edits (nvm,
/// Homebrew shellenv, Docker Desktop alternatives, …) apply — some installers
/// only add to the interactive-only file. Windows: `%COMSPEC% /C <cmd>`
/// (defaults to `cmd.exe`).
pub fn login_shell() -> (String, &'static [&'static str]) {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, &["-lic"])
    }
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        (shell, &["/C"])
    }
}

/// A shell command line that exits 0 iff `binary` resolves on `PATH`.
/// Unix uses `command -v`; Windows uses `where`.
pub fn command_exists_query(binary: &str) -> String {
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
/// [`login_shell`]: POSIX single-quote quoting for `$SHELL -lc` on Unix,
/// Windows argv quoting (the C-runtime backslash/quote escaping rules that
/// `CommandLineToArgvW` reverses) for `cmd.exe /C` on Windows.
///
/// The two are not interchangeable: `cmd.exe` has no concept of POSIX
/// single-quote grouping, so a POSIX-quoted argument containing a `'` (e.g.
/// `You're …`) is not treated as one quoted token — `cmd.exe` still splits it
/// on whitespace, truncating the argument at the first space.
pub fn quote_arg(arg: &str) -> CoreResult<String> {
    #[cfg(unix)]
    {
        shlex::try_quote(arg)
            .map(std::borrow::Cow::into_owned)
            .map_err(|e| CoreError::InvalidInput(format!("cannot quote argument: {e}")))
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
    let (shell, prefix) = login_shell();
    std::process::Command::new(shell)
        .args(prefix)
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Run `cmd` through the login shell, discarding stderr; returns its stdout
/// when the command exits successfully, otherwise `None`. Used only where a
/// command's non-secret stdout must be read (e.g. the Auggie installed-check's
/// JSON listing), not just its exit status.
pub fn run_shell_stdout(cmd: &str) -> Option<String> {
    let (shell, prefix) = login_shell();
    let output = std::process::Command::new(shell)
        .args(prefix)
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The `git` binary to invoke directly (bypassing the login shell) for
/// worktree operations, which can spawn several git calls per action
/// (list/create/remove) — a login shell per call is slow on heavy shell
/// profiles. Resolved once via the login shell's `command -v git` so
/// shell-profile PATH edits (e.g. a Homebrew git) are still honoured, then
/// reused directly; falls back to the bare name if resolution fails.
/// Windows PATH is process-wide, so no shell round-trip is needed there.
pub fn git_binary() -> &'static str {
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
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", &uri])
        .status();
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
    fn wsl_backend_needs_windows_and_a_distro() {
        assert_eq!(
            wsl_backend_for(Some("Ubuntu"), true),
            Some(WslBackend {
                distro: "Ubuntu".to_string()
            })
        );
        assert_eq!(
            wsl_backend_for(Some("  Ubuntu-24.04 "), true).map(|w| w.distro().to_string()),
            Some("Ubuntu-24.04".to_string())
        );
        assert_eq!(wsl_backend_for(None, true), None);
        assert_eq!(wsl_backend_for(Some("   "), true), None);
        // Off Windows a configured distro is ignored.
        assert_eq!(wsl_backend_for(Some("Ubuntu"), false), None);
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
