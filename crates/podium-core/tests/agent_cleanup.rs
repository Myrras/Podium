//! Integration tests for the exited-agent cleanup sweep, on real PTYs with
//! millisecond "minutes".

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use podium_core::agent::cleanup;
use podium_core::{
    AgentCleanupConfig, Orchestrator, PodiumEvent, ProcessId, ProcessKind, ProcessSpec,
    ProcessStatus, ProjectId, RestartPolicy,
};
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

fn spec(kind: ProcessKind, command: &str, cwd: &Path) -> ProcessSpec {
    ProcessSpec {
        name: "fix-login".to_string(),
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        kind,
        restart_policy: RestartPolicy::Never,
        color: None,
    }
}

fn agent() -> ProcessKind {
    ProcessKind::Agent {
        adapter: "claude-code".to_string(),
    }
}

/// The sweep with one "minute" worth `minute`, ticking ten times as often.
fn start_sweep(orch: &Arc<Orchestrator>, minute: Duration) {
    cleanup::spawn(
        orch,
        AgentCleanupConfig {
            tick: minute / 10,
            minute,
        },
    );
}

async fn open(orch: &Orchestrator, dir: &Path) -> ProjectId {
    orch.open_project(dir.to_path_buf()).await.unwrap()
}

fn status_of(orch: &Orchestrator, id: ProcessId) -> Option<ProcessStatus> {
    orch.list_processes(None)
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.status)
}

async fn wait_for(
    orch: &Orchestrator,
    id: ProcessId,
    done: impl Fn(Option<ProcessStatus>) -> bool,
) {
    timeout(TEST_TIMEOUT, async {
        while !done(status_of(orch, id)) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("process never reached the expected state");
}

fn exited(status: Option<ProcessStatus>) -> bool {
    matches!(status, Some(ProcessStatus::Exited { .. }))
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exited_agent_goes_after_the_window_and_terminals_and_services_stay() {
    let dir = tempfile::tempdir().unwrap();
    let orch = Arc::new(Orchestrator::new());
    // 3 "minutes" of 100 ms each.
    orch.set_agent_remove_exited_after(3).unwrap();
    let project = open(&orch, dir.path()).await;
    let agent_id = orch
        .add_process(project, spec(agent(), "exit 0", dir.path()))
        .await
        .unwrap();
    let terminal = orch
        .add_process(project, spec(ProcessKind::Terminal, "exit 0", dir.path()))
        .await
        .unwrap();
    let service = orch
        .add_process(project, spec(ProcessKind::Service, "exit 0", dir.path()))
        .await
        .unwrap();
    let mut rx = orch.subscribe_events();
    start_sweep(&orch, Duration::from_millis(100));
    for id in [agent_id, terminal, service] {
        orch.start_process(id).await.unwrap();
    }
    wait_for(&orch, agent_id, exited).await;
    // Not before the window is up.
    assert!(
        exited(status_of(&orch, agent_id)),
        "agent removed right at its exit"
    );

    wait_for(&orch, agent_id, |s| s.is_none()).await;
    // Well past the window, the terminal and the service are still there.
    sleep(Duration::from_millis(500)).await;
    assert!(exited(status_of(&orch, terminal)));
    assert!(exited(status_of(&orch, service)));

    // The exit — what the assistant's exit nudge reacts to — is published
    // before the removal.
    let mut exit_at = None;
    let mut removed_at = None;
    let mut seen = 0;
    while let Ok(event) = rx.try_recv() {
        match event {
            PodiumEvent::ProcessStatusChanged {
                process_id,
                status: ProcessStatus::Exited { .. },
                ..
            } if process_id == agent_id => exit_at = Some(seen),
            PodiumEvent::ProcessRemoved { process_id, .. } => {
                assert_eq!(process_id, agent_id, "only the agent is removed");
                removed_at = Some(seen);
            }
            _ => {}
        }
        seen += 1;
    }
    assert!(exit_at.unwrap() < removed_at.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(windows, ignore = "POSIX sleep")]
async fn a_restart_within_the_window_restarts_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let orch = Arc::new(Orchestrator::new());
    // A 1 s window (5 × 200 ms); the agent runs 2 s once restarted, so its
    // second exit comes well after the first one's window would have ended.
    orch.set_agent_remove_exited_after(5).unwrap();
    let project = open(&orch, dir.path()).await;
    let id = orch
        .add_process(project, spec(agent(), "sleep 2", dir.path()))
        .await
        .unwrap();
    start_sweep(&orch, Duration::from_millis(200));

    orch.start_process(id).await.unwrap();
    orch.stop_process(id).await.unwrap();
    wait_for(&orch, id, exited).await;
    sleep(Duration::from_millis(300)).await;
    orch.start_process(id).await.unwrap();
    wait_for(&orch, id, |s| {
        matches!(s, Some(ProcessStatus::Running { .. }))
    })
    .await;

    // Running — through what would have been the end of the first window.
    wait_for(&orch, id, exited).await;
    // Exited again just now, so its clock starts over.
    assert!(
        exited(status_of(&orch, id)),
        "removed on the first exit's clock"
    );
    wait_for(&orch, id, |s| s.is_none()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn zero_never_removes_and_a_changed_setting_applies_to_the_next_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let orch = Arc::new(Orchestrator::new());
    orch.set_agent_remove_exited_after(0).unwrap();
    let project = open(&orch, dir.path()).await;
    let id = orch
        .add_process(project, spec(agent(), "exit 0", dir.path()))
        .await
        .unwrap();
    start_sweep(&orch, Duration::from_millis(10));
    orch.start_process(id).await.unwrap();
    wait_for(&orch, id, exited).await;

    // Many windows of any length later, still there.
    sleep(Duration::from_millis(400)).await;
    assert!(exited(status_of(&orch, id)));

    // No restart needed: the next sweep reads the new value.
    orch.set_agent_remove_exited_after(1).unwrap();
    wait_for(&orch, id, |s| s.is_none()).await;
}
