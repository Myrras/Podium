//! Integration tests for the personal-assistant session, driven by a fake
//! `claude` binary: a script that speaks the same `stream-json` protocol as
//! the real CLI, so the whole spawn → send → parse → event path runs for real
//! without needing Claude Code installed.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use podium_core::assistant::{AssistantEvent, AssistantPolling, TranscriptRole};
use podium_core::{
    AssistantConfig, Attachment, CoreError, McpConnectInfo, Orchestrator, PendingId, PodiumEvent,
    ProactivityConfig, ProcessKind, ProcessSpec, RestartPolicy,
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

/// A fake CLI that always answers with the bare sentinel — what the assistant
/// says when a sweep found nothing the user has to act on.
const SILENT_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r _line; do
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"NOTHING_TO_REPORT"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
done
"#;

/// A fake CLI whose answer *mentions* the sentinel while still reporting
/// something real — that must reach the user untouched.
const MENTIONS_SENTINEL_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r _line; do
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Not NOTHING_TO_REPORT: a deploy is waiting on you."}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
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

/// A fake CLI that echoes each turn's own text back as its prose reply
/// (`echo:<text>`), so a test can tell *which* message produced *which*
/// turn — `FAKE_CLAUDE`'s canned reply cannot distinguish that.
const ECHOING_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r line; do
  text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')
  printf '{"type":"assistant","message":{"content":[{"type":"text","text":"echo:%s"}]}}\n' "$text"
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
done
"#;

/// A fake CLI whose first turn fails, and every turn after that succeeds —
/// for asserting that an error still advances the queue.
const ERRORS_ONCE_THEN_OK_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
first=1
while IFS= read -r _line; do
  if [ "$first" = "1" ]; then
    echo '{"type":"result","subtype":"error_max_turns","is_error":true}'
    first=0
  else
    echo '{"type":"assistant","message":{"content":[{"type":"text","text":"recovered"}]}}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
  fi
done
"#;

/// A fake CLI that hangs forever on a turn whose text starts with "hang",
/// and echoes back anything else — for driving `assistant_interrupt`.
const HANGING_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r line; do
  text=$(printf '%s' "$line" | sed -n 's/.*"text":"\([^"]*\)".*/\1/p')
  case "$text" in
    hang*) sleep 30 ;;
    *)
      printf '{"type":"assistant","message":{"content":[{"type":"text","text":"echo:%s"}]}}\n' "$text"
      echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
      ;;
  esac
done
"#;

/// A fake CLI that reads exactly one turn, then dies without ever answering
/// it — a crash mid-turn, for asserting a queued message is reported as
/// undelivered rather than silently dropped.
const DIES_MID_TURN_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
read -r _line
exit 1
"#;

/// A fake CLI that crashes on its *first* turn only (a marker file next to it
/// remembers it already did), then behaves normally — for asserting that a
/// user turn lost to a crash does not leave "the user is waiting" set behind.
const CRASHES_ONCE_THEN_OK_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
if [ ! -f "$(dirname "$0")/crashed" ]; then
  : > "$(dirname "$0")/crashed"
  read -r _line
  exit 1
fi
while IFS= read -r _line; do
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"an agent finished"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
done
"#;

/// A fake CLI that appends every turn's raw stdin line to `stdin.jsonl` next
/// to it, so a test can inspect exactly what content blocks a turn carried —
/// for asserting an attachment reaches the child's stdin intact.
const RECORDS_STDIN_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$(dirname "$0")/stdin.jsonl"
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
done
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

