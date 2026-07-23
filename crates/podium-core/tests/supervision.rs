//! Integration tests for `podium.yml` project config and process supervision.

use std::path::Path;
use std::time::Duration;

use podium_core::{
    Orchestrator, PodiumEvent, ProcessId, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy,
    SupervisorConfig,
};
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
// Processes spawn via an interactive login shell (`-lic`), so a spawn can
// itself take a couple of seconds on a heavier shell profile (oh-my-zsh,
// p10k, …) — comfortably longer than the fast backoff timings below. The
// "no more restarts" quiet window must outlast that per-spawn cost.
const RESTART_QUIET_WINDOW: Duration = Duration::from_secs(8);

fn fast_supervisor() -> SupervisorConfig {
    SupervisorConfig {
        backoff_base: Duration::from_millis(20),
        backoff_cap: Duration::from_millis(100),
        breaker_window: Duration::from_secs(60),
        breaker_max_restarts: 3,
        backoff_reset_after: Duration::from_secs(60),
    }
}

fn spec(name: &str, command: &str, cwd: &Path, policy: RestartPolicy) -> ProcessSpec {
    ProcessSpec {
        name: name.to_string(),
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        kind: ProcessKind::Service,
        restart_policy: policy,
        color: None,
    }
}

fn write_config(dir: &Path, contents: &str) {
    std::fs::write(dir.join("podium.yml"), contents).expect("write podium.yml");
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
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for process to exit")
}

