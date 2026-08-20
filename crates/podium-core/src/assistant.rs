//! The personal assistant: one long-lived headless Claude Code child process
//! Podium talks to over `stream-json`.
//!
//! Unlike agents (PTY-backed, per project, visible in the sidebar) the
//! assistant is a plain piped child — no terminal, workspace-wide, spawned
//! lazily on the first chat message. Podium writes user turns to its stdin as
//! JSON lines and parses its stdout stream into [`AssistantEvent`]s, which are
//! published on the shared [`EventBus`] so the UI renders the conversation.
//!
//! Phase 1 is the chat slice only: no Podium MCP tools, no proposals, no
//! persistence. File-editing tools are refused at the CLI so the assistant
//! cannot write code even before those layers land.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex as StdMutex;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};

/// Tools the assistant is never allowed to run: it delegates implementation
/// work to spawned agents instead of touching the filesystem itself.
const DISALLOWED_TOOLS: &str = "Edit,Write,NotebookEdit,Bash";

const LOCK_POISONED: &str = "assistant config lock poisoned";

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

/// Owns the assistant session and its lazy (re)spawn.
pub struct AssistantManager {
    config: StdMutex<AssistantConfig>,
    session: Mutex<Option<Session>>,
    events: EventBus,
}

impl AssistantManager {
    pub fn new(events: EventBus) -> Self {
        Self {
            config: StdMutex::new(AssistantConfig::default()),
            session: Mutex::new(None),
            events,
        }
    }

    /// Point the manager at a binary / working directory (the app data dir in
    /// the real app, a fake CLI in tests).
    pub fn configure(&self, config: AssistantConfig) {
        *self.config.lock().expect(LOCK_POISONED) = config;
    }

    /// Send a chat turn, spawning the session on first use (or after it died).
    pub async fn send(&self, text: &str) -> CoreResult<()> {
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
            *guard = Some(self.spawn(&config)?);
        }
        let session = guard.as_mut().expect("session present");
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

    /// Kill the session, if any (app shutdown).
    pub async fn shutdown(&self) {
        if let Some(mut session) = self.session.lock().await.take() {
            let _ = session.child.kill().await;
        }
    }

    fn spawn(&self, config: &AssistantConfig) -> CoreResult<Session> {
        // Run through the login shell like agent launches do, so a `claude`
        // installed by nvm/homebrew resolves even when Podium was started
        // from Finder with a bare PATH. `exec` replaces the shell, so the
        // child we hold IS the CLI — killing it needs no process group.
        let (shell, prefix) = crate::platform::login_shell();
        let command = format!(
            "exec {} -p --input-format stream-json --output-format stream-json \
             --verbose --disallowedTools {}",
            crate::platform::quote_arg(&config.command)?,
            DISALLOWED_TOOLS,
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
        let err = manager.send("   ").await.expect_err("blank rejected");
        assert!(matches!(err, CoreError::InvalidInput(_)));
    }
}
