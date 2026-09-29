//! Integration tests for WSL-backend mode: real ConPTY, real `wsl.exe`, a
//! real distro. Windows-only, and opt-in: every test is `#[ignore]`d, so CI
//! and machines without WSL report them as *ignored* (not as passing). Run
//! them one at a time (a burst of `wsl.exe` starts fails) with an installed
//! distro named in `PODIUM_TEST_WSL_DISTRO`:
//! `PODIUM_TEST_WSL_DISTRO=Ubuntu cargo test -p podium-core --test wsl_backend -- --ignored --test-threads=1`.
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

/// The distro under test, with the backend fixed to it. A run that asked for
/// these tests (`--ignored`) without naming a distro fails rather than
/// passing vacuously.
fn distro() -> String {
    let distro = std::env::var("PODIUM_TEST_WSL_DISTRO")
        .expect("set PODIUM_TEST_WSL_DISTRO to an installed distro (e.g. Ubuntu)");
    podium_core::platform::init_wsl(Some(&distro));
    distro
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

/// `wslinfo --networking-mode` inside `distro`, run without a shell.
fn networking_mode(distro: &str) -> Option<String> {
    let output = std::process::Command::new("wsl.exe")
        .args(["-d", distro, "-e", "wslinfo", "--networking-mode"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn a_service_runs_in_the_distro_with_its_cwd_and_env() {
    let distro = distro();
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
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn a_posix_quoted_argument_survives_portable_pty_and_wsl_exe() {
    let distro = distro();
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
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn stopping_a_service_ends_its_linux_children() {
    let distro = distro();
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
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn a_windows_config_file_is_readable_at_its_backend_path() {
    // Agent/assistant `--mcp-config` and `--rules` files live in the Windows
    // app data dir; the CLI inside WSL must be able to open them.
    let distro = distro();
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
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn the_windows_host_is_reachable_on_localhost() {
    // The MCP server binds 127.0.0.1 on Windows; agents in WSL reach it only
    // with mirrored networking (`networkingMode=mirrored` in .wslconfig).
    let distro = distro();
    // This checks the machine's .wslconfig, not Podium: without mirrored
    // networking skip loudly rather than fail a correct build.
    let mode = networking_mode(&distro);
    if mode.as_deref() != Some("mirrored") {
        eprintln!("skipped: WSL networking mode is {mode:?}, not \"mirrored\"");
        return;
    }
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

/// Run git inside the distro, in `dir` (a Linux path); panics on failure.
fn wsl_git(distro: &str, dir: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("wsl.exe")
        .args(["-d", distro, "-e", "git", "-C", dir])
        .args(args)
        .output()
        .expect("run git in WSL");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn worktrees_are_created_listed_and_removed_with_wsl_git() {
    let distro = distro();
    let dir = WslDir::new(&distro, "worktree");
    let repo = dir.linux();
    wsl_git(&distro, &repo, &["init", "-q", "-b", "main"]);
    wsl_git(&distro, &repo, &["config", "user.name", "Test"]);
    wsl_git(
        &distro,
        &repo,
        &["config", "user.email", "test@example.com"],
    );
    std::fs::write(dir.windows.join("README.md"), "sample\n").unwrap();
    wsl_git(&distro, &repo, &["add", "."]);
    wsl_git(&distro, &repo, &["commit", "-q", "-m", "initial"]);

    let created = podium_core::worktree::create(&dir.windows, "Fix login").expect("create");
    assert_eq!(created.branch, "podium/fix-login");
    assert!(created.path.is_dir(), "{:?} not a directory", created.path);
    // The branch exists in the distro's repo.
    let branches = wsl_git(&distro, &repo, &["branch", "--list", "podium/*"]);
    assert!(branches.contains("podium/fix-login"), "{branches:?}");
    assert_eq!(
        podium_core::worktree::current_branch(&created.path).as_deref(),
        Some("podium/fix-login")
    );

    let listed = podium_core::worktree::list(&dir.windows).expect("list");
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].name, "fix-login");
    assert_eq!(listed[0].path, created.path);

    // A process started in the worktree lands in its Linux directory.
    let orch = Orchestrator::new();
    let project = orch
        .open_project(dir.windows.clone())
        .await
        .expect("open project");
    let mut spec = service("printf 'pwd:%s:end' \"$(pwd)\"", &dir, Vec::new());
    spec.cwd = created.path.clone();
    let id = orch.add_process(project, spec).await.expect("add process");
    orch.start_process(id).await.expect("start");
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(
        text.contains(&format!("pwd:{repo}/.podium/worktrees/fix-login:end")),
        "{text:?}"
    );
    orch.shutdown().await;

    podium_core::worktree::remove(&dir.windows, "fix-login", false).expect("remove");
    assert!(podium_core::worktree::list(&dir.windows)
        .unwrap()
        .is_empty());
    assert!(!created.path.exists());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a WSL distro: set PODIUM_TEST_WSL_DISTRO, run with --ignored"]
async fn a_windows_exe_registered_for_wsl_clients_starts_through_interop() {
    // `claude mcp add … -- <exe> mcp-bridge` inside WSL: the exe path must be
    // its /mnt/c spelling, and WSL must be able to start that Windows binary.
    // This test binary stands in for Podium.exe.
    let distro = distro();
    let exe = std::env::current_exe().expect("current exe");
    let add = podium_core::mcp::install::claude_add_command(&exe).expect("add command");
    let linux_exe = podium_core::platform::backend_path(&exe).expect("backend path");
    assert!(linux_exe.starts_with("/mnt/"), "{linux_exe}");
    assert!(add.contains(&linux_exe), "{add}");

    let dir = WslDir::new(&distro, "interop");
    let command = format!(
        "{} --list; printf ':end'",
        podium_core::platform::quote_posix(&linux_exe).unwrap()
    );
    let orch = Orchestrator::new();
    let id = start(&orch, &dir, service(&command, &dir, Vec::new())).await;
    let text = wait_for_output(&orch, id, ":end").await;
    assert!(
        text.contains("a_windows_exe_registered_for_wsl_clients_starts_through_interop"),
        "{text:?}"
    );
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_startup_burst_of_wsl_launches_all_succeed() {
    // What Podium fires at startup: adapter probes and git calls for every
    // project at once, plus auto-start services. Ungated, wsl.exe lets only
    // two of a concurrent burst through and fails the rest after ~30 s.
    let Some(distro) = distro() else { return };
    let started = std::time::Instant::now();
    // A git repo to point the git calls at, as the app does for open projects.
    let repo_dir = WslDir::new(&distro, "burst-repo");
    wsl_git(&distro, &repo_dir.linux(), &["init", "-q", "-b", "main"]);
    wsl_git(
        &distro,
        &repo_dir.linux(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    let repo = repo_dir.windows.clone();
    // Mixed like the startup burst: exit-status probes (null stdio),
    // stdout-reading shell calls and git calls (piped stdio).
    let calls: Vec<std::thread::JoinHandle<bool>> = (0..9)
        .map(|i| {
            let repo = repo.clone();
            std::thread::spawn(move || match i % 3 {
                0 => podium_core::platform::run_shell_ok(
                    &podium_core::platform::command_exists_query("bash"),
                ),
                1 => podium_core::platform::run_shell_stdout("printf ok").as_deref() == Some("ok"),
                _ => podium_core::worktree::current_branch(&repo).as_deref() == Some("main"),
            })
        })
        .collect();
    let results: Vec<bool> = calls.into_iter().map(|c| c.join().unwrap()).collect();
    assert!(results.iter().all(|ok| *ok), "calls: {results:?}");

    let dir = WslDir::new(&distro, "burst");
    let orch = std::sync::Arc::new(Orchestrator::new());
    let project = orch
        .open_project(dir.windows.clone())
        .await
        .expect("open project");
    let mut ids = Vec::new();
    for i in 0..6 {
        let spec = service(&format!("printf 'svc{i}:end'"), &dir, Vec::new());
        ids.push(orch.add_process(project, spec).await.expect("add process"));
    }
    let starts: Vec<_> = ids
        .iter()
        .map(|&id| {
            let orch = std::sync::Arc::clone(&orch);
            tokio::spawn(async move { orch.start_process(id).await })
        })
        .collect();
    for start in starts {
        start.await.unwrap().expect("start");
    }
    for (i, &id) in ids.iter().enumerate() {
        wait_for_output(&orch, id, &format!("svc{i}:end")).await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "burst took {:?}: launches are timing out",
        started.elapsed()
    );
    orch.shutdown().await;
}