/// The raw `stdin.jsonl` lines `RECORDS_STDIN_CLAUDE` captured, one per turn —
/// waits until at least `at_least` lines have landed.
async fn recorded_stdin(dir: &Path, at_least: usize) -> Vec<serde_json::Value> {
    timeout(TEST_TIMEOUT, async {
        loop {
            if let Ok(raw) = std::fs::read_to_string(dir.join("stdin.jsonl")) {
                let lines: Vec<serde_json::Value> = raw
                    .lines()
                    .map(|l| serde_json::from_str(l).expect("valid json line"))
                    .collect();
                if lines.len() >= at_least {
                    return lines;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fake cli records stdin")
}

/// Watcher timings a test can wait on: everything in milliseconds.
fn fast_proactivity() -> ProactivityConfig {
    ProactivityConfig {
        tick: Duration::from_millis(20),
        idle_after: Duration::from_millis(50),
        // Longer than any test that is not about escalation, so a stuck
        // agent is raised exactly once there.
        escalate_after: Duration::from_secs(60),
        lingering_after: Duration::from_millis(10),
        sweep_every: Duration::from_millis(30),
        // Real minutes, so the default 10-minute external sweep never fires
        // during a millisecond test. The poll tests shrink this themselves.
        poll_unit: Duration::from_secs(60),
    }
}

/// Watcher timings whose external sweep fires almost at once: one "minute" of
/// `poll_interval_minutes` is 10ms here.
fn fast_polling() -> ProactivityConfig {
    ProactivityConfig {
        // Nothing else may speak, or it would be indistinguishable from a poll.
        idle_after: Duration::from_secs(60),
        lingering_after: Duration::from_secs(60 * 60),
        sweep_every: Duration::from_secs(60 * 60),
        poll_unit: Duration::from_millis(10),
        ..fast_proactivity()
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
            AssistantEvent::TurnEnd {
                error: None,
                notify: true,
            },
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
            AssistantEvent::TurnEnd {
                error: None,
                notify: true,
            },
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

    // A live session to nudge.
    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
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
async fn an_agent_sitting_on_a_prompt_is_reported_without_waiting_out_the_timer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let agent = orch
        .add_process(project, agent_spec("sleep 30", dir.path()))
        .await
        .expect("add agent");
    orch.start_process(agent).await.expect("start agent");
    // A silence timer far longer than the test: only the reported prompt can
    // produce a nudge here.
    podium_core::proactivity::spawn(
        &orch,
        ProactivityConfig {
            idle_after: Duration::from_secs(60),
            ..fast_proactivity()
        },
    );
    orch.report_agent_waiting(agent);

    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("fix-login"), "{notice}");
    assert!(notice.contains("sitting on a prompt"), "{notice}");
    orch.shutdown().await;
}

#[tokio::test]
async fn a_stuck_agent_is_reported_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

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
async fn an_agent_still_stuck_later_is_raised_again_asking_for_a_decision() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hello").await.expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let agent = orch
        .add_process(project, agent_spec("sleep 30", dir.path()))
        .await
        .expect("add agent");
    orch.start_process(agent).await.expect("start agent");
    podium_core::proactivity::spawn(
        &orch,
        ProactivityConfig {
            escalate_after: Duration::from_millis(60),
            ..fast_proactivity()
        },
    );

    let first = next_notice(&mut rx).await;
    assert!(first.contains("waiting on input"), "{first}");

    // Nobody answered, so the next pass must ask the assistant to decide
    // rather than put the same question a second time. (Consecutive nudges
    // can be merged into one turn, hence "a notice containing it" rather
    // than "the next notice".)
    let escalated = timeout(TEST_TIMEOUT, async {
        loop {
            let notice = next_notice(&mut rx).await;
            if notice.contains("still blocked") {
                return notice;
            }
        }
    })
    .await
    .expect("the stuck agent is raised again");
    assert!(
        escalated.contains("which one you would pick"),
        "{escalated}"
    );
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

    // Not a word about the exited agent, nothing.
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
    orch.shutdown().await;
}

#[tokio::test]
async fn a_nudge_with_nobody_chatting_starts_the_session_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), FAKE_CLAUDE));
    let mut rx = orch.subscribe_events();

    let project = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    orch.add_todo(project, "Fix the login redirect")
        .expect("add todo");

    // Nobody has chatted, so there is no session — the nudge must start one
    // rather than wait for the user to happen to type. (A fresh session is
    // not a stranger: every app start clears the conversation anyway, and
    // the memory file carries the continuity.)
    podium_core::proactivity::spawn(&orch, fast_proactivity());

    let notice = next_notice(&mut rx).await;
    assert!(notice.contains("Fix the login redirect"), "{notice}");
    // And it really ran a turn, unprompted.
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    orch.shutdown().await;
}

