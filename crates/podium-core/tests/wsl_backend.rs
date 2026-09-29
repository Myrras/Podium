//! Integration tests for WSL-backend mode: real ConPTY, real `wsl.exe`, a
//! real distro. Windows-only, and opt-in: they run only when
//! `PODIUM_TEST_WSL_DISTRO` names an installed distro (e.g. `Ubuntu`), so CI
//! and machines without WSL skip them. Run with
//! `PODIUM_TEST_WSL_DISTRO=Ubuntu cargo test -p podium-core --test wsl_backend`.
//!
//! Each test binary is its own process, so fixing the process-wide backend
//! with `init_wsl` here cannot leak into any other test.
#![cfg(windows)]

use std::path::PathBuf;
use std::time::Duration;

use podium_core::{
    Orchestrator, ProcessId, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy,
};
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The distro under test, with the backend fixed to it; `None` = skip.
fn distro() -> Option<String> {
    let distro = std::env::var("PODIUM_TEST_WSL_DISTRO").ok()?;
    podium_core::platform::init_wsl(Some(&distro));
    Some(distro)
}

/// A fresh directory inside the distro, as Podium sees it from Windows.
struct WslDir {
    windows: PathBuf,
}

impl WslDir {
    fn new(distro: &str, name: &str) -> Self {
        let unique = format!("podium-test-{name}-{}", std::process::id());
        let windows = PathBuf::from(format!(r"\\wsl.localhost\{distro}\tmp\{unique}"));
        std::fs::create_dir_all(&windows).expect("create dir inside the distro");
        Self { windows }
    }

    fn linux(&self) -> String {
        let name = self.windows.file_name().unwrap().to_string_lossy();
        format!("/tmp/{name}")
    }
}

impl Drop for WslDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.windows);
    }
}

fn service(command: &str, dir: &WslDir, env: Vec<(String, String)>) -> ProcessSpec {
    ProcessSpec {
        name: "test".to_string(),
        command: command.to_string(),
        cwd: dir.windows.clone(),
        env,
        kind: ProcessKind::Service,
        restart_policy: RestartPolicy::Never,
        color: None,
    }
}

async fn start(orch: &Orchestrator, dir: &WslDir, spec: ProcessSpec) -> ProcessId {
    let project = orch
        .open_project(dir.windows.clone())
        .await
        .expect("open project");
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

fn status_of(orch: &Orchestrator, id: ProcessId) -> ProcessStatus {
    orch.list_processes(None)
        .into_iter()
        .find(|p| p.id == id)
        .expect("process listed")
        .status
}

/// Whether a process whose command line matches `pattern` runs in `distro`.
fn running_in_distro(distro: &str, pattern: &str) -> bool {
    std::process::Command::new("wsl.exe")
        .args(["-d", distro, "-e", "pgrep", "-f", pattern])
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_runs_in_the_distro_with_its_cwd_and_env() {
    let Some(distro) = distro() else { return };
    let dir = WslDir::new(&distro, "cwd-env");
    let orch = Orchestrator::new();
    let env = vec![("PODIUM_TEST_VALUE".to_string(), "hi there".to_string())];
    let id = start(
        &orch,
        &dir,
        service(
            r#"printf 'got:%s|%s|%s:end' "$(pwd)" "$PODIUM_TEST_VALUE" "$TERM""#,
            &dir,
            env,
        ),
    )
    .await;
    let text = wait_for_output(&orch, id, ":end").await;
    let expected = format!("got:{}|hi there|xterm-256color:end", dir.linux());
    assert!(text.contains(&expected), "{text:?}");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_posix_quoted_argument_survives_portable_pty_and_wsl_exe() {
    let Some(distro) = distro() else { return };
    let dir = WslDir::new(&distro, "argv");
    let orch = Orchestrator::new();
    let prompt = "You're the \"agent\"; $HOME `id` \\ back";
    let quoted = podium_core::platform::quote_posix(prompt).unwrap();
    let id = start(
        &orch,
        &dir,
        service(&format!("printf '<%s>' {quoted}"), &dir, Vec::new()),
    )
    .await;
    let text = wait_for_output(&orch, id, ">").await;
    assert!(text.contains(&format!("<{prompt}>")), "{text:?}");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_service_ends_its_linux_children() {
    let Some(distro) = distro() else { return };
    let dir = WslDir::new(&distro, "stop");
    let orch = Orchestrator::new();
    // Two long-lived children of the service's shell, like `pnpm dev` and
    // the dev server it forks.
    let marker = format!("podium-stop-{}", std::process::id());
    let command = format!(
        "bash -c 'exec -a {marker}-a sleep 300' & bash -c 'exec -a {marker}-b sleep 300' & wait"
    );
    let id = start(&orch, &dir, service(&command, &dir, Vec::new())).await;
    // Anchored on argv0 (`exec -a`): the marker also appears in the wrapper
    // `bash -lic` command line, which a bare `pgrep -f marker` would match
    // before the `sleep` children exist.
    let children = format!("^{marker}-");
    timeout(TEST_TIMEOUT, async {
        while !running_in_distro(&distro, &children) {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("children never started");

    orch.stop_process(id).await.expect("stop");
    timeout(TEST_TIMEOUT, async {
        while !matches!(status_of(&orch, id), ProcessStatus::Exited { .. }) {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("service never exited");
    // Give WSL a moment to tear the session down.
    let gone = timeout(Duration::from_secs(10), async {
        while running_in_distro(&distro, &children) {
            sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .is_ok();
    assert!(gone, "Linux children outlived the stopped service");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_windows_config_file_is_readable_at_its_backend_path() {
    // Agent/assistant `--mcp-config` and `--rules` files live in the Windows
    // app data dir; the CLI inside WSL must be able to open them.
    let Some(distro) = distro() else { return };
    let dir = WslDir::new(&distro, "config");
    let config = tempfile::Builder::new()
        .suffix(".json")
        .tempfile_in(std::env::temp_dir())
        .expect("temp config file");
    std::fs::write(config.path(), "{\"podium\":\"reachable\"}").unwrap();
    let linux = podium_core::platform::backend_path(config.path()).expect("backend path");
    assert!(linux.starts_with("/mnt/"), "{linux}");
    let command = format!(
        "cat {}; printf ':end'",
        podium_core::platform::quote_posix(&linux).unwrap()
    );
    let orch = Orchestrator::new();
    let id = start(&orch, &dir, service(&command, &dir, Vec::new())).await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(text.contains("\"podium\":\"reachable\""), "{text:?}");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_windows_host_is_reachable_on_localhost() {
    // The MCP server binds 127.0.0.1 on Windows; agents in WSL reach it only
    // with mirrored networking (`networkingMode=mirrored` in .wslconfig).
    let Some(distro) = distro() else { return };
    let dir = WslDir::new(&distro, "net");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nfrom-window",
            );
        }
    });
    let command = format!("curl -s --max-time 5 http://127.0.0.1:{port}/; printf ':end'");
    let orch = Orchestrator::new();
    let id = start(&orch, &dir, service(&command, &dir, Vec::new())).await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(
        text.contains("from-window"),
        "Windows host not reachable from WSL on 127.0.0.1 — is networkingMode=mirrored set? {text:?}"
    );
    orch.shutdown().await;
}
