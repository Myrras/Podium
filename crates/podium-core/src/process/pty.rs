//! PTY-backed process: spawn, stream output, write stdin, resize, stop.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

use crate::error::{CoreError, CoreResult};
use crate::process::scrollback::ScrollbackBuffer;
#[cfg(windows)]
use crate::process::ProcessKind;
use crate::process::ProcessSpec;

const LOCK_POISONED: &str = "pty lock poisoned";
#[cfg(unix)]
const STOP_GRACE: Duration = Duration::from_secs(5);

/// A chunk of raw PTY output tagged with its scrollback sequence number.
#[derive(Debug, Clone)]
pub struct TermChunk {
    pub seq: u64,
    pub bytes: Vec<u8>,
}

/// Callback invoked (from a dedicated wait-thread) when the child exits.
pub type ExitCallback = Box<dyn FnOnce(Option<i32>) + Send>;

/// A live process attached to a PTY.
///
/// Output is pumped on a blocking reader thread: every chunk is appended to
/// the shared [`ScrollbackBuffer`] and broadcast as a [`TermChunk`] *under
/// the same lock*, so sequence numbers observed by subscribers are always
/// consistent with the scrollback contents.
pub struct PtyProcess {
    master: Mutex<Box<dyn MasterPty + Send>>,
    // Shared with the reader thread on Windows, which answers ConPTY's
    // startup cursor-position query through it (see `CursorHandshake`).
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    // Windows stop() terminates the child through this handle (no process
    // groups); Unix stops the whole group via killpg and never touches it.
    #[cfg(windows)]
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    pid: u32,
}

impl PtyProcess {
    /// Builds the [`CommandBuilder`] for `spec`, deciding *how* to hand its
    /// `command` string to the OS.
    ///
    /// Agent commands (`ProcessKind::Agent`) are always `binary arg1 arg2 …`
    /// with no shell syntax — [`crate::agent::CliAdapter::build_launch`]
    /// builds that string by shell-quoting each argument. On Unix that quoted
    /// string is handed to `$SHELL -lc` as one opaque argument and parsed
    /// once, correctly, by a real POSIX shell. On Windows the analogous
    /// `cmd.exe /C <string>` does not work: portable-pty must itself flatten
    /// our `.arg()` calls into one `CreateProcessW` command line, so passing
    /// our already-quoted string as a single argument makes portable-pty
    /// quote it *again* — but `cmd.exe`'s own `/C` parsing has no notion of
    /// the backslash-escaping that second layer relies on, so it re-splits
    /// the string on whitespace, corrupting any argument that needed
    /// quoting (e.g. truncating a prompt at the first space or quote).
    ///
    /// So on Windows, agent commands bypass `cmd.exe` entirely: the string
    /// is re-tokenized with [`crate::platform::parse_windows_argv`] (the
    /// exact inverse of the quoting `build_launch` applied) and the binary is
    /// spawned directly, one argument per `.arg()` call, so portable-pty's
    /// own quoting is the *only* layer in play. Services and terminals are
    /// unaffected — their `command` is arbitrary shell syntax from
    /// `podium.yml` or the user's chosen shell, which still needs a real
    /// shell (`cmd.exe` on Windows) to interpret.
    #[cfg(windows)]
    fn build_command(spec: &ProcessSpec) -> CoreResult<CommandBuilder> {
        if matches!(spec.kind, ProcessKind::Agent { .. }) {
            let mut tokens = crate::platform::parse_windows_argv(&spec.command).into_iter();
            let program = tokens
                .next()
                .ok_or_else(|| CoreError::Pty("empty agent command".to_string()))?;
            // Resolve to an absolute path ourselves: portable-pty's own
            // Windows PATH search accepts the first same-named *directory*
            // it finds as readily as a real executable, which makes
            // `CreateProcessW` fail with `ERROR_ACCESS_DENIED` when a CLI's
            // own data folder (e.g. Claude Code's `%LOCALAPPDATA%\claude\`)
            // sits earlier in `PATH` than its actual binary. See
            // `platform::resolve_windows_executable`.
            let program = crate::platform::resolve_windows_executable(&program);
            let mut cmd = CommandBuilder::new(program);
            for arg in tokens {
                cmd.arg(arg);
            }
            return Ok(cmd);
        }
        let (shell, prefix) = crate::platform::login_shell();
        let mut cmd = CommandBuilder::new(shell);
        for arg in prefix {
            cmd.arg(arg);
        }
        cmd.arg(&spec.command);
        Ok(cmd)
    }

