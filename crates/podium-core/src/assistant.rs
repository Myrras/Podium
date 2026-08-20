//! The personal assistant: one long-lived headless Claude Code child process
//! Podium talks to over `stream-json`.
//!
//! Unlike agents (PTY-backed, per project, visible in the sidebar) the
//! assistant is a plain piped child — no terminal, workspace-wide, spawned
//! lazily on the first chat message. Podium writes user turns to its stdin as
//! JSON lines and parses its stdout stream into [`AssistantEvent`]s, which are
//! published on the shared [`EventBus`] so the UI renders the conversation.
//!
//! The session talks to Podium over its own MCP endpoint (`/mcp-assistant`),
//! which puts every tool call through the [`AutonomyMode`] gate: gated calls
//! become [`ProposalInfo`]s the user confirms or rejects in the chat, and only
//! then execute. File-editing tools are refused at the CLI so the assistant
//! cannot write code — implementation work is delegated to spawned agents.
//!
//! Not here yet: transcript/memory persistence (phase 3) and proactive nudges
//! (phase 4), so proposals live in memory and are lost on restart.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Mutex as StdMutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

use crate::agent::McpConnectInfo;
use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};
use crate::ids::ProposalId;

/// Tools the assistant is never allowed to run: it delegates implementation
/// work to spawned agents instead of touching the filesystem itself.
const DISALLOWED_TOOLS: &str = "Edit,Write,NotebookEdit,Bash";

const LOCK_POISONED: &str = "assistant config lock poisoned";

/// Longest tool result echoed back to the assistant after a confirmed
/// proposal — enough to see what happened without pasting a whole listing
/// into the next turn.
const MAX_OUTCOME_CHARS: usize = 800;

/// Who the assistant is, appended to its system prompt at spawn. The autonomy
/// mode is deliberately absent: it is enforced server-side and can change
/// between turns, so stating it here could only ever go stale.
const IDENTITY_PROMPT: &str = "\
You are the personal assistant docked in the right-hand panel of Podium, a \
desktop cockpit for AI coding agents. You work across the user's whole \
workspace, not one project: call the podium MCP tools (start with \
list_projects) to see and change to-dos, agents, processes, scratchpads and \
worktrees. Never write or edit code yourself — for implementation work, add a \
to-do describing it and spawn an agent on that to-do, then report back. Some \
actions need the user's confirmation: the tool then answers that a proposal is \
awaiting them, which means a card is shown in the chat — never retry such a \
call, just carry on and you will be told the outcome. Keep answers short and \
concrete; the panel is narrow.";

/// Tools that only read state. Never gated — in any mode.
const READ_ONLY_TOOLS: &[&str] = &[
    "list_projects",
    "list_processes",
    "get_process_status",
    "get_process_output",
    "list_todos",
    "list_scratchpads",
    "list_worktrees",
];

/// Tools that stop, archive or remove something. These are what
/// [`AutonomyMode::ConfirmDestructive`] holds back.
const DESTRUCTIVE_TOOLS: &[&str] = &[
    "stop_process",
    "restart_process",
    "set_todo_archived",
    "set_scratchpad_archived",
    "remove_scratchpad_tag",
    "remove_worktree",
];

/// How much the assistant may do on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
// Kebab-case on the wire, like its neighbours in the settings file
// (`MergeMode`, `RestartPolicy`).
#[serde(rename_all = "kebab-case")]
pub enum AutonomyMode {
    /// Everything runs immediately.
    Full,
    /// Stopping, archiving and removing need a confirmation; the rest runs.
    #[default]
    ConfirmDestructive,
    /// Everything that changes state is proposed first.
    ProposeAll,
}

/// Assistant preferences, persisted alongside the agent settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSettings {
    #[serde(default)]
    pub autonomy: AutonomyMode,
}

