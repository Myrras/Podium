//! Whether something runs in a terminal's foreground.
//!
//! Unix only: an interactive shell leads its own process group and, with job
//! control, gives each command it runs a group of its own. So the PTY's
//! foreground group is the shell's pid at the prompt and another while a
//! command runs.

use crate::process::pty::PtyProcess;

/// `Some(true)` while a command runs in the foreground of `pty`,
/// `Some(false)` at its shell prompt, `None` when it cannot be told: Windows
/// (no process groups) or a terminal started with its own command instead of
/// an interactive shell. Asks the PTY: call it outside any lock.
#[cfg(unix)]
pub fn busy(pty: &PtyProcess) -> Option<bool> {
    if !pty.is_interactive_shell() {
        return None;
    }
    pty.foreground_group()
        .map(|group| i64::from(group) != i64::from(pty.pid()))
}

#[cfg(not(unix))]
pub fn busy(_pty: &PtyProcess) -> Option<bool> {
    None
}
