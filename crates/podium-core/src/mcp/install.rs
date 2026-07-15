//! Registering Podium's stdio MCP bridge with external agent CLIs.
//!
//! Two clients are supported today:
//! - **Claude Code**: `claude mcp add --scope user --transport stdio podium
//!   -- <podium-exe> mcp-bridge`.
//! - **Auggie**: `auggie mcp add podium --command <podium-exe> --args
//!   mcp-bridge --replace`.
//!
//! Everything shells out via the user's login shell (like adapter
//! availability probing) so shell-profile PATH edits (nvm, homebrew, …) are
//! honoured. Command output is discarded and only exit statuses inspected,
//! with one exception: Auggie has no `get <name>` subcommand, so its
//! installed-check reads `auggie mcp list --json` on stdout to find the
//! `podium` entry. That listing never carries the bearer token (the bridge
//! reads it from `server.json`) and is inspected in memory, never logged.

use std::borrow::Cow;
use std::path::Path;

use serde::Deserialize;

use crate::error::{CoreError, CoreResult};
use crate::platform::{run_shell_ok as login_shell_ok, run_shell_stdout as login_shell_stdout};

/// The server name Podium registers under in external clients.
pub const SERVER_NAME: &str = "podium";

fn quote(arg: &str) -> CoreResult<String> {
    shlex::try_quote(arg)
        .map(Cow::into_owned)
        .map_err(|e| CoreError::InvalidInput(format!("cannot quote argument: {e}")))
}

/// Registration state of Podium's bridge in an external client's CLI.
#[derive(Debug, Clone, Copy)]
pub struct ClientStatus {
    /// Whether the client's CLI binary resolves on the login-shell PATH.
    pub cli_available: bool,
    /// Whether a `podium` MCP server entry is registered.
    pub installed: bool,
}

