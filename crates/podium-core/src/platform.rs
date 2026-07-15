//! The small OS gap Podium has to bridge: wrapping a command *string* in a
//! login shell, and asking that shell whether a binary is on `PATH`. Every
//! other platform difference is handled by portable-pty (ConPTY on Windows).
//!
//! Keeping these three helpers in one place means the shell idiom
//! (`$SHELL -lc` vs `cmd /C`) is written once, not re-derived at each of the
//! PTY-spawn / adapter-probe / MCP-install call sites.

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
}
