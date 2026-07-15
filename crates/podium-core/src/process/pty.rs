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
    writer: Mutex<Box<dyn Write + Send>>,
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
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| CoreError::Pty(e.to_string()))?;

        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let bytes = buf[..n].to_vec();
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
            writer: Mutex::new(writer),
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
