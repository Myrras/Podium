//! Whether something runs in a terminal's foreground.
//!
//! Stub: [`busy`] knows nothing yet.

use crate::process::pty::PtyProcess;

/// `Some(true)` while a command runs in the foreground of `pty`,
/// `Some(false)` at its shell prompt, `None` when it cannot be told. Asks the
/// PTY: call it outside any lock.
pub fn busy(_pty: &PtyProcess) -> Option<bool> {
    None
}