/// Whether `tool` needs the user's confirmation under `mode`.
///
/// Unknown tools count as state-changing, so a tool added later is proposed
/// rather than silently allowed in [`AutonomyMode::ProposeAll`]; a new
/// *destructive* tool must be added to [`DESTRUCTIVE_TOOLS`] (the MCP
/// integration test fails until it is classified).
pub fn requires_proposal(mode: AutonomyMode, tool: &str) -> bool {
    match mode {
        AutonomyMode::Full => false,
        AutonomyMode::ConfirmDestructive => DESTRUCTIVE_TOOLS.contains(&tool),
        AutonomyMode::ProposeAll => !READ_ONLY_TOOLS.contains(&tool),
    }
}

/// A gated tool call, parked until the user decides. Running it returns the
/// tool's own result text, or its error message.
pub type ProposalAction =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send>;

/// UI-facing snapshot of a pending proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalInfo {
    pub id: ProposalId,
    /// The MCP tool the assistant wants to run, e.g. `set_todo_archived`.
    pub tool: String,
    /// Compact description of the call (its arguments), so the user can see
    /// what they are approving.
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

/// One parsed item from the assistant's output stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssistantEvent {
    /// The session is up; carries Claude's own session id (phase 3 resumes it).
    Started { session_id: String },
    /// A chunk of assistant prose.
    Message { text: String },
    /// The assistant invoked a tool (name only — arguments stay out of the UI).
    Tool { name: String },
    /// A tool call came back as an error, so a failed action is never silent.
    ToolFailed { message: String },
    /// A tool call was held back by the autonomy gate: it runs only once the
    /// user confirms this proposal in the chat.
    Proposal {
        id: ProposalId,
        tool: String,
        summary: String,
    },
    /// A proposal was confirmed (and executed) or rejected. `error` is set
    /// when the confirmed action itself failed.
    ProposalResolved {
        id: ProposalId,
        approved: bool,
        error: Option<String>,
    },
    /// The turn finished; `error` is set when it ended badly.
    TurnEnd { error: Option<String> },
    /// The session ended (crashed, or the CLI is missing). A later send
    /// starts a fresh one.
    Exited,
}

/// Where the assistant session comes from. Injectable so tests can point at a
/// fake `claude` binary instead of the real CLI.
#[derive(Debug, Clone)]
pub struct AssistantConfig {
    /// Binary speaking Claude Code's headless `stream-json` protocol.
    pub command: String,
    /// Working directory for the child; `None` inherits Podium's own.
    pub cwd: Option<PathBuf>,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            cwd: None,
        }
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
}

/// Owns the assistant session, its lazy (re)spawn, and the proposals its
/// gated tool calls are parked in.
pub struct AssistantManager {
    config: StdMutex<AssistantConfig>,
    session: Mutex<Option<Session>>,
    /// Gated tool calls awaiting the user, oldest first. In memory only: a
    /// proposal never expires on its own, but does not survive a restart
    /// (phase 3 persists the conversation).
    proposals: StdMutex<Vec<(ProposalInfo, ProposalAction)>>,
    events: EventBus,
}

impl AssistantManager {
    pub fn new(events: EventBus) -> Self {
        Self {
            config: StdMutex::new(AssistantConfig::default()),
            session: Mutex::new(None),
            proposals: StdMutex::new(Vec::new()),
            events,
        }
    }

    /// Point the manager at a binary / working directory (the app data dir in
    /// the real app, a fake CLI in tests).
    pub fn configure(&self, config: AssistantConfig) {
        *self.config.lock().expect(LOCK_POISONED) = config;
    }