/// Count `Running` status events until the bus stays quiet for `quiet`.
async fn running_events_until_quiet(
    rx: &mut broadcast::Receiver<PodiumEvent>,
    quiet: Duration,
) -> usize {
    let mut count = 0;
    loop {
        match timeout(quiet, rx.recv()).await {
            Ok(Ok(PodiumEvent::ProcessStatusChanged {
                status: ProcessStatus::Running { .. },
                ..
            })) => count += 1,
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => return count,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn on_crash_policy_restarts_until_breaker_trips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::with_supervisor_config(fast_supervisor());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .add_process(
            project_id,
            spec("crasher", "exit 1", dir.path(), RestartPolicy::OnCrash),
        )
        .await
        .expect("add process");
    let mut rx = orch.subscribe_events();
    orch.start_process(id).await.expect("start");

    let runs = timeout(
        TEST_TIMEOUT,
        running_events_until_quiet(&mut rx, RESTART_QUIET_WINDOW),
    )
    .await
    .expect("timed out counting restarts");
    assert_eq!(runs, 4, "1 manual start + breaker_max_restarts(3) restarts");
    let status = status_of(&orch, id);
    assert!(
        matches!(status, ProcessStatus::Exited { crashed: true, .. }),
        "settled as crashed after the breaker tripped, got {status:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn always_policy_restarts_clean_exits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::with_supervisor_config(fast_supervisor());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .add_process(
            project_id,
            spec("looper", "exit 0", dir.path(), RestartPolicy::Always),
        )
        .await
        .expect("add process");
    let mut rx = orch.subscribe_events();
    orch.start_process(id).await.expect("start");

    let runs = timeout(
        TEST_TIMEOUT,
        running_events_until_quiet(&mut rx, RESTART_QUIET_WINDOW),
    )
    .await
    .expect("timed out counting restarts");
    assert_eq!(runs, 4, "clean exits are restarted under `always`");
    let status = status_of(&orch, id);
    assert!(
        matches!(status, ProcessStatus::Exited { crashed: false, .. }),
        "clean exit is not a crash, got {status:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn never_policy_does_not_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::with_supervisor_config(fast_supervisor());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .add_process(
            project_id,
            spec("oneshot", "exit 1", dir.path(), RestartPolicy::Never),
        )
        .await
        .expect("add process");
    let mut rx = orch.subscribe_events();
    orch.start_process(id).await.expect("start");

    let runs = timeout(
        TEST_TIMEOUT,
        running_events_until_quiet(&mut rx, Duration::from_secs(1)),
    )
    .await
    .expect("timed out counting restarts");
    assert_eq!(runs, 1, "no supervised restart under `never`");
}

#[tokio::test(flavor = "multi_thread")]
async fn user_stop_prevents_supervised_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::with_supervisor_config(fast_supervisor());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .add_process(
            project_id,
            spec("service", "sleep 30", dir.path(), RestartPolicy::Always),
        )
        .await
        .expect("add process");
    orch.start_process(id).await.expect("start");

    let mut rx = orch.subscribe_events();
    orch.stop_process(id).await.expect("stop");
    let status = wait_for_exit(&orch, id).await;
    assert!(
        matches!(status, ProcessStatus::Exited { crashed: false, .. }),
        "user stop is not a crash, got {status:?}"
    );
    let runs = running_events_until_quiet(&mut rx, Duration::from_millis(500)).await;
    assert_eq!(runs, 0, "user-stopped process must not be restarted");
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_cancels_a_pending_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Long backoff so the pending restart is still waiting when we cancel.
    let orch = Orchestrator::with_supervisor_config(SupervisorConfig {
        backoff_base: Duration::from_secs(1),
        backoff_cap: Duration::from_secs(1),
        ..fast_supervisor()
    });
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .add_process(
            project_id,
            spec("crasher", "exit 1", dir.path(), RestartPolicy::OnCrash),
        )
        .await
        .expect("add process");
    orch.start_process(id).await.expect("start");
    wait_for_exit(&orch, id).await;
    // Give the exit handler a moment to record the pending restart task.
    sleep(Duration::from_millis(100)).await;

    let mut rx = orch.subscribe_events();
    orch.stop_process(id)
        .await
        .expect("cancelling a pending restart is ok");
    let runs = running_events_until_quiet(&mut rx, Duration::from_millis(2500)).await;
    assert_eq!(runs, 0, "cancelled restart must never fire");
    assert!(matches!(
        status_of(&orch, id),
        ProcessStatus::Exited { crashed: true, .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn open_project_loads_config_and_auto_starts() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        "name: Web Shop\nprocesses:\n  - name: web\n    command: sleep 30\n    auto_start: true\n",
    );
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let projects = orch.list_projects();
    let project = projects
        .iter()
        .find(|p| p.id == project_id)
        .expect("listed");
    assert_eq!(project.name, "Web Shop");
    assert_eq!(project.icon_initials, "WS");
    assert!(project.config_error.is_none());

    let procs = orch.list_processes(Some(project_id));
    assert_eq!(procs.len(), 1);
    assert_eq!(procs[0].name, "web");
    assert!(
        matches!(procs[0].status, ProcessStatus::Running { .. }),
        "auto_start process is running, got {:?}",
        procs[0].status
    );
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_config_still_opens_project_with_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(dir.path(), "nmae: Webshop\n");
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("a broken config still opens the project");

    let projects = orch.list_projects();
    let project = projects
        .iter()
        .find(|p| p.id == project_id)
        .expect("listed");
    let err = project
        .config_error
        .as_deref()
        .expect("config error surfaced");
    assert!(err.contains("nmae"), "error names the bad key: {err}");
    let folder = dir.path().file_name().unwrap().to_string_lossy();
    assert_eq!(project.name, folder);
    assert!(orch.list_processes(Some(project_id)).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_same_folder_returns_the_same_project() {
    // Re-opening an already-open folder must be idempotent: it returns the
    // existing project id and never creates a second record. This is what
    // keeps a double startup restore from duplicating sidebar entries.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();

    let first = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("first open");
    let second = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("second open");

    assert_eq!(first, second, "same folder yields the same project id");
    assert_eq!(orch.list_projects().len(), 1, "no duplicate record");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrently_opening_same_folder_dedupes() {
    // The frontend fires `restoreWorkspace()` from a `useEffect` with no
    // dependency array; React StrictMode (dev builds) intentionally mounts,
    // cleans up, and remounts once, so two overlapping `open_project` calls
    // for the same folder are a real startup scenario, not just a
    // sequential re-open. The "already open" check and the project-map
    // insert are separated by an `.await` (the config-file load), so two
    // concurrent calls can both pass the check before either has inserted.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();

    let (first, second) = tokio::join!(
        orch.open_project(dir.path().to_path_buf()),
        orch.open_project(dir.path().to_path_buf()),
    );

    assert_eq!(
        first.expect("first open"),
        second.expect("second open"),
        "concurrent opens of the same folder must dedupe to one project id"
    );
    assert_eq!(
        orch.list_projects().len(),
        1,
        "no duplicate record from the race"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_via_different_path_spelling_dedupes() {
    // A trailing-slash / `.` spelling of the same folder canonicalizes to the
    // same identity, so it also dedupes to one project.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();

    let first = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("first open");
    let second = orch
        .open_project(dir.path().join("."))
        .await
        .expect("second open (dotted path)");

    assert_eq!(first, second, "canonicalized spellings share one id");
    assert_eq!(orch.list_projects().len(), 1, "no duplicate record");
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_replaces_config_processes_and_keeps_manual_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        "processes:\n  - name: alpha\n    command: sleep 30\n",
    );
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    orch.add_process(
        project_id,
        spec("manual", "sleep 30", dir.path(), RestartPolicy::Never),
    )
    .await
    .expect("add manual process");

    write_config(
        dir.path(),
        "name: Renamed\nprocesses:\n  - name: beta\n    command: sleep 30\n    auto_start: true\n",
    );
    orch.reload_project_config(project_id)
        .await
        .expect("reload");

    let projects = orch.list_projects();
    let project = projects
        .iter()
        .find(|p| p.id == project_id)
        .expect("listed");
    assert_eq!(project.name, "Renamed");
    assert!(project.config_error.is_none());

    let procs = orch.list_processes(Some(project_id));
    let names: Vec<&str> = procs.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"manual"), "manual process kept: {names:?}");
    assert!(
        names.contains(&"beta"),
        "new config process added: {names:?}"
    );
    assert!(
        !names.contains(&"alpha"),
        "old config process gone: {names:?}"
    );
    let beta = procs.iter().find(|p| p.name == "beta").unwrap();
    assert!(
        matches!(beta.status, ProcessStatus::Running { .. }),
        "reload auto-starts, got {:?}",
        beta.status
    );
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_with_broken_yaml_keeps_processes_and_sets_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_config(
        dir.path(),
        "processes:\n  - name: alpha\n    command: sleep 30\n",
    );
    let orch = Orchestrator::new();
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    write_config(dir.path(), "processes: [not a process]\n");
    orch.reload_project_config(project_id)
        .await
        .expect("broken reload still returns ok");

    let projects = orch.list_projects();
    let project = projects
        .iter()
        .find(|p| p.id == project_id)
        .expect("listed");
    assert!(project.config_error.is_some(), "reload error surfaced");
    let procs = orch.list_processes(Some(project_id));
    assert_eq!(procs.len(), 1, "existing processes kept on broken reload");
    assert_eq!(procs[0].name, "alpha");
}