/// Probe the Claude Code CLI: is it on PATH, and is `podium` registered?
pub fn claude_status() -> ClientStatus {
    let cli_available = login_shell_ok(&crate::platform::command_exists_query("claude"));
    let installed = cli_available && login_shell_ok(&format!("claude mcp get {SERVER_NAME}"));
    ClientStatus {
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
    if !login_shell_ok(&crate::platform::command_exists_query("claude")) {
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

/// The `auggie mcp add …` command line that registers the bridge, exactly as
/// [`auggie_install`] runs it (also shown in the UI for manual use).
pub fn auggie_add_command(exe: &Path) -> CoreResult<String> {
    let exe = quote(&exe.to_string_lossy())?;
    Ok(format!(
        "auggie mcp add {SERVER_NAME} --command {exe} --args mcp-bridge --replace"
    ))
}

/// Whether a `podium` entry appears in `auggie mcp list --json`. Auggie has no
/// `get <name>` subcommand, so we parse the JSON listing (`{"servers":[…]}`)
/// off stdout and match by name — the listing carries no secrets.
fn auggie_installed() -> bool {
    #[derive(Deserialize)]
    struct Entry {
        name: String,
    }
    #[derive(Deserialize)]
    struct Listing {
        #[serde(default)]
        servers: Vec<Entry>,
    }
    login_shell_stdout("auggie mcp list --json")
        .and_then(|out| serde_json::from_str::<Listing>(&out).ok())
        .map(|listing| listing.servers.iter().any(|s| s.name == SERVER_NAME))
        .unwrap_or(false)
}

/// Probe the Auggie CLI: is it on PATH, and is `podium` registered?
pub fn auggie_status() -> ClientStatus {
    let cli_available = login_shell_ok(&crate::platform::command_exists_query("auggie"));
    let installed = cli_available && auggie_installed();
    ClientStatus {
        cli_available,
        installed,
    }
}

/// Register the bridge with Auggie. `--replace` overwrites any existing
/// `podium` entry, so re-runs and moved binaries just work.
pub fn auggie_install(exe: &Path) -> CoreResult<()> {
    if !login_shell_ok(&crate::platform::command_exists_query("auggie")) {
        return Err(CoreError::Config(
            "Auggie CLI (`auggie`) not found on PATH".to_string(),
        ));
    }
    if !login_shell_ok(&auggie_add_command(exe)?) {
        return Err(CoreError::Config(
            "`auggie mcp add` failed to register Podium".to_string(),
        ));
    }
    Ok(())
}

/// One external MCP client Podium can register its stdio bridge with. The
/// per-client behaviour (status probe, add command, install) is dispatched by
/// [`id`](Self::id) below, so this is the single source of truth for which
/// clients Podium supports.
#[derive(Debug, Clone, Copy)]
pub struct McpClient {
    /// Stable identifier surfaced to the UI (e.g. `"claude-code"`).
    pub id: &'static str,
    /// Human-readable name for the settings card.
    pub display_name: &'static str,
}

/// The Claude Code client.
pub const CLAUDE_CODE: McpClient = McpClient {
    id: "claude-code",
    display_name: "Claude Code",
};

/// The Auggie client.
pub const AUGGIE: McpClient = McpClient {
    id: "auggie",
    display_name: "Auggie",
};

/// Every external client Podium can register with, in display order.
pub static CLIENTS: &[McpClient] = &[CLAUDE_CODE, AUGGIE];

/// Look up a supported client by its stable id.
pub fn client(id: &str) -> Option<McpClient> {
    CLIENTS.iter().copied().find(|c| c.id == id)
}

impl McpClient {
    /// Current PATH + registration state for this client.
    pub fn status(&self) -> ClientStatus {
        match self.id {
            "claude-code" => claude_status(),
            "auggie" => auggie_status(),
            _ => ClientStatus {
                cli_available: false,
                installed: false,
            },
        }
    }

    /// The registration command line (shown in the UI, run by [`Self::install`]).
    pub fn add_command(&self, exe: &Path) -> CoreResult<String> {
        match self.id {
            "claude-code" => claude_add_command(exe),
            "auggie" => auggie_add_command(exe),
            _ => Err(CoreError::InvalidInput("unknown MCP client".to_string())),
        }
    }

    /// Register Podium's bridge with this client.
    pub fn install(&self, exe: &Path) -> CoreResult<()> {
        match self.id {
            "claude-code" => claude_install(exe),
            "auggie" => auggie_install(exe),
            _ => Err(CoreError::InvalidInput("unknown MCP client".to_string())),
        }
    }

    /// The CLI command that lists registered servers, shown in the card hint.
    pub fn check_command(&self) -> &'static str {
        match self.id {
            "auggie" => "auggie mcp list",
            _ => "claude mcp list",
        }
    }
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

    #[test]
    fn auggie_add_command_is_plain_for_simple_paths() {
        let cmd = auggie_add_command(&PathBuf::from("/usr/local/bin/podium")).unwrap();
        assert_eq!(
            cmd,
            "auggie mcp add podium --command /usr/local/bin/podium --args mcp-bridge --replace"
        );
    }

    #[test]
    fn auggie_add_command_quotes_paths_with_spaces() {
        let cmd =
            auggie_add_command(&PathBuf::from("/Applications/My Podium.app/MacOS/Podium")).unwrap();
        let tokens = shlex::split(&cmd).expect("valid shell line");
        assert!(tokens.contains(&"/Applications/My Podium.app/MacOS/Podium".to_string()));
        assert!(tokens.contains(&"mcp-bridge".to_string()));
    }

    #[test]
    fn clients_registry_lists_both_and_lookup_works() {
        assert_eq!(CLIENTS.len(), 2);
        assert_eq!(client("claude-code").unwrap().display_name, "Claude Code");
        assert_eq!(client("auggie").unwrap().display_name, "Auggie");
        assert_eq!(client("auggie").unwrap().check_command(), "auggie mcp list");
        assert!(client("nope").is_none());
    }
}