    /// Send a chat turn, spawning the session on first use (or after it died).
    /// `mcp` is how the new session reaches Podium's own MCP endpoint; a
    /// session spawned before the server is up simply has no Podium tools.
    pub async fn send(&self, text: &str, mcp: Option<McpConnectInfo>) -> CoreResult<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "message must not be blank".to_string(),
            ));
        }
        let mut guard = self.session.lock().await;
        // A session that exited (crash, missing CLI) is replaced rather than
        // written to: the write would only fail with a broken pipe.
        if guard
            .as_mut()
            .is_some_and(|s| matches!(s.child.try_wait(), Ok(Some(_)) | Err(_)))
        {
            *guard = None;
        }
        if guard.is_none() {
            let config = self.config.lock().expect(LOCK_POISONED).clone();
            *guard = Some(self.spawn(&config, mcp.as_ref())?);
        }
        write_turn(guard.as_mut().expect("session present"), text).await
    }

    /// Park a gated tool call and tell the UI about it. Returns the id the
    /// user's confirm/reject refers to.
    pub fn propose(&self, tool: String, summary: String, action: ProposalAction) -> ProposalId {
        let info = ProposalInfo {
            id: ProposalId::new(),
            tool,
            summary,
            created_at: Utc::now(),
        };
        let id = info.id;
        self.events.publish(PodiumEvent::AssistantEvent {
            event: AssistantEvent::Proposal {
                id,
                tool: info.tool.clone(),
                summary: info.summary.clone(),
            },
        });
        self.proposals
            .lock()
            .expect(LOCK_POISONED)
            .push((info, action));
        id
    }

    /// The proposals still awaiting the user, oldest first.
    pub fn proposals(&self) -> Vec<ProposalInfo> {
        self.proposals
            .lock()
            .expect(LOCK_POISONED)
            .iter()
            .map(|(info, _)| info.clone())
            .collect()
    }

    /// Confirm (`approved`) or reject one proposal. A confirmed proposal runs
    /// its stored tool call now; either way the assistant is told the outcome
    /// so it can carry on, and the UI gets a `ProposalResolved`.
    pub async fn resolve_proposal(&self, id: ProposalId, approved: bool) -> CoreResult<()> {
        // Take it out under the lock — the action is then run without holding
        // it, and a double confirm finds nothing left.
        let taken = {
            let mut pending = self.proposals.lock().expect(LOCK_POISONED);
            let at = pending
                .iter()
                .position(|(info, _)| info.id == id)
                .ok_or(CoreError::ProposalNotFound)?;
            pending.remove(at)
        };
        let (info, action) = taken;
        let outcome = if approved { Some(action().await) } else { None };
        let error = match &outcome {
            Some(Err(message)) => Some(message.clone()),
            _ => None,
        };
        self.events.publish(PodiumEvent::AssistantEvent {
            event: AssistantEvent::ProposalResolved {
                id,
                approved,
                error,
            },
        });
        self.notify(&outcome_notice(&info.tool, outcome)).await;
        Ok(())
    }

    /// Write a Podium-side message into a *live* session. Unlike [`send`] this
    /// never spawns one: a fresh session would have no memory of what the
    /// notice is about.
    ///
    /// [`send`]: Self::send
    async fn notify(&self, text: &str) {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return;
        };
        if let Err(e) = write_turn(session, text).await {
            tracing::warn!("could not notify the assistant: {e}");
        }
    }

    /// Kill the session, if any (app shutdown).
    pub async fn shutdown(&self) {
        if let Some(mut session) = self.session.lock().await.take() {
            let _ = session.child.kill().await;
        }
    }

    fn spawn(&self, config: &AssistantConfig, mcp: Option<&McpConnectInfo>) -> CoreResult<Session> {
        // Run through the login shell like agent launches do, so a `claude`
        // installed by nvm/homebrew resolves even when Podium was started
        // from Finder with a bare PATH. `exec` replaces the shell, so the
        // child we hold IS the CLI — killing it needs no process group.
        let (shell, prefix) = crate::platform::login_shell();
        // Its own endpoint, not the agents' one: that is what puts the
        // assistant's tool calls through the autonomy gate. The user's own MCP
        // servers (mail, Slack, …) stay available — this config only adds to
        // them.
        let mcp_config = mcp
            .map(crate::agent::write_assistant_mcp_config)
            .transpose()?
            .map(|path| {
                Ok::<String, CoreError>(format!(
                    " --mcp-config {}",
                    crate::platform::quote_arg(&path.to_string_lossy())?
                ))
            })
            .transpose()?
            .unwrap_or_default();
        let command = format!(
            "exec {} -p --input-format stream-json --output-format stream-json \
             --verbose --disallowedTools {}{} --append-system-prompt {}",
            crate::platform::quote_arg(&config.command)?,
            DISALLOWED_TOOLS,
            mcp_config,
            crate::platform::quote_arg(IDENTITY_PROMPT)?,
        );
        let mut cmd = Command::new(shell);
        cmd.args(prefix)
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // ponytail: stderr dropped — the repo forbids logging child
            // output, and a dead session already surfaces as `Exited`.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            // Non-JSON noise (a chatty shell profile) is skipped, not fatal.
            while let Ok(Some(line)) = lines.next_line().await {
                for event in parse_stream_line(&line) {
                    events.publish(PodiumEvent::AssistantEvent { event });
                }
            }
            events.publish(PodiumEvent::AssistantEvent {
                event: AssistantEvent::Exited,
            });
        });
        Ok(Session { child, stdin })
    }
}

