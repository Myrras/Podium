//! Claude Code's own record of what a session is doing.
//!
//! Stub: [`read`] reports nothing yet.

use std::path::Path;
use std::time::SystemTime;

use super::ReportedState;

/// What identifies one agent's Claude Code session on disk.
#[derive(Debug, Clone, Copy)]
pub struct Probe<'a> {
    /// Claude Code's config dir (`$CLAUDE_CONFIG_DIR`, else `~/.claude`).
    pub config_dir: &'a Path,
    /// The `podium-<tag>` name Podium stamps on the session
    /// ([`crate::agent::peer_session_name`]).
    pub session_name: &'a str,
    /// The agent's PTY child, while it has one.
    pub pty_pid: Option<u32>,
    /// The directory the agent runs in.
    pub cwd: &'a Path,
    /// When Podium started this run (wall clock).
    pub started: SystemTime,
}

/// What Claude Code says the session is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeStatus {
    pub state: ReportedState,
    /// Only a background shell is running.
    pub shell: bool,
    /// What a waiting session is waiting for.
    pub waiting_for: Option<String>,
    /// When `state` began, when Claude Code records it.
    pub since: Option<SystemTime>,
}

/// The status of the session `probe` describes, or `None` when Claude Code
/// has recorded none. Reads the disk: call it outside any lock.
pub fn read(_probe: &Probe) -> Option<ClaudeStatus> {
    None
}
