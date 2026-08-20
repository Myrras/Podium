//! Integration tests for the personal-assistant session, driven by a fake
//! `claude` binary: a script that speaks the same `stream-json` protocol as
//! the real CLI, so the whole spawn → send → parse → event path runs for real
//! without needing Claude Code installed.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use podium_core::assistant::AssistantEvent;
use podium_core::{AssistantConfig, McpConnectInfo, Orchestrator, PodiumEvent};
use tokio::sync::broadcast::Receiver;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A fake CLI: greets with an init line, then answers every stdin turn with
/// one prose block, one tool call and a success result.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r _line; do
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hi there"},{"type":"tool_use","name":"list_todos"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"hi there"}'
done
"#;

/// A fake CLI that dies immediately — stands in for "claude is not installed".
const DYING_CLAUDE: &str = "#!/bin/sh\nexit 1\n";

/// A fake CLI that records the arguments Podium launched it with, so the
/// spawn flags can be asserted without a real Claude Code.
const ARGV_RECORDING_CLAUDE: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$(dirname "$0")/argv.txt"
sleep 5
"#;

fn write_fake(dir: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude");
    std::fs::write(&path, script).expect("write fake cli");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn setup(script: &str) -> (Orchestrator, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new();
    orch.configure_assistant(AssistantConfig {
        command: write_fake(dir.path(), script)
            .to_string_lossy()
            .into_owned(),
        cwd: Some(dir.path().to_path_buf()),
    });
    (orch, dir)
}

/// Collect assistant events until `stop` matches (or the test times out).
async fn collect_until(
    rx: &mut Receiver<PodiumEvent>,
    stop: impl Fn(&AssistantEvent) -> bool,
) -> Vec<AssistantEvent> {
    timeout(TEST_TIMEOUT, async {
        let mut seen = Vec::new();
        while let Ok(event) = rx.recv().await {
            if let PodiumEvent::AssistantEvent { event } = event {
                let done = stop(&event);
                seen.push(event);
                if done {
                    return seen;
                }
            }
        }
        seen
    })
    .await
    .expect("assistant events within timeout")
}

#[tokio::test]
async fn a_chat_turn_streams_prose_tool_calls_and_a_turn_end() {
    let (orch, _dir) = setup(FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    assert_eq!(
        events,
        vec![
            AssistantEvent::Started {
                session_id: "fake-session".to_string()
            },
            AssistantEvent::Message {
                text: "hi there".to_string()
            },
            AssistantEvent::Tool {
                name: "list_todos".to_string()
            },
            AssistantEvent::TurnEnd { error: None },
        ]
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn a_second_turn_reuses_the_same_session() {
    let (orch, _dir) = setup(FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("first send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    orch.assistant_send("second").await.expect("second send");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    // No second `Started`: the child from the first turn is still alive.
    assert_eq!(
        events,
        vec![
            AssistantEvent::Message {
                text: "hi there".to_string()
            },
            AssistantEvent::Tool {
                name: "list_todos".to_string()
            },
            AssistantEvent::TurnEnd { error: None },
        ]
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn a_session_that_dies_reports_exited_and_respawns_on_the_next_turn() {
    let (orch, _dir) = setup(DYING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;
    assert_eq!(events, vec![AssistantEvent::Exited]);

    // A dead session is replaced rather than written to, so the next turn
    // spawns a fresh child instead of failing on a broken pipe.
    orch.assistant_send("again").await.expect("send after exit");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;
    assert_eq!(events, vec![AssistantEvent::Exited]);
    orch.shutdown().await;
}

#[tokio::test]
async fn shutdown_kills_the_session() {
    let (orch, _dir) = setup(FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    orch.shutdown().await;

    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;
    assert_eq!(events, vec![AssistantEvent::Exited]);
}

#[tokio::test]
async fn the_session_is_launched_against_podiums_own_mcp_endpoint() {
    let (orch, dir) = setup(ARGV_RECORDING_CLAUDE);
    let config_dir = dir.path().join("mcp");
    orch.set_mcp_connect_info(McpConnectInfo {
        url: "http://127.0.0.1:39217".to_string(),
        token: "sekret-token".to_string(),
        config_dir: config_dir.clone(),
    });

    orch.assistant_send("hello").await.expect("send");
    let argv = timeout(TEST_TIMEOUT, async {
        loop {
            if let Ok(argv) = std::fs::read_to_string(dir.path().join("argv.txt")) {
                return argv;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake cli records its argv");

    let args: Vec<&str> = argv.lines().collect();
    // The assistant's own (gated) MCP config, and an identity prompt telling
    // it what it is. `--strict-mcp-config` is deliberately absent: the user's
    // own MCP servers (mail, Slack, …) must stay available.
    let at = args
        .iter()
        .position(|a| *a == "--mcp-config")
        .expect("--mcp-config passed");
    assert_eq!(
        args[at + 1],
        config_dir.join("assistant.json").to_string_lossy()
    );
    let at = args
        .iter()
        .position(|a| *a == "--append-system-prompt")
        .expect("identity prompt passed");
    assert!(args[at + 1].contains("Podium"), "{}", args[at + 1]);
    assert!(!args.contains(&"--strict-mcp-config"));

    orch.shutdown().await;
}