/// Parse one line of Claude Code's `stream-json` output. One line can carry
/// several events (an assistant message with prose plus a tool call);
/// anything unrecognised yields none.
pub fn parse_stream_line(line: &str) -> Vec<AssistantEvent> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("system") if value.get("subtype").and_then(|s| s.as_str()) == Some("init") => value
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(|session_id| {
                vec![AssistantEvent::Started {
                    session_id: session_id.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("assistant") => value
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .map(|blocks| blocks.iter().filter_map(content_block_event).collect())
            .unwrap_or_default(),
        // Tool results come back as a `user` message; only failures are
        // interesting here, so a failed action is never silent in the chat.
        Some("user") => value
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .map(|blocks| blocks.iter().filter_map(tool_failure_event).collect())
            .unwrap_or_default(),
        Some("result") => {
            let failed = value.get("is_error").and_then(|e| e.as_bool()) == Some(true);
            let error = failed.then(|| {
                value
                    .get("subtype")
                    .and_then(|s| s.as_str())
                    .unwrap_or("the assistant turn failed")
                    .to_string()
            });
            vec![AssistantEvent::TurnEnd { error }]
        }
        _ => Vec::new(),
    }
}

fn content_block_event(block: &serde_json::Value) -> Option<AssistantEvent> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => block
            .get("text")
            .and_then(|t| t.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(|t| AssistantEvent::Message {
                text: t.to_string(),
            }),
        Some("tool_use") => {
            block
                .get("name")
                .and_then(|n| n.as_str())
                .map(|name| AssistantEvent::Tool {
                    name: name.to_string(),
                })
        }
        _ => None,
    }
}

/// A `tool_result` block that reports an error, as one [`AssistantEvent`].
fn tool_failure_event(block: &serde_json::Value) -> Option<AssistantEvent> {
    if block.get("type").and_then(|t| t.as_str()) != Some("tool_result")
        || block.get("is_error").and_then(|e| e.as_bool()) != Some(true)
    {
        return None;
    }
    Some(AssistantEvent::ToolFailed {
        message: tool_result_text(block.get("content")),
    })
}

/// The human-readable part of a `tool_result` content field, which is either a
/// plain string or a list of content blocks.
fn tool_result_text(content: Option<&serde_json::Value>) -> String {
    let text = match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    let text = text.trim();
    if text.is_empty() {
        "the action failed".to_string()
    } else {
        truncate(text, MAX_OUTCOME_CHARS)
    }
}

/// Write one turn as a `stream-json` user message.
async fn write_turn(session: &mut Session, text: &str) -> CoreResult<()> {
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    });
    session
        .stdin
        .write_all(format!("{line}\n").as_bytes())
        .await?;
    session.stdin.flush().await?;
    Ok(())
}

/// What the assistant is told once the user decided on a proposal. `None`
/// means it was rejected.
fn outcome_notice(tool: &str, outcome: Option<Result<String, String>>) -> String {
    match outcome {
        None => format!(
            "[Podium] The user rejected your proposed `{tool}` action. Do not retry it; \
             ask what they would like instead."
        ),
        Some(Ok(result)) => format!(
            "[Podium] The user confirmed your proposed `{tool}` action and Podium ran it. \
             Result: {}",
            truncate(result.trim(), MAX_OUTCOME_CHARS)
        ),
        Some(Err(message)) => format!(
            "[Podium] The user confirmed your proposed `{tool}` action but it failed: {}. \
             Tell them what went wrong.",
            truncate(message.trim(), MAX_OUTCOME_CHARS)
        ),
    }
}

