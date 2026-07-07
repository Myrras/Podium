//! Registering Podium's stdio MCP bridge with external agent CLIs.
//!
//! Today that means Claude Code: `claude mcp add --scope user --transport
//! stdio podium -- <podium-exe> mcp-bridge`. Everything shells out via the
//! user's login shell (like adapter availability probing) so shell-profile
//! PATH edits (nvm, homebrew, …) are honoured. Output is always discarded —
//! only exit statuses are inspected — so nothing the CLI prints can leak
//! into logs or errors.

use std::borrow::Cow;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{CoreError, CoreResult};

/// The server name Podium registers under in external clients.
pub const SERVER_NAME: &str = "podium";

/// Run `cmd` via the login shell, discarding all output. Returns whether it
/// exited successfully.
fn login_shell_ok(cmd: &str) -> bool {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    Command::new(shell)
        .arg("-lc")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn quote(arg: &str) -> CoreResult<String> {
    shlex::try_quote(arg)
        .map(Cow::into_owned)
        .map_err(|e| CoreError::InvalidInput(format!("cannot quote argument: {e}")))
}

/// Registration state of Podium's bridge in the Claude Code CLI.
#[derive(Debug, Clone, Copy)]
pub struct ClaudeClientStatus {
    /// Whether the `claude` binary resolves on the login-shell PATH.
    pub cli_available: bool,
    /// Whether a `podium` MCP server entry exists (`claude mcp get podium`).
    pub installed: bool,
}

/// Probe the Claude Code CLI: is it on PATH, and is `podium` registered?
pub fn claude_status() -> ClaudeClientStatus {
    let cli_available = login_shell_ok("command -v claude");
    let installed = cli_available && login_shell_ok(&format!("claude mcp get {SERVER_NAME}"));
    ClaudeClientStatus {
        cli_available,
        installed,
    }
}

/// The `claude mcp add …` command line that registers the bridge, exactly as
/// [`claude_install`] runs it (also shown in the UI for manual use).
pub fn claude_add_command(exe: &Path) -> CoreResult<String> {
    let exe = quote(&exe.to_string_lossy())?;
    Ok(format!(
        "claude mcp add --scope user --transport stdio {SERVER_NAME} -- {exe} mcp-bridge"
    ))
}

/// Register the bridge with Claude Code, replacing any existing `podium`
/// entry first (so re-runs and moved binaries just work).
pub fn claude_install(exe: &Path) -> CoreResult<()> {
    if !login_shell_ok("command -v claude") {
        return Err(CoreError::Config(
            "Claude Code CLI (`claude`) not found on PATH".to_string(),
        ));
    }
    // Best-effort: fails harmlessly when no entry exists yet.
    login_shell_ok(&format!("claude mcp remove --scope user {SERVER_NAME}"));
    if !login_shell_ok(&claude_add_command(exe)?) {
        return Err(CoreError::Config(
            "`claude mcp add` failed to register Podium".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn add_command_is_plain_for_simple_paths() {
        let cmd = claude_add_command(&PathBuf::from("/usr/local/bin/podium")).unwrap();
        assert_eq!(
            cmd,
            "claude mcp add --scope user --transport stdio podium -- /usr/local/bin/podium mcp-bridge"
        );
    }

    #[test]
    fn add_command_quotes_paths_with_spaces() {
        let cmd =
            claude_add_command(&PathBuf::from("/Applications/My Podium.app/MacOS/Podium")).unwrap();
        let tokens = shlex::split(&cmd).expect("valid shell line");
        assert!(tokens.contains(&"/Applications/My Podium.app/MacOS/Podium".to_string()));
        assert_eq!(tokens.last().unwrap(), "mcp-bridge");
    }
}
