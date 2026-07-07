//! PTY-backed process: spawn, stream output, write stdin, resize, stop.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

use crate::error::{CoreError, CoreResult};
use crate::process::scrollback::ScrollbackBuffer;
use crate::process::ProcessSpec;

const LOCK_POISONED: &str = "pty lock poisoned";
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
    pid: u32,
}

impl PtyProcess {
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

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-lc");
        cmd.arg(&spec.command);
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

    /// Gracefully stop the whole process group: SIGTERM now, then SIGKILL
    /// after a grace period if the group is still alive.
    ///
    /// portable-pty spawns the child as a session leader, so pgid == pid.
    /// Must be called from within a tokio runtime (the SIGKILL escalation
    /// runs on a spawned task).
    pub fn stop(&self) {
        let pgid = Pid::from_raw(self.pid as i32);
        let _ = killpg(pgid, Signal::SIGTERM);
        tokio::spawn(async move {
            tokio::time::sleep(STOP_GRACE).await;
            if killpg(pgid, None::<Signal>).is_ok() {
                let _ = killpg(pgid, Signal::SIGKILL);
            }
        });
    }
}