/// Clip `text` to at most `max` characters (never mid-`char`).
fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((cut, _)) => format!("{}…", &text[..cut]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_line_yields_the_session_id() {
        let events = parse_stream_line(
            r#"{"type":"system","subtype":"init","session_id":"sess-1","tools":[]}"#,
        );
        assert_eq!(
            events,
            vec![AssistantEvent::Started {
                session_id: "sess-1".to_string()
            }]
        );
    }

    #[test]
    fn assistant_line_splits_prose_and_tool_calls() {
        let events = parse_stream_line(
            r#"{"type":"assistant","message":{"content":[
                {"type":"text","text":"Making a to-do"},
                {"type":"tool_use","name":"add_todo","input":{"text":"x"}},
                {"type":"thinking","thinking":"hidden"}
            ]}}"#,
        );
        assert_eq!(
            events,
            vec![
                AssistantEvent::Message {
                    text: "Making a to-do".to_string()
                },
                AssistantEvent::Tool {
                    name: "add_todo".to_string()
                },
            ]
        );
    }

    #[test]
    fn blank_text_blocks_are_dropped() {
        let events = parse_stream_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"  "}]}}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn successful_result_ends_the_turn_without_error() {
        let events = parse_stream_line(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#,
        );
        assert_eq!(events, vec![AssistantEvent::TurnEnd { error: None }]);
    }

    #[test]
    fn failed_result_carries_the_failure_subtype() {
        let events =
            parse_stream_line(r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#);
        assert_eq!(
            events,
            vec![AssistantEvent::TurnEnd {
                error: Some("error_max_turns".to_string())
            }]
        );
    }

    #[test]
    fn noise_and_unknown_events_are_ignored() {
        assert!(parse_stream_line("zsh: profile chatter").is_empty());
        assert!(parse_stream_line(r#"{"type":"stream_event","event":{}}"#).is_empty());
        assert!(parse_stream_line(r#"{"type":"user","message":{}}"#).is_empty());
    }

    #[test]
    fn events_serialize_camel_case_tagged_by_type() {
        let json = serde_json::to_value(AssistantEvent::Started {
            session_id: "s".to_string(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "started", "sessionId": "s" })
        );
        let json = serde_json::to_value(AssistantEvent::TurnEnd { error: None }).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "turnEnd", "error": null })
        );
    }

    #[tokio::test]
    async fn blank_messages_are_rejected() {
        let manager = AssistantManager::new(EventBus::new());
        let err = manager.send("   ", None).await.expect_err("blank rejected");
        assert!(matches!(err, CoreError::InvalidInput(_)));
    }

    #[test]
    fn tool_result_errors_surface_as_a_failure() {
        let events = parse_stream_line(
            r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","is_error":true,
                 "content":[{"type":"text","text":"to-do not found"}]}
            ]}}"#,
        );
        assert_eq!(
            events,
            vec![AssistantEvent::ToolFailed {
                message: "to-do not found".to_string()
            }]
        );
        // A successful tool result is bookkeeping, not chat content.
        assert!(parse_stream_line(
            r#"{"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#
        )
        .is_empty());
    }

    #[test]
    fn full_autonomy_never_proposes() {
        for tool in ["list_todos", "add_todo", "stop_process", "remove_worktree"] {
            assert!(!requires_proposal(AutonomyMode::Full, tool), "{tool}");
        }
    }

    #[test]
    fn confirm_destructive_gates_only_destructive_tools() {
        let mode = AutonomyMode::ConfirmDestructive;
        assert!(requires_proposal(mode, "stop_process"));
        assert!(requires_proposal(mode, "set_todo_archived"));
        assert!(requires_proposal(mode, "remove_worktree"));
        assert!(!requires_proposal(mode, "add_todo"));
        assert!(!requires_proposal(mode, "spawn_agent"));
        assert!(!requires_proposal(mode, "list_projects"));
    }

    #[test]
    fn propose_all_gates_everything_that_changes_state() {
        let mode = AutonomyMode::ProposeAll;
        assert!(requires_proposal(mode, "add_todo"));
        assert!(requires_proposal(mode, "stop_process"));
        // An unknown (newly added) tool counts as state-changing.
        assert!(requires_proposal(mode, "future_tool"));
        for tool in READ_ONLY_TOOLS {
            assert!(!requires_proposal(mode, tool), "{tool}");
        }
    }

    #[test]
    fn autonomy_mode_is_kebab_case_on_the_wire() {
        let json = serde_json::to_string(&AutonomyMode::ConfirmDestructive).unwrap();
        assert_eq!(json, r#""confirm-destructive""#);
        assert_eq!(AutonomyMode::default(), AutonomyMode::ConfirmDestructive);
    }

    /// A proposal whose action records that it ran, and returns `result`.
    fn action(
        ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
        result: Result<String, String>,
    ) -> ProposalAction {
        Box::new(move || {
            Box::pin(async move {
                ran.store(true, std::sync::atomic::Ordering::SeqCst);
                result
            })
        })
    }

    #[tokio::test]
    async fn confirming_a_proposal_runs_it_once() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let manager = AssistantManager::new(EventBus::new());
        let ran = std::sync::Arc::new(AtomicBool::new(false));
        let id = manager.propose(
            "set_todo_archived".to_string(),
            "{\"todoId\":\"t-1\"}".to_string(),
            action(ran.clone(), Ok("archived".to_string())),
        );
        assert_eq!(manager.proposals().len(), 1);

        manager.resolve_proposal(id, true).await.expect("confirmed");
        assert!(ran.load(Ordering::SeqCst));
        assert!(manager.proposals().is_empty());
        // A second decision on the same proposal has nothing left to run.
        let err = manager
            .resolve_proposal(id, true)
            .await
            .expect_err("gone after resolving");
        assert!(matches!(err, CoreError::ProposalNotFound));
    }

    #[tokio::test]
    async fn rejecting_a_proposal_drops_it_without_running() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();
        let ran = std::sync::Arc::new(AtomicBool::new(false));
        let id = manager.propose(
            "stop_process".to_string(),
            "{}".to_string(),
            action(ran.clone(), Ok("stopped".to_string())),
        );
        manager.resolve_proposal(id, false).await.expect("rejected");

        assert!(!ran.load(Ordering::SeqCst));
        assert!(manager.proposals().is_empty());
        let mut seen = Vec::new();
        while let Ok(PodiumEvent::AssistantEvent { event }) = rx.try_recv() {
            seen.push(event);
        }
        assert_eq!(
            seen,
            vec![
                AssistantEvent::Proposal {
                    id,
                    tool: "stop_process".to_string(),
                    summary: "{}".to_string(),
                },
                AssistantEvent::ProposalResolved {
                    id,
                    approved: false,
                    error: None,
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_confirmed_action_that_fails_reports_the_reason() {
        use std::sync::atomic::AtomicBool;

        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();
        let id = manager.propose(
            "remove_worktree".to_string(),
            "{}".to_string(),
            action(
                std::sync::Arc::new(AtomicBool::new(false)),
                Err("worktree has uncommitted changes".to_string()),
            ),
        );
        manager.resolve_proposal(id, true).await.expect("resolved");

        let mut seen = Vec::new();
        while let Ok(PodiumEvent::AssistantEvent { event }) = rx.try_recv() {
            seen.push(event);
        }
        assert!(seen.contains(&AssistantEvent::ProposalResolved {
            id,
            approved: true,
            error: Some("worktree has uncommitted changes".to_string()),
        }));
    }

    #[test]
    fn the_outcome_notice_tells_the_assistant_what_happened() {
        assert!(outcome_notice("add_todo", None).contains("rejected"));
        assert!(
            outcome_notice("add_todo", Some(Ok("{\"id\":\"t-1\"}".to_string())))
                .contains("confirmed")
        );
        assert!(outcome_notice("add_todo", Some(Err("nope".to_string()))).contains("failed"));
    }

    #[test]
    fn truncate_clips_on_a_char_boundary() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("héllo", 3), "hél…");
    }
}
