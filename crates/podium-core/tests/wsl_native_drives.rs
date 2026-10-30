//! Integration tests for WSL-backend mode with the `nativeDriveProjects`
//! opt-in on: a project on a Windows drive runs natively, one inside the
//! distro still runs there. Real ConPTY, real `wsl.exe`, a real distro.
//! Windows-only and opt-in, like `wsl_backend.rs` (whose tests cover the
//! default, where every project runs in the distro):
//! `PODIUM_TEST_WSL_DISTRO=Ubuntu cargo test -p podium-core --test wsl_native_drives -- --ignored --test-threads=1`.
//!
//! A separate test binary because the backend is process-wide: this one is
//! fixed with the opt-in on, `wsl_backend.rs` with it off.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use podium_core::{Orchestrator, ProcessId, ProcessKind, ProcessSpec, ProjectId, RestartPolicy};
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The distro under test, with the backend fixed to it and Windows-drive
/// projects opted in to running natively.
fn distro() -> String {
    let distro = std::env::var("PODIUM_TEST_WSL_DISTRO")
        .expect("set PODIUM_TEST_WSL_DISTRO to an installed distro (e.g. Ubuntu)");
    podium_core::platform::init_wsl(Some(&distro), true);
    distro
}

/// A fresh directory inside the distro, as Podium sees it from Windows.
struct WslDir {
    windows: PathBuf,
}

impl WslDir {
    /// Lives in `/var/tmp`, not `/tmp`: the first test of a run often boots
    /// a stopped distro, and creating the directory through
    /// `\\wsl.localhost` is what boots it. With systemd (on in the Ubuntu
    /// WSL images) boot then empties `/tmp` (`D /tmp` in
    /// `tmpfiles.d/tmp.conf`) after the directory was made, and the next
    /// `wsl.exe --cd` fails with `chdir(...) failed 2`. `/var/tmp` exists on
    /// every distro, is world-writable and survives boot.
    fn new(distro: &str, name: &str) -> Self {
        let unique = format!("podium-test-native-{name}-{}", std::process::id());
        let windows = PathBuf::from(format!(r"\\wsl.localhost\{distro}\var\tmp\{unique}"));
        std::fs::create_dir_all(&windows).expect("create dir inside the distro");
        Self { windows }
    }
}

impl Drop for WslDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.windows);
    }
}

async fn open(orch: &Orchestrator, root: &Path) -> ProjectId {
    orch.open_project(root.to_path_buf())
        .await
        .expect("open project")
}

async fn start_service(
    orch: &Orchestrator,
    project: ProjectId,
    root: &Path,
    command: &str,
) -> ProcessId {
    let spec = ProcessSpec {
        name: "test".to_string(),
        command: command.to_string(),
        cwd: root.to_path_buf(),
        env: Vec::new(),
        kind: ProcessKind::Service,
        restart_policy: RestartPolicy::Never,
        color: None,
    };
    let id = orch.add_process(project, spec).await.expect("add process");
    orch.start_process(id).await.expect("start");
    id
}

async fn wait_for_output(orch: &Orchestrator, id: ProcessId, needle: &str) -> String {
    timeout(TEST_TIMEOUT, async {
        loop {
            let text = orch.tail_text(id, usize::MAX).await.expect("tail_text");
            if text.contains(needle) {
                return text;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
}

/// Spawn an agent whose binary is overridden with `binary` (no MCP server is
/// running, so the launch is just `<binary> <prompt>`), through the real
/// `build_launch` quoting and the real PTY launch path.
async fn spawn_agent(
    orch: &Orchestrator,
    project: ProjectId,
    binary: &str,
    prompt: &str,
) -> ProcessId {
    orch.set_agent_override("claude-code", Some(binary.to_string()), Vec::new())
        .expect("override");
    orch.spawn_agent(
        project,
        Some("claude-code".to_string()),
        Some("test-agent".to_string()),
        Some(prompt.to_string()),
        Vec::new(),
        Vec::new(),
        false,
        None,
        None,
    )
    .await
    .expect("spawn agent")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn a_drive_project_runs_natively_and_a_distro_project_in_the_distro() {
    let distro = distro();
    let drive = tempfile::tempdir().expect("temp dir");
    let inside = WslDir::new(&distro, "svc");
    let orch = Orchestrator::new();

    let native = open(&orch, drive.path()).await;
    let in_distro = open(&orch, &inside.windows).await;
    let infos = orch.list_projects();
    let info = |id| infos.iter().find(|p| p.id == id).expect("listed");
    assert!(info(native).native_in_wsl_mode);
    assert!(!info(in_distro).native_in_wsl_mode);

    // `%OS%` only expands in cmd.exe; bash would print it verbatim.
    let id = start_service(&orch, native, drive.path(), "echo os:%OS%:end").await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(text.contains("os:Windows_NT:end"), "{text:?}");

    let id = start_service(
        &orch,
        in_distro,
        &inside.windows,
        r#"printf 'os:%s:end' "$(uname -s)""#,
    )
    .await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(text.contains("os:Linux:end"), "{text:?}");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn an_agent_in_a_drive_project_is_quoted_and_launched_natively() {
    distro();
    let drive = tempfile::tempdir().expect("temp dir");
    let orch = Orchestrator::new();
    let project = open(&orch, drive.path()).await;
    // Windows PowerShell runs its first positional argument as a command. It
    // only prints `arg:it's spaced` when the prompt reached it as one argv
    // element through the native direct launch: POSIX quoting would hand it
    // `'Write-Output …'` (a string literal, echoed back as code), and a
    // `cmd.exe /C` layer would split it on its spaces.
    let powershell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
    let id = spawn_agent(
        &orch,
        project,
        powershell,
        "Write-Output ('arg:' + 'it''s spaced' + ':end')",
    )
    .await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(text.contains("arg:it's spaced:end"), "{text:?}");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn an_agent_in_a_distro_project_is_quoted_for_bash() {
    let distro = distro();
    let inside = WslDir::new(&distro, "agent");
    let orch = Orchestrator::new();
    let project = open(&orch, &inside.windows).await;
    let id = spawn_agent(&orch, project, "printf", "got:it's \"spaced\":end").await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(text.contains("got:it's \"spaced\":end"), "{text:?}");
    orch.shutdown().await;
}
