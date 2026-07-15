//! Integration tests for the PTY engine, against real PTYs.

use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use nix::sys::signal::{killpg, Signal};
#[cfg(unix)]
use nix::unistd::Pid;
use podium_core::{
    Orchestrator, ProcessId, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy,
};
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

fn spec(command: &str, cwd: &Path) -> ProcessSpec {
    ProcessSpec {
        name: "test".to_string(),
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        kind: ProcessKind::Service,
        restart_policy: RestartPolicy::Never,
    }
}

async fn setup(command: &str) -> (Orchestrator, ProcessId, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let process_id = orch
        .add_process(project_id, spec(command, dir.path()))
        .await
        .expect("add process");
    (orch, process_id, dir)
}

fn status_of(orch: &Orchestrator, id: ProcessId) -> ProcessStatus {
    orch.list_processes(None)
        .into_iter()
        .find(|p| p.id == id)
        .expect("process listed")
        .status
}

async fn wait_for_exit(orch: &Orchestrator, id: ProcessId) -> ProcessStatus {
    timeout(TEST_TIMEOUT, async {
        loop {
            let status = status_of(orch, id);
            if matches!(status, ProcessStatus::Exited { .. }) {
                return status;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for process to exit")
}

async fn wait_for_running(orch: &Orchestrator, id: ProcessId) {
    timeout(TEST_TIMEOUT, async {
        loop {
            if matches!(status_of(orch, id), ProcessStatus::Running { .. }) {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for process to start");
}

async fn wait_for_output(orch: &Orchestrator, id: ProcessId, needle: &str) -> String {
    timeout(TEST_TIMEOUT, async {
        loop {
            let text = orch.tail_text(id, usize::MAX).await.expect("tail_text");
            if text.contains(needle) {
                return text;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for expected output")
}

#[tokio::test(flavor = "multi_thread")]
async fn printf_hello_exits_cleanly() {
    let (orch, id, _dir) = setup("printf hello; exit 0").await;
    let (snapshot, _next_seq, _rx) = orch.attach(id).await.expect("attach");
    assert!(snapshot.is_empty(), "no output before start");

    orch.start_process(id).await.expect("start");
    let status = wait_for_exit(&orch, id).await;
    let ProcessStatus::Exited { code, crashed, .. } = status else {
        panic!("expected Exited, got {status:?}");
    };
    assert_eq!(code, Some(0));
    assert!(!crashed);

    let text = wait_for_output(&orch, id, "hello").await;
    assert!(text.contains("hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn nonzero_exit_is_a_crash() {
    let (orch, id, _dir) = setup("exit 3").await;
    orch.start_process(id).await.expect("start");
    let status = wait_for_exit(&orch, id).await;
    let ProcessStatus::Exited { code, crashed, .. } = status else {
        panic!("expected Exited, got {status:?}");
    };
    assert_eq!(code, Some(3));
    assert!(crashed);
}

// Unix-only: asserts process-group teardown (killpg). Windows kills the
// single ConPTY child, so there is no group to poll.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn user_stop_is_not_a_crash_and_kills_the_group() {
    let (orch, id, _dir) = setup("sleep 30").await;
    orch.start_process(id).await.expect("start");

    let pid = match status_of(&orch, id) {
        ProcessStatus::Running { pid, .. } => pid,
        other => panic!("expected Running, got {other:?}"),
    };

    orch.stop_process(id).await.expect("stop");
    let status = wait_for_exit(&orch, id).await;
    let ProcessStatus::Exited { crashed, .. } = status else {
        panic!("expected Exited, got {status:?}");
    };
    assert!(!crashed, "user-stopped process must not count as crashed");

    // The whole process group must be gone shortly after.
    let pgid = Pid::from_raw(pid as i32);
    timeout(Duration::from_secs(5), async {
        while killpg(pgid, None::<Signal>).is_ok() {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("process group still alive after stop");
}

#[tokio::test(flavor = "multi_thread")]
async fn active_agents_or_terminals_gate_tracks_only_agents_and_terminals() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let add_started = |kind: ProcessKind| {
        let orch = &orch;
        let dir = &dir;
        async move {
            let mut s = spec("sleep 30", dir.path());
            s.kind = kind;
            let id = orch.add_process(project_id, s).await.expect("add process");
            orch.start_process(id).await.expect("start");
            wait_for_running(orch, id).await;
            id
        }
    };

    // A running service alone must NOT trip the gate.
    let svc_id = add_started(ProcessKind::Service).await;
    assert!(!orch.has_active_agents_or_terminals());

    // A running terminal does.
    let term_id = add_started(ProcessKind::Terminal).await;
    assert!(orch.has_active_agents_or_terminals());

    // And so does a running agent.
    let agent_id = add_started(ProcessKind::Agent {
        adapter: "claude-code".to_string(),
    })
    .await;
    assert!(orch.has_active_agents_or_terminals());

    // Stopping the terminal but not the agent keeps the gate closed.
    orch.stop_process(term_id).await.expect("stop terminal");
    wait_for_exit(&orch, term_id).await;
    assert!(orch.has_active_agents_or_terminals());

    // Once the agent is gone too, only the service remains → gate is clear.
    orch.stop_process(agent_id).await.expect("stop agent");
    wait_for_exit(&orch, agent_id).await;
    assert!(!orch.has_active_agents_or_terminals());

    orch.stop_process(svc_id).await.expect("stop service");
    wait_for_exit(&orch, svc_id).await;
    assert!(!orch.has_active_agents_or_terminals());
}

#[tokio::test(flavor = "multi_thread")]
async fn mid_stream_attach_reconstructs_output_without_gaps_or_dups() {
    let command = "i=0; while [ $i -lt 20 ]; do echo line$i; i=$((i+1)); sleep 0.05; done";
    let (orch, id, _dir) = setup(command).await;
    orch.start_process(id).await.expect("start");

    // Let some output accumulate, then attach mid-stream.
    wait_for_output(&orch, id, "line1\r").await;
    let (snapshot, next_seq, mut rx) = orch.attach(id).await.expect("attach");

    let mut combined = snapshot;
    let mut expected_seq = next_seq;
    let deadline = tokio::time::Instant::now() + TEST_TIMEOUT;
    loop {
        let exited = matches!(status_of(&orch, id), ProcessStatus::Exited { .. });
        match timeout(Duration::from_millis(300), rx.recv()).await {
            Ok(Ok(chunk)) => {
                assert_eq!(
                    chunk.seq, expected_seq,
                    "chunk seqs must be contiguous from the snapshot's next_seq"
                );
                expected_seq += 1;
                combined.extend_from_slice(&chunk.bytes);
            }
            Ok(Err(e)) => panic!("broadcast channel error: {e}"),
            Err(_) if exited => break, // drained after exit
            Err(_) => assert!(
                tokio::time::Instant::now() < deadline,
                "timed out collecting chunks"
            ),
        }
    }

    // snapshot + streamed chunks must equal the full retained scrollback.
    let (full, final_next_seq) = {
        let (bytes, seq, _rx) = orch.attach(id).await.expect("re-attach");
        (bytes, seq)
    };
    assert_eq!(expected_seq, final_next_seq);
    assert_eq!(combined, full, "reconstructed stream must match scrollback");

    let text = String::from_utf8_lossy(&combined);
    for i in 0..20 {
        let marker = format!("line{i}\r");
        let count = text.matches(&marker).count();
        assert_eq!(count, 1, "expected exactly one occurrence of line{i}");
    }
}

/// Regression test for the `-lic` (login *and* interactive) shell invocation:
/// PATH/env edits some installers (nvm, Homebrew shellenv, Docker Desktop
/// alternatives, …) only add to `.zshrc` — an interactive-only rc file that a
/// plain login shell (`-lc`) never reads. `zsh` is macOS's default shell, but
/// some environments (e.g. GitHub Actions macOS runners) still run with a
/// non-zsh `$SHELL`, so skip rather than fail when this zsh-specific
/// assertion doesn't apply.
#[tokio::test(flavor = "multi_thread")]
async fn zshrc_only_env_is_visible_to_spawned_processes() {
    if !std::env::var("SHELL").unwrap_or_default().contains("zsh") {
        eprintln!("skipping: this test asserts zsh-specific rc behaviour; host $SHELL is not zsh");
        return;
    }

    let fake_home = tempfile::tempdir().expect("tempdir for fake $HOME");
    std::fs::write(
        fake_home.path().join(".zshrc"),
        "export PODIUM_INTERACTIVE_TEST_MARKER=zshrc_was_sourced\n",
    )
    .expect("write .zshrc");

    let dir = tempfile::tempdir().expect("tempdir for project");
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let process_id = orch
        .add_process(
            project_id,
            ProcessSpec {
                name: "test".to_string(),
                command: "echo $PODIUM_INTERACTIVE_TEST_MARKER".to_string(),
                cwd: dir.path().to_path_buf(),
                env: vec![(
                    "HOME".to_string(),
                    fake_home.path().to_string_lossy().into_owned(),
                )],
                kind: ProcessKind::Service,
                restart_policy: RestartPolicy::Never,
            },
        )
        .await
        .expect("add process");

    orch.start_process(process_id).await.expect("start");
    let text = wait_for_output(&orch, process_id, "zshrc_was_sourced").await;
    assert!(text.contains("zshrc_was_sourced"));
}
