//! The small OS gap Podium has to bridge: wrapping a command *string* in a
//! login shell, asking that shell whether a binary is on `PATH`, and quoting
//! a single argument for inclusion in that command string. Every other
//! platform difference is handled by portable-pty (ConPTY on Windows).
//!
//! Keeping these helpers in one place means the shell idiom (`$SHELL -lc` vs
//! `cmd /C`) is written once, not re-derived at each of the PTY-spawn /
//! adapter-probe / MCP-install call sites.

use crate::error::{CoreError, CoreResult};

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
}