    #[cfg(unix)]
    fn build_command(spec: &ProcessSpec) -> CoreResult<CommandBuilder> {
        let (shell, prefix) = crate::platform::login_shell();
        let mut cmd = CommandBuilder::new(shell);
        for arg in prefix {
            cmd.arg(arg);
        }
        cmd.arg(&spec.command);
        Ok(cmd)
    }

    pub fn spawn(
        spec: &ProcessSpec,
        size: Option<(u16, u16)>,
        scrollback: Arc<Mutex<ScrollbackBuffer>>,
        chunk_tx: broadcast::Sender<TermChunk>,
        on_exit: ExitCallback,
    ) -> CoreResult<Self> {
        let (cols, rows) = size.unwrap_or((80, 24));
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| CoreError::Pty(e.to_string()))?;

        let mut cmd = Self::build_command(spec)?;
        cmd.cwd(&spec.cwd);
        for (key, value) in &spec.env {
            cmd.env(key, value);
        }
        cmd.env("TERM", "xterm-256color");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| CoreError::Pty(e.to_string()))?;
        // Drop our slave handle so the reader sees EOF once the child exits.
        drop(pair.slave);

        let pid = child
            .process_id()
            .ok_or_else(|| CoreError::Pty("spawned child has no pid".to_string()))?;

        // Split off a killer handle before the child moves into the wait
        // thread; Windows stop() uses it (Unix signals the process group).
        #[cfg(windows)]
        let killer = child.clone_killer();

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| CoreError::Pty(e.to_string()))?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| CoreError::Pty(e.to_string()))?,
        ));
        #[cfg(windows)]
        let reply_writer = Arc::clone(&writer);

        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            #[cfg(windows)]
            let mut handshake = CursorHandshake::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        #[allow(unused_mut)] // only mutated on Windows
                        let mut bytes = buf[..n].to_vec();
                        #[cfg(windows)]
                        if handshake.strip(&mut bytes) {
                            let mut w = reply_writer.lock().expect(LOCK_POISONED);
                            let _ = w.write_all(CURSOR_POSITION_REPLY);
                            let _ = w.flush();
                        }
                        if bytes.is_empty() {
                            continue;
                        }
                        // Append + broadcast under one lock: seq order stays
                        // consistent for attach() snapshots.
                        let mut sb = scrollback.lock().expect(LOCK_POISONED);
                        let seq = sb.append(&bytes);
                        let _ = chunk_tx.send(TermChunk { seq, bytes });
                    }
                }
            }
        });

        std::thread::spawn(move || {
            let code = match child.wait() {
                // Signal-terminated children have no meaningful exit code.
                Ok(status) if status.signal().is_some() => None,
                Ok(status) => Some(status.exit_code() as i32),
                Err(_) => None,
            };
            on_exit(code);
        });

        Ok(Self {
            master: Mutex::new(pair.master),
            writer,
            #[cfg(windows)]
            killer: Mutex::new(killer),
            pid,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn write(&self, bytes: &[u8]) -> CoreResult<()> {
        let mut writer = self.writer.lock().expect(LOCK_POISONED);
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> CoreResult<()> {
        self.master
            .lock()
            .expect(LOCK_POISONED)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| CoreError::Pty(e.to_string()))
    }

    /// Gracefully stop the process.
    ///
    /// Unix: SIGTERM the whole process group now, then SIGKILL it after a
    /// grace period if it is still alive (portable-pty spawns the child as a
    /// session leader, so pgid == pid). Must be called from within a tokio
    /// runtime — the SIGKILL escalation runs on a spawned task.
    #[cfg(unix)]
    pub fn stop(&self) {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;

        let pgid = Pid::from_raw(self.pid as i32);
        let _ = killpg(pgid, Signal::SIGTERM);
        tokio::spawn(async move {
            tokio::time::sleep(STOP_GRACE).await;
            if killpg(pgid, None::<Signal>).is_ok() {
                let _ = killpg(pgid, Signal::SIGKILL);
            }
        });
    }

    /// Windows: terminate the ConPTY child via its killer handle. Windows has
    /// no process groups, so this is an immediate TerminateProcess; any
    /// grandchildren are reaped when the pseudoconsole tears down as the
    /// master handle drops.
    // ponytail: single-process kill. If orphaned grandchildren become a
    // problem, spawn the child into a Win32 Job Object and kill that instead.
    #[cfg(windows)]
    pub fn stop(&self) {
        let _ = self.killer.lock().expect(LOCK_POISONED).kill();
    }
}

/// ConPTY's startup cursor-position query (DSR 6).
#[cfg(any(windows, test))]
const CURSOR_POSITION_QUERY: &[u8] = b"[6n";
/// Reply for a fresh pseudoconsole: cursor at row 1, column 1.
#[cfg(windows)]
const CURSOR_POSITION_REPLY: &[u8] = b"[1;1R";
/// How far into a spawn's output the startup query may appear. Past this,
/// a query is the program's own and is left for the terminal to answer.
#[cfg(any(windows, test))]
const HANDSHAKE_WINDOW: usize = 256;

/// Answers ConPTY's startup handshake on the backend's behalf.
///
/// portable-pty opens the pseudoconsole with `PSEUDOCONSOLE_INHERIT_CURSOR`,
/// so ConPTY emits `ESC[6n` first and holds the child until it gets a
/// cursor-position reply. The frontend's xterm.js would answer, but only once
/// someone opens that terminal — until then an `auto_start` service, a
/// background agent or a supervised restart would sit blocked. So the reader
/// thread answers the first query itself and strips it from the output (so
/// xterm doesn't answer it a second time when it replays the scrollback).
/// Unix PTYs never send it.
#[cfg(any(windows, test))]
struct CursorHandshake {
    seen: usize,
    done: bool,
}

#[cfg(any(windows, test))]
impl CursorHandshake {
    fn new() -> Self {
        Self {
            seen: 0,
            done: false,
        }
    }

    /// Removes the startup query from `bytes` if this chunk carries it;
    /// returns whether it did (the caller then sends the reply).
    fn strip(&mut self, bytes: &mut Vec<u8>) -> bool {
        if self.done {
            return false;
        }
        let window = HANDSHAKE_WINDOW.saturating_sub(self.seen);
        let found = bytes
            .windows(CURSOR_POSITION_QUERY.len())
            .position(|w| w == CURSOR_POSITION_QUERY)
            .filter(|&at| at < window);
        self.seen += bytes.len();
        match found {
            Some(at) => {
                bytes.drain(at..at + CURSOR_POSITION_QUERY.len());
                self.done = true;
                true
            }
            None => {
                self.done = self.seen >= HANDSHAKE_WINDOW;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_query_is_stripped_once() {
        let mut hs = CursorHandshake::new();
        let mut first = b"[?9001h[6n".to_vec();
        assert!(hs.strip(&mut first));
        assert_eq!(first, b"[?9001h");

        // A later query is the program's own: left for the terminal.
        let mut later = b"[6n".to_vec();
        assert!(!hs.strip(&mut later));
        assert_eq!(later, b"[6n");
    }

    #[test]
    fn query_in_a_later_chunk_within_the_window_is_stripped() {
        let mut hs = CursorHandshake::new();
        let mut init = b"[?25l".to_vec();
        assert!(!hs.strip(&mut init));
        let mut chunk = b"[6n[?25h".to_vec();
        assert!(hs.strip(&mut chunk));
        assert_eq!(chunk, b"[?25h");
    }

    #[test]
    fn query_past_the_window_is_left_alone() {
        let mut hs = CursorHandshake::new();
        let mut output = vec![b'x'; HANDSHAKE_WINDOW];
        assert!(!hs.strip(&mut output));
        let mut query = CURSOR_POSITION_QUERY.to_vec();
        assert!(!hs.strip(&mut query));
        assert_eq!(query, CURSOR_POSITION_QUERY);
    }
}