#[tokio::test]
async fn the_day_start_button_sends_a_real_user_turn() {
    let (orch, _dir) = setup(FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    // Nothing has been sent yet: unlike a nudge, this must spawn the session
    // by itself and run a turn — that is the whole point of it being a
    // button and not proactivity.
    orch.assistant_day_start().await.expect("day start");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let first = orch
        .assistant_transcript()
        .into_iter()
        .next()
        .expect("a transcript row");
    assert_eq!(first.role, TranscriptRole::User);
    // Prioritising with a reason, and asking rather than guessing, are what
    // the summary is for.
    assert!(first.text.contains("day-start summary"), "{}", first.text);
    assert!(first.text.contains("picking up first"), "{}", first.text);
    assert!(first.text.contains("ask me"), "{}", first.text);
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

#[tokio::test]
async fn queued_messages_are_delivered_one_per_turn_in_order() {
    let (orch, _dir) = setup(ECHOING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    // Busy the instant `send` returns (in-flight is set before the child
    // even has a chance to answer) — these must queue, not interleave.
    orch.assistant_send("second").await.expect("queue second");
    orch.assistant_send("third").await.expect("queue third");

    let turn1 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn1.contains(&AssistantEvent::Message {
        text: "echo:first".to_string()
    }));
    assert!(
        !turn1.iter().any(|e| matches!(e, AssistantEvent::Message { text } if text.contains("second") || text.contains("third"))),
        "queued messages must not interleave into the running turn: {turn1:?}"
    );

    // The oldest queued message becomes its own turn once the first ends,
    // announced so the UI can promote its waiting row into the ordinary log.
    let turn2 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn2.contains(&AssistantEvent::QueueItemDelivered {
        text: "second".to_string()
    }));
    assert!(turn2.contains(&AssistantEvent::Message {
        text: "echo:second".to_string()
    }));

    // …and the next one after that — never merged together.
    let turn3 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn3.contains(&AssistantEvent::Message {
        text: "echo:third".to_string()
    }));

    orch.shutdown().await;
}

#[tokio::test]
async fn a_turn_that_ends_in_error_still_advances_the_queue() {
    let (orch, _dir) = setup(ERRORS_ONCE_THEN_OK_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    orch.assistant_send("second").await.expect("queue second");

    let turn1 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert_eq!(
        turn1.last(),
        Some(&AssistantEvent::TurnEnd {
            error: Some("error_max_turns".to_string()),
            notify: true,
        })
    );

    // The queue still drains despite the failure — the second message gets
    // its own (successful) turn right after.
    let turn2 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn2.contains(&AssistantEvent::Message {
        text: "recovered".to_string()
    }));
    orch.shutdown().await;
}

#[tokio::test]
async fn interrupt_stops_the_turn_and_delivers_the_queued_message_right_away() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = setup_in(dir.path(), HANGING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("hang please").await.expect("send");
    // Wait for the session id so the respawn below can `--resume` it.
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::Started { .. })).await;

    orch.assistant_send("queued while hanging")
        .await
        .expect("queue while the turn hangs");

    orch.assistant_interrupt().await.expect("interrupt");

    // The interruption itself is a visible chat line, not a silent cut.
    let rows = orch.assistant_transcript();
    assert!(
        rows.iter().any(|r| r.text.contains("interrupted")),
        "{rows:?}"
    );

    // The waiting message is handed over immediately — no `TurnEnd` for the
    // killed turn was ever coming.
    let turn = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn.contains(&AssistantEvent::Message {
        text: "echo:queued while hanging".to_string()
    }));

    orch.shutdown().await;
}

#[tokio::test]
async fn interrupt_with_nothing_running_does_nothing() {
    let (orch, _dir) = setup(FAKE_CLAUDE);
    let mut rx = orch.subscribe_events();

    // No session at all yet: nothing to stop, nothing to report.
    orch.assistant_interrupt().await.expect("interrupt");
    assert!(rx.try_recv().is_err());
    orch.shutdown().await;
}

