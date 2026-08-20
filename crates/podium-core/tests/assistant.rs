//! Integration tests for the personal-assistant session, driven by a fake
//! `claude` binary: a script that speaks the same `stream-json` protocol as
//! the real CLI, so the whole spawn → send → parse → event path runs for real
//! without needing Claude Code installed.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use podium_core::assistant::{AssistantEvent, TranscriptRole};
use podium_core::{
    AssistantConfig, McpConnectInfo, Orchestrator, PodiumEvent, ProactivityConfig, ProcessKind,
    ProcessSpec, RestartPolicy,
};
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

/// An orchestrator whose assistant runs `script` and persists its
/// conversation in `dir` — the same dir twice stands in for an app restart.
fn setup_in(dir: &Path, script: &str) -> Orchestrator {
    let orch = Orchestrator::new();
    orch.configure_assistant(AssistantConfig {
        command: write_fake(dir, script).to_string_lossy().into_owned(),
        cwd: Some(dir.to_path_buf()),
        data_dir: Some(dir.to_path_buf()),
    });
    orch
}

fn setup(script: &str) -> (Orchestrator, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = setup_in(dir.path(), script);
    (orch, dir)
}

/// The arguments the fake CLI was launched with (it records them on spawn).
async fn recorded_argv(dir: &Path) -> Vec<String> {
    timeout(TEST_TIMEOUT, async {
        loop {
            if let Ok(argv) = std::fs::read_to_string(dir.join("argv.txt")) {
                return argv.lines().map(str::to_string).collect();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake cli records its argv")
}

/// The value that follows `flag` in `args`, if the flag was passed.
fn arg_after(args: &[String], flag: &str) -> Option<String> {
    let at = args.iter().position(|a| a == flag)?;
    args.get(at + 1).cloned()
}

/// Watcher timings a test can wait on: everything in milliseconds.
fn fast_proactivity() -> ProactivityConfig {
    ProactivityConfig {
        tick: Duration::from_millis(20),
        idle_after: Duration::from_millis(50),
        lingering_after: Duration::from_millis(10),
        sweep_every: Duration::from_millis(30),
    }
}

/// An agent process running `command`, so its exit can be observed.
fn agent_spec(command: &str, cwd: &Path) -> ProcessSpec {
    ProcessSpec {
        name: "fix-login".to_string(),
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        kind: ProcessKind::Agent {
            adapter: "claude-code".to_string(),
        },
        restart_policy: RestartPolicy::Never,
        color: None,
    }
}

/// The text of the first `notice` event, i.e. what Podium said unprompted.
async fn next_notice(rx: &mut Receiver<PodiumEvent>) -> String {
    let events = collect_until(rx, |e| matches!(e, AssistantEvent::Notice { .. })).await;
    match events.last() {
        Some(AssistantEvent::Notice { text }) => text.clone(),
        other => panic!("expected a notice, got {other:?}"),
    }
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
    let args = recorded_argv(dir.path()).await;
    // The assistant's own (gated) MCP config, and an identity prompt telling
    // it what it is. `--strict-mcp-config` is deliberately absent: the user's
    // own MCP servers (mail, Slack, …) must stay available.
    assert_eq!(
        arg_after(&args, "--mcp-config"),
        Some(
            config_dir
                .join("assistant.json")
                .to_string_lossy()
                .into_owned()
        ),
    );
    let prompt = arg_after(&args, "--append-system-prompt").expect("identity prompt passed");
    assert!(prompt.contains("Podium"), "{prompt}");
    assert!(!args.iter().any(|a| a == "--strict-mcp-config"));

    orch.shutdown().await;
}

#[tokio::test]
async fn the_transcript_and_session_survive_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = setup_in(dir.path(), FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("what is open?").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    orch.shutdown().await;

    // A second orchestrator over the same data dir is what a restart looks
    // like: the panel replays these rows.
    let restarted = setup_in(dir.path(), ARGV_RECORDING_CLAUDE);
    let rows: Vec<(TranscriptRole, String)> = restarted
        .assistant_transcript()
        .into_iter()
        .map(|e| (e.role, e.text))
        .collect();
    assert_eq!(
        rows,
        vec![
            (TranscriptRole::User, "what is open?".to_string()),
            (TranscriptRole::Assistant, "hi there".to_string()),
            (TranscriptRole::Tool, "list_todos".to_string()),
        ]
    );

    // …and the next turn picks Claude's own session back up rather than
    // starting a stranger.
    restarted.assistant_send("and now?").await.expect("send");
    let args = recorded_argv(dir.path()).await;
    assert_eq!(
        arg_after(&args, "--resume"),
        Some("fake-session".to_string())
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn a_new_conversation_drops_the_transcript_and_the_session_quietly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = setup_in(dir.path(), FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(!orch.assistant_transcript().is_empty());

    orch.assistant_new_conversation().await;
    assert!(orch.assistant_transcript().is_empty());
    // Killing the old session is Podium's doing, so it must not surface as
    // "the assistant session ended" in the freshly cleared panel.
    let quiet = timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(quiet.is_err(), "unexpected event after a new conversation");

    // Nothing left to resume either.
    let fresh = setup_in(dir.path(), ARGV_RECORDING_CLAUDE);
    fresh.assistant_send("hello again").await.expect("send");
    let args = recorded_argv(dir.path()).await;
    assert_eq!(arg_after(&args, "--resume"), None);
    fresh.shutdown().await;
    orch.shutdown().await;
}

#[tokio::test]
async fn a_session_id_that_no_longer_starts_is_forgotten() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A stored id Claude has since pruned: resuming it fails at launch.
    std::fs::write(
        dir.path().join("assistant.json"),
        r#"{"sessionId":"stale-session","transcript":[]}"#,
    )
    .expect("seed conversation");
    let orch = setup_in(dir.path(), DYING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;

    // The next attempt must not carry the same doomed `--resume` forever.
    let stored = std::fs::read_to_string(dir.path().join("assistant.json")).expect("read");
    assert!(!stored.contains("stale-session"), "{stored}");
    orch.shutdown().await;
}

#[tokio::test]
async fn the_memory_file_is_injected_into_the_session_prompt() {
    let (orch, dir) = setup(ARGV_RECORDING_CLAUDE);

    orch.set_assistant_memory("Prefers short answers. Ships on Fridays.")
        .expect("store memory");
    assert_eq!(
        orch.assistant_memory(),
        "Prefers short answers. Ships on Fridays."
    );
    assert!(dir.path().join("assistant-memory.md").exists());

    orch.assistant_send("hello").await.expect("send");
    recorded_argv(dir.path()).await;
    // The memory is a multi-line block inside the identity prompt, so assert
    // on the whole recorded argv rather than one line of it.
    let argv = std::fs::read_to_string(dir.path().join("argv.txt")).expect("read argv");
    assert!(argv.contains("# Your memory"), "{argv}");
    assert!(argv.contains("Ships on Fridays"), "{argv}");
    orch.shutdown().await;
}

#[tokio::test]
async fn the_session_may_never_edit_files_itself() {
    let (orch, dir) = setup(ARGV_RECORDING_CLAUDE);

    orch.assistant_send("hello").await.expect("send");
    let args = recorded_argv(dir.path()).await;

    // The assistant delegates code work to spawned agents; the CLI itself
    // refuses the file-editing and shell tools. Loosening this line would let
    // it write code straight into the user's projects.
    assert_eq!(
        arg_after(&args, "--disallowedTools"),
        Some("Edit,Write,NotebookEdit,Bash".to_string())
    );
    // bypassPermissions is required for -p/stream-json mode to make any
    // progress (no TTY for an interactive prompt) — safe only because the
    // disallowed-tools deny list above still wins over it.
    assert_eq!(
        arg_after(&args, "--permission-mode"),
        Some("bypassPermissions".to_string())
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_finished_is_reported_in_the_chat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    // A live session to nudge, and the day-start summary claimed up front so
    // it is not what we end up asserting on.
    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(orch.assistant_claim_day_start());
    // Only the exit is under test here: a login shell can take longer to
    // start than the fast idle threshold, which would flag the agent as stuck
    // first.
    podium_core::proactivity::spawn(
        &orch,
        ProactivityConfig {
            idle_after: Duration::from_secs(60),
            ..fast_proactivity()
        },
    );

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let agent = orch
        .add_process(project, agent_spec("exit 0", dir.path()))
        .await
        .expect("add agent");
    orch.start_process(agent).await.expect("start agent");

    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("fix-login"), "{notice}");
    assert!(notice.contains("finished"), "{notice}");
    orch.shutdown().await;
}

#[tokio::test]
async fn a_stuck_agent_is_reported_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(orch.assistant_claim_day_start());

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    // Silent but alive: exactly what an agent waiting on a question looks like.
    let agent = orch
        .add_process(project, agent_spec("sleep 30", dir.path()))
        .await
        .expect("add agent");
    orch.start_process(agent).await.expect("start agent");
    podium_core::proactivity::spawn(&orch, fast_proactivity());

    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("waiting on input"), "{notice}");

    // Every tick sees the same silent agent; it must be mentioned only once.
    let repeated = timeout(Duration::from_millis(300), async {
        loop {
            if let Ok(PodiumEvent::AssistantEvent {
                event: AssistantEvent::Notice { text },
            }) = rx.recv().await
            {
                return text;
            }
        }
    })
    .await;
    assert!(repeated.is_err(), "nudged twice about the same agent");
    orch.shutdown().await;
}

#[tokio::test]
async fn proactivity_off_keeps_podium_quiet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.set_assistant_proactivity(false).expect("switch off");
    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    podium_core::proactivity::spawn(&orch, fast_proactivity());

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let agent = orch
        .add_process(project, agent_spec("exit 0", dir.path()))
        .await
        .expect("add agent");
    orch.start_process(agent).await.expect("start agent");

    // Not the day-start summary, not the exited agent, nothing.
    let quiet = timeout(Duration::from_millis(400), async {
        loop {
            if let Ok(PodiumEvent::AssistantEvent {
                event: AssistantEvent::Notice { text },
            }) = rx.recv().await
            {
                return text;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "nudged while proactivity is off");
    assert!(
        orch.assistant_claim_day_start(),
        "the day-start summary must not be claimed while switched off"
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn a_nudge_with_nobody_chatting_waits_for_the_next_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), ARGV_RECORDING_CLAUDE));
    let mut rx = orch.subscribe_events();

    // No session yet: the day-start nudge must not start the CLI on its own —
    // a fresh session would not know what the nudge is about.
    podium_core::proactivity::spawn(&orch, fast_proactivity());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!dir.path().join("argv.txt").exists(), "spawned for a nudge");

    // The user's first turn takes it along.
    orch.assistant_send("morning").await.expect("send");
    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("day-start summary"), "{notice}");
    orch.shutdown().await;
}

#[tokio::test]
async fn a_pending_proposal_dies_with_its_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = setup_in(dir.path(), FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();
    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let id = orch.propose_assistant_action(
        "set_todo_archived".to_string(),
        "{}".to_string(),
        Some(project),
        Box::new(|| Box::pin(async { Ok("archived".to_string()) })),
    );
    assert_eq!(orch.assistant_proposals().len(), 1);

    orch.close_project(project).await.expect("close project");

    // Confirming it later could only fail, so the card resolves itself with
    // the reason instead of waiting for a user who cannot help.
    assert!(orch.assistant_proposals().is_empty());
    let events = collect_until(&mut rx, |e| {
        matches!(e, AssistantEvent::ProposalResolved { .. })
    })
    .await;
    assert_eq!(
        events.last(),
        Some(&AssistantEvent::ProposalResolved {
            id,
            approved: false,
            error: Some("its project was closed".to_string()),
        })
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn to_dos_left_open_are_swept_into_the_chat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(orch.assistant_claim_day_start());

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    orch.add_todo(project, "Fix the login redirect")
        .expect("add todo");
    // Anything older than 10ms counts as lingering here.
    podium_core::proactivity::spawn(
        &orch,
        ProactivityConfig {
            idle_after: Duration::from_secs(60),
            ..fast_proactivity()
        },
    );

    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("Fix the login redirect"), "{notice}");
    orch.shutdown().await;
}