#[tokio::test]
async fn a_session_that_crashes_mid_turn_reports_the_queued_message_as_undelivered() {
    let (orch, _dir) = setup(DIES_MID_TURN_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    // Busy the moment `send` returns — queues rather than being lost when
    // the session dies without ever finishing the first turn.
    orch.assistant_send("second").await.expect("queue second");

    collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;

    let rows = orch.assistant_transcript();
    assert!(
        rows.iter()
            .any(|r| r.text.contains("second") && r.text.contains("delivered")),
        "the queued message must be reported, not silently dropped: {rows:?}"
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn a_user_turn_lost_to_a_crash_does_not_make_later_nudges_notify() {
    let (orch, _dir) = setup(CRASHES_ONCE_THEN_OK_CLAUDE);
    let mut rx = orch.subscribe_events();

    // The user's turn goes out and the session dies under it — no `TurnEnd`
    // ever arrives for it.
    orch.assistant_send("what is going on?")
        .await
        .expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::Exited)).await;

    // Podium's own next turn is nobody's question: it must stay in the panel.
    orch.assistant_nudge("[Podium] The agent \"fix-login\" finished.".to_string())
        .await;
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    assert_eq!(
        events.last(),
        Some(&AssistantEvent::TurnEnd {
            error: None,
            notify: false,
        }),
        "a nudge after a crashed user turn must not inherit its notification: {events:?}"
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn editing_and_removing_a_waiting_message_changes_what_is_delivered() {
    let (orch, _dir) = setup(ECHOING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    orch.assistant_send("edit me").await.expect("queue");
    orch.assistant_send("remove me").await.expect("queue");

    // Wait for the snapshot that carries both queued messages (the first one
    // published only has "edit me" — "remove me" is queued a moment later).
    let events = collect_until(
        &mut rx,
        |e| matches!(e, AssistantEvent::QueueChanged { items } if items.len() == 2),
    )
    .await;
    let ids = match events.last() {
        Some(AssistantEvent::QueueChanged { items }) => items.clone(),
        other => panic!("expected a queue snapshot with 2 items, got {other:?}"),
    };
    let edit_id = ids[0].id;
    let remove_id = ids[1].id;

    orch.assistant_queue_edit(edit_id, "edited")
        .await
        .expect("edit");
    orch.assistant_queue_remove(remove_id)
        .await
        .expect("remove");

    // First turn's own reply — not under test here, just drained out of the
    // way so the next `collect_until` starts clean.
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    // Only the edited text is delivered — the removed one never is.
    let turn2 = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(turn2.contains(&AssistantEvent::Message {
        text: "echo:edited".to_string()
    }));
    assert!(!turn2
        .iter()
        .any(|e| matches!(e, AssistantEvent::Message { text } if text.contains("remove me"))));

    orch.shutdown().await;
}

#[tokio::test]
async fn send_now_pushes_a_waiting_message_into_the_running_turn() {
    let (orch, _dir) = setup(ECHOING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    // Busy the instant `send` returns, so this one waits.
    orch.assistant_send("second").await.expect("queue second");

    let events = collect_until(
        &mut rx,
        |e| matches!(e, AssistantEvent::QueueChanged { items } if items.len() == 1),
    )
    .await;
    let id = match events.last() {
        Some(AssistantEvent::QueueChanged { items }) => items[0].id,
        other => panic!("expected a queue snapshot with 1 item, got {other:?}"),
    };

    orch.assistant_queue_send_now(id).await.expect("send now");

    let user_rows: Vec<_> = orch
        .assistant_transcript()
        .into_iter()
        .filter(|r| r.role == TranscriptRole::User)
        .map(|r| r.text)
        .collect();
    assert_eq!(
        user_rows,
        vec!["first".to_string(), "second".to_string()],
        "send now must write into the live session, not wait for the turn to end"
    );

    orch.shutdown().await;
}

#[tokio::test]
async fn send_now_refuses_a_message_that_is_not_waiting() {
    let (orch, _dir) = setup(ECHOING_CLAUDE);
    // Nothing is queued: the id cannot match anything.
    let err = orch
        .assistant_queue_send_now(PendingId::new())
        .await
        .expect_err("unknown id refused");
    assert!(matches!(err, CoreError::QueuedMessageNotFound), "{err:?}");
    orch.shutdown().await;
}

#[tokio::test]
async fn a_nudge_sent_while_busy_waits_its_turn() {
    let (orch, _dir) = setup(ECHOING_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    // A nudge is Podium's own input: it queues like everything else, and
    // unlike a user message it has no "send now" escape hatch.
    orch.assistant_nudge("[Podium] an agent finished".to_string())
        .await;

    let events = collect_until(
        &mut rx,
        |e| matches!(e, AssistantEvent::QueueChanged { items } if items.len() == 1),
    )
    .await;
    assert!(
        matches!(
            events.last(),
            Some(AssistantEvent::QueueChanged { items })
                if items[0].text.contains("an agent finished")
        ),
        "{events:?}"
    );

    orch.shutdown().await;
}

#[tokio::test]
async fn nudges_that_piled_up_are_delivered_as_one_turn_ahead_of_the_user() {
    // Podium's own status lines used to go out one per turn, so after a
    // quiet spell the user's next message waited behind the whole backlog.
    let (orch, dir) = setup(RECORDS_STDIN_CLAUDE);
    let mut rx = orch.subscribe_events();

    // Occupy the session so everything after this has to wait.
    orch.assistant_send("first").await.expect("send first");
    orch.assistant_nudge("[Podium] an agent finished".to_string())
        .await;
    orch.assistant_nudge("[Podium] another agent crashed".to_string())
        .await;
    // Repeated verbatim, the way the 6-hourly sweep does: must not stack.
    orch.assistant_nudge("[Podium] an agent finished".to_string())
        .await;
    orch.assistant_send("what is open?").await.expect("queue");

    // First turn, then the merged nudge turn, then the user's own.
    for _ in 0..3 {
        collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    }

    let lines = recorded_stdin(dir.path(), 3).await;
    let text_of = |i: usize| {
        lines[i]
            .pointer("/message/content/0/text")
            .and_then(|t| t.as_str())
            .expect("text block")
            .to_string()
    };
    assert_eq!(text_of(0), "first");
    let merged = text_of(1);
    assert!(merged.contains("an agent finished"), "{merged}");
    assert!(merged.contains("another agent crashed"), "{merged}");
    assert_eq!(
        merged.matches("an agent finished").count(),
        1,
        "the repeated nudge was deduped: {merged}"
    );
    assert_eq!(
        text_of(2),
        "what is open?",
        "the user is not stuck behind it"
    );

    orch.shutdown().await;
}

// -- attachments -------------------------------------------------------

#[tokio::test]
async fn an_image_attachment_reaches_the_childs_stdin_intact() {
    let (orch, dir) = setup(RECORDS_STDIN_CLAUDE);
    let mut rx = orch.subscribe_events();

    let attachments = vec![Attachment::Image {
        name: "shot.png".to_string(),
        media_type: "image/png".to_string(),
        data: "Zm9v".to_string(),
    }];
    orch.assistant_send_with_attachments("what colour is this?", attachments)
        .await
        .expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let lines = recorded_stdin(dir.path(), 1).await;
    let content = lines[0].pointer("/message/content").expect("content");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "what colour is this?");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["source"]["data"], "Zm9v");

    orch.shutdown().await;
}

#[tokio::test]
async fn a_path_attachment_is_passed_through_verbatim() {
    let (orch, dir) = setup(RECORDS_STDIN_CLAUDE);
    let mut rx = orch.subscribe_events();

    let attachments = vec![Attachment::Path {
        path: "/abs/notes.txt".to_string(),
    }];
    orch.assistant_send_with_attachments("look at this", attachments)
        .await
        .expect("send");
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let lines = recorded_stdin(dir.path(), 1).await;
    let text = lines[0]
        .pointer("/message/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text block");
    assert!(text.contains("/abs/notes.txt"), "{text}");

    orch.shutdown().await;
}

#[tokio::test]
async fn an_over_limit_attachment_is_reported_but_the_rest_of_the_message_still_sends() {
    let (orch, dir) = setup(RECORDS_STDIN_CLAUDE);
    let mut rx = orch.subscribe_events();

    let attachments = vec![
        Attachment::Text {
            name: "paste.txt".to_string(),
            text: "x".repeat(200_000),
        },
        Attachment::Path {
            path: "/abs/notes.txt".to_string(),
        },
    ];
    orch.assistant_send_with_attachments("see attached", attachments)
        .await
        .expect("send");

    // The refused attachment is reported as a Podium-owned chat notice...
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    assert!(
        events.iter().any(
            |e| matches!(e, AssistantEvent::QueueNotice { text } if text.contains("paste.txt"))
        ),
        "{events:?}"
    );

    // ...and the rest of the message — including the attachment that was
    // within the limits — still goes out.
    let lines = recorded_stdin(dir.path(), 1).await;
    let text = lines[0]
        .pointer("/message/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text block");
    assert!(text.contains("see attached"), "{text}");
    assert!(text.contains("/abs/notes.txt"), "{text}");
    assert!(!text.contains("paste.txt"), "{text}");

    orch.shutdown().await;
}

#[tokio::test]
async fn a_queued_message_keeps_its_attachment_until_delivered() {
    let (orch, dir) = setup(RECORDS_STDIN_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("first").await.expect("send first");
    let attachments = vec![Attachment::Path {
        path: "/abs/report.pdf".to_string(),
    }];
    orch.assistant_send_with_attachments("second", attachments)
        .await
        .expect("queue second with an attachment");

    // First turn's own reply, drained out of the way.
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;
    // The queued message becomes its own turn once the first ends.
    collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    let lines = recorded_stdin(dir.path(), 2).await;
    let text = lines[1]
        .pointer("/message/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text block");
    assert!(text.contains("second"), "{text}");
    assert!(text.contains("/abs/report.pdf"), "{text}");

    orch.shutdown().await;
}

// -- the external sweep (slow lane) and its silence contract ------------

#[tokio::test]
async fn a_reply_that_is_only_the_sentinel_never_reaches_the_user() {
    let (orch, _dir) = setup(SILENT_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("anything to act on?")
        .await
        .expect("send");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    // No `Message` at all: the turn came and went without a word. That is
    // also what keeps the panel from firing a notification for it.
    assert_eq!(
        events,
        vec![
            AssistantEvent::Started {
                session_id: "fake-session".to_string()
            },
            AssistantEvent::TurnEnd {
                error: None,
                notify: true,
            },
        ]
    );
    // And it is not in the transcript either — only the user's own row.
    let roles: Vec<_> = orch
        .assistant_transcript()
        .into_iter()
        .map(|r| r.role)
        .collect();
    assert_eq!(roles, vec![TranscriptRole::User]);
    orch.shutdown().await;
}

#[tokio::test]
async fn a_reply_that_merely_mentions_the_sentinel_is_a_real_reply() {
    let (orch, _dir) = setup(MENTIONS_SENTINEL_CLAUDE);
    let mut rx = orch.subscribe_events();

    orch.assistant_send("anything to act on?")
        .await
        .expect("send");
    let events = collect_until(&mut rx, |e| matches!(e, AssistantEvent::TurnEnd { .. })).await;

    assert!(
        events.contains(&AssistantEvent::Message {
            text: "Not NOTHING_TO_REPORT: a deploy is waiting on you.".to_string()
        }),
        "{events:?}"
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn the_external_sweep_asks_about_the_enabled_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), RECORDS_STDIN_CLAUDE));

    orch.set_assistant_polling(AssistantPolling {
        interval_minutes: 1,
        slack: true,
        teams: false,
    })
    .expect("set polling");
    // Nobody is chatting: like a nudge, the sweep starts the session itself.
    podium_core::proactivity::spawn(&orch, fast_polling());

    let lines = recorded_stdin(dir.path(), 1).await;
    let text = lines[0]
        .pointer("/message/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text block")
        .to_string();
    assert!(text.contains("Slack"), "{text}");
    assert!(
        !text.contains("Teams"),
        "disabled source asked about: {text}"
    );
    // The silence contract must be spelled out, or a quiet sweep still talks.
    assert!(text.contains("NOTHING_TO_REPORT"), "{text}");
    orch.shutdown().await;
}

#[tokio::test]
async fn the_external_sweep_stays_off_without_a_source_or_without_proactivity() {
    // Both sources off: nothing to look at, so nothing to ask.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), ARGV_RECORDING_CLAUDE));
    orch.set_assistant_polling(AssistantPolling {
        interval_minutes: 1,
        slack: false,
        teams: false,
    })
    .expect("set polling");
    podium_core::proactivity::spawn(&orch, fast_polling());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !dir.path().join("argv.txt").exists(),
        "swept with no source enabled"
    );
    orch.shutdown().await;

    // Sources on, but the user switched Podium's unprompted speaking off.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), ARGV_RECORDING_CLAUDE));
    orch.set_assistant_proactivity(false).expect("switch off");
    orch.set_assistant_polling(AssistantPolling {
        interval_minutes: 1,
        slack: true,
        teams: true,
    })
    .expect("set polling");
    podium_core::proactivity::spawn(&orch, fast_polling());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !dir.path().join("argv.txt").exists(),
        "swept while proactivity is off"
    );
    orch.shutdown().await;
}

#[tokio::test]
async fn a_zero_interval_turns_the_external_sweep_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(setup_in(dir.path(), ARGV_RECORDING_CLAUDE));
    orch.set_assistant_polling(AssistantPolling {
        interval_minutes: 0,
        slack: true,
        teams: true,
    })
    .expect("set polling");
    podium_core::proactivity::spawn(&orch, fast_polling());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !dir.path().join("argv.txt").exists(),
        "swept with the interval set to 0"
    );
    orch.shutdown().await;
}
