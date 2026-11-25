//! The external sweep's own session: the every-`poll_interval_minutes`
//! Slack/Teams "Routine check" runs on Haiku in a headless Claude Code child
//! of its own, never in the assistant's conversation.
//!
//! A plain piped child like the assistant's (`claude -p --input-format
//! stream-json --output-format stream-json`), spawned through the same login
//! shell, **fresh for every sweep** and gone after it: the sweep is stateless,
//! each one asks for the full set of messages still waiting for the user, so
//! nothing has to carry over (and a day of sweeps never piles up in one
//! context). It is passed no
//! Podium `--mcp-config` — the user's own user-scope MCP servers (Slack,
//! Teams) are all it needs, and `--strict-mcp-config` is deliberately not
//! passed — but that does not keep the user's own `podium mcp-bridge` server
//! out (it is user-scope too, and its input here is untrusted chat text), so
//! the `podium` MCP server is denied outright. The file-edit, shell, web and
//! messaging tools are refused at the CLI, only the user's own settings load,
//! the session runs in an empty directory of its own, and Podium's MCP config
//! dir (bearer token, caller keys) is unreadable to it.
//!
//! Each sweep is one turn: [`poll_prompt`](crate::assistant::poll_prompt) in,
//! the turn's closing JSON out, parsed here ([`parse_messages`]) into
//! [`WaitingMessage`]s that [`crate::proactivity`] files on the Responses
//! list. Nothing of it reaches the chat: nothing here publishes an event or
//! records a transcript row. An answer that cannot be read counts as a failed
//! sweep and files (or removes) nothing.
//!
//! Only one sweep runs at a time ([`SweepManager::begin`]); a turn running
//! longer than [`SweepConfig::turn_timeout`] kills the child. A check that
//! keeps failing (the CLI missing or logged out, a broken MCP server, an
//! unreadable answer) would look exactly like "nothing waiting", so
//! [`FAILURES_BEFORE_NOTICE`] failed sweeps in a row hand the assistant one
//! notice ([`SweepOutcome::NotWorking`]), then stay quiet until a sweep has
//! worked again.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Notify;

use crate::assistant::{
    parse_stream_line, poll_sources, truncate, AssistantEvent, AssistantPolling,
};
use crate::error::CoreResult;
use crate::events::{EventBus, PodiumEvent};

/// The model the sweep runs on: reading a few channels and judging whether
/// anything needs the user is a small job, and it runs every few minutes.
pub const SWEEP_MODEL: &str = "haiku";

/// Refused at the CLI regardless of permission mode: the sweep only reads
/// messages, and only through its Slack/Teams MCP servers. `AskUserQuestion`
/// too — nobody is there to answer it — the web tools, which an injected
/// message could use to carry data out, and every local file reader, which
/// it has no use for.
const SWEEP_DISALLOWED_CLI_TOOLS: &str = "Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task,\
     SendMessage,AskUserQuestion,WebFetch,WebSearch,Read,Grep,Glob,LS";

/// The user's own `podium mcp-bridge` MCP server is user-scope, so the sweep
/// loads it (no `--strict-mcp-config`, or Slack/Teams would be hidden too) and
/// it serves the agents' route: spawning, stopping and removing agents,
/// ungated, on the say-so of untrusted chat text. `mcp__podium` is the CLI's
/// server-level rule (every tool of that server).
const SWEEP_DENIED_MCP_SERVER: &str = "mcp__podium";

/// Everything the sweep is refused, as a `--disallowedTools` value:
/// [`SWEEP_DISALLOWED_CLI_TOOLS`], the whole [`SWEEP_DENIED_MCP_SERVER`] and,
/// as a second layer should the server-level form not match, `mcp__podium__
/// <tool>` for every tool of Podium's server, generated from its router like a
/// lookup sub-agent's list ([`crate::subagent::lookup_disallowed_tools`]).
/// A bridge registered under another name is not covered by either.
fn sweep_disallowed_tools() -> String {
    std::iter::once(SWEEP_DISALLOWED_CLI_TOOLS.to_string())
        .chain(std::iter::once(SWEEP_DENIED_MCP_SERVER.to_string()))
        .chain(
            crate::mcp::tools::PodiumTools::all_tool_names()
                .into_iter()
                .map(|tool| format!("{SWEEP_DENIED_MCP_SERVER}__{tool}")),
        )
        .collect::<Vec<_>>()
        .join(",")
}

/// The sweep's own empty working directory, inside the app data dir.
const SWEEP_WORKSPACE: &str = "sweep-workspace";

/// Failed sweeps in a row before the assistant is told the check is down.
pub const FAILURES_BEFORE_NOTICE: u32 = 3;

/// Why a sweep failed, as Podium-owned text for the notice: never CLI output.
const FAIL_START: &str = "the CLI could not be started";
const FAIL_ENDED: &str = "the session ended without answering";
const FAIL_TIMEOUT: &str = "it took too long and was restarted";
const FAIL_ERRORED: &str = "the CLI answered with an error";
const FAIL_UNREADABLE: &str = "it could not read a source, or its answer was not readable";

/// How long one sweep turn may take before its child is killed.
pub const SWEEP_TURN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// How long a session gets to exit by itself once its stdin closes (so the
/// CLI can stop its MCP servers) before it is killed.
const SESSION_EXIT_GRACE: Duration = Duration::from_secs(5);

/// How far back a waiting message counts, in days. The prompt asks for the
/// same window; an older entry is dropped here whatever the model returns.
pub const MESSAGE_WINDOW_DAYS: i64 = 2;

/// Longest conversation/thread id accepted (chars): they go into the item
/// key, and a real Slack/Teams id is far shorter.
const MAX_ID_CHARS: usize = 200;

/// The sweep session's identity. It is told that nobody but Podium reads it,
/// so it neither greets nor explains, and that it may only read.
const SWEEP_IDENTITY: &str = "You are Podium's routine checker. Every few minutes Podium \
asks you to look at the user's Slack and/or Microsoft Teams, through the MCP servers you \
have, and list every message still waiting for the user. You are strictly read-only: never \
send, reply, react, mark as read, edit, delete or change anything anywhere — only read. \
Nobody but Podium reads your replies, and a program parses them. Only your final message is \
read: put the whole answer in it, never an earlier part of the turn. That final message is \
exactly the JSON object the check asks for and nothing else — no code fence, no prose. If a \
source cannot be read (no tools for it, not signed in), reply with one plain line saying \
which source is unreachable instead of JSON. This reply format overrides any reply-format or \
style instructions from CLAUDE.md or other instructions: no TL;DR, ledger or preamble.";

/// Where the sweep session comes from. Injectable so tests can point at a
/// fake `claude` and shrink the timeout; the app sets the command and cwd
/// together with the assistant's ([`crate::Orchestrator::configure_assistant`]).
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// Binary speaking Claude Code's headless `stream-json` protocol.
    pub command: String,
    /// Working directory for the child; `None` inherits Podium's own. Created
    /// when missing, and kept empty: the sweep has no business in any
    /// directory with something in it.
    pub cwd: Option<PathBuf>,
    /// Podium's MCP config dir (bearer token, caller keys), which the child
    /// is denied reading; `None` adds no such rule.
    pub mcp_dir: Option<PathBuf>,
    /// How long one sweep turn may take before the child is killed.
    pub turn_timeout: Duration,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            cwd: None,
            mcp_dir: None,
            turn_timeout: SWEEP_TURN_TIMEOUT,
        }
    }
}

/// One message the sweep found still waiting for the user, as parsed from
/// its JSON answer. Filed as Responses item [`Self::key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingMessage {
    /// `slack` or `teams`.
    pub source: &'static str,
    pub conversation: String,
    /// The thread id; the conversation id when the message is not in a thread.
    pub thread: String,
    pub sender: String,
    /// Where it is, as the user reads it ("Slack, direct message").
    pub place: String,
    /// The newest message in it: the item's fingerprint, so a follow-up
    /// brings a dismissed item back.
    pub newest: DateTime<Utc>,
    /// A short gist.
    pub text: String,
}

impl WaitingMessage {
    pub fn key(&self) -> String {
        format!("msg:{}:{}:{}", self.source, self.conversation, self.thread)
    }
}

/// One entry of the sweep's answer, as the model wrote it.
#[derive(Deserialize)]
struct RawMessage {
    source: String,
    conversation: String,
    #[serde(default)]
    thread: Option<String>,
    sender: String,
    place: String,
    newest: String,
    text: String,
}

/// The sweep's answer as messages still waiting for the user, newer than
/// [`MESSAGE_WINDOW_DAYS`] before `now`. `None` when the answer as a whole
/// cannot be read: then nothing may be filed or removed. A malformed entry
/// is dropped on its own (logged without content).
pub(crate) fn parse_messages(reply: &str, now: DateTime<Utc>) -> Option<Vec<WaitingMessage>> {
    // Haiku wraps JSON in a code fence or a line of prose despite the prompt.
    let json = reply.get(reply.find('{')?..=reply.rfind('}')?)?;
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let entries = value.get("messages")?.as_array()?;
    let cutoff = now - chrono::Duration::days(MESSAGE_WINDOW_DAYS);
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let message = waiting_message(entry);
                if message.is_none() {
                    tracing::debug!("dropped a malformed entry from the routine check");
                }
                message
            })
            .filter(|m| m.newest >= cutoff)
            .collect(),
    )
}

fn waiting_message(entry: &serde_json::Value) -> Option<WaitingMessage> {
    let raw = RawMessage::deserialize(entry).ok()?;
    let source = match raw.source.trim().to_ascii_lowercase().as_str() {
        "slack" => "slack",
        "teams" => "teams",
        _ => return None,
    };
    let id = |s: &str| {
        let s = s.trim();
        (!s.is_empty() && s.chars().count() <= MAX_ID_CHARS).then(|| s.to_string())
    };
    let text = |s: &str, max: usize| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        (!s.is_empty()).then(|| truncate(&s, max))
    };
    let conversation = id(&raw.conversation)?;
    let thread = match raw.thread.as_deref().map(str::trim) {
        None | Some("") => conversation.clone(),
        Some(thread) => id(thread)?,
    };
    Some(WaitingMessage {
        source,
        conversation,
        thread,
        sender: text(&raw.sender, 40)?,
        place: text(&raw.place, 40)?,
        newest: DateTime::parse_from_rfc3339(raw.newest.trim())
            .ok()?
            .with_timezone(&Utc),
        text: text(&raw.text, 300)?,
    })
}

/// What the assistant is told when the check has been failing. Podium's own
/// text only: `reason` is one of the `FAIL_*` strings, never CLI output.
pub fn not_working_text(polling: AssistantPolling, reason: &str) -> String {
    format!(
        "[Podium] The routine check of {} is not working: {FAILURES_BEFORE_NOTICE} sweeps in a \
         row failed ({reason}), so nothing is being checked. Tell the user once, following \
         your Responses rules; they can check that the claude CLI is installed and logged in \
         and that the Slack/Teams MCP servers work. Podium says nothing more about it until a \
         check has worked again.",
        poll_sources(polling),
    )
}

/// How one sweep ended, as far as the watcher is concerned.
#[derive(Debug, PartialEq, Eq)]
pub enum SweepOutcome {
    /// Nothing to act on: the sweep failed (not yet a notice) or was cut
    /// short by shutdown. Files and removes nothing.
    Quiet,
    /// The sweep worked: every message still waiting for the user (maybe
    /// none).
    Messages(Vec<WaitingMessage>),
    /// [`FAILURES_BEFORE_NOTICE`] sweeps in a row failed, for this reason
    /// (told once per streak).
    NotWorking(&'static str),
}

/// Consecutive failed sweeps, and whether the streak has been told yet.
#[derive(Debug, Default)]
struct FailureStreak {
    failures: u32,
    told: bool,
}

impl FailureStreak {
    /// Record a sweep: `None` when it worked, else why it failed. Returns the
    /// reason exactly when this failure is the one to tell the assistant of.
    fn record(&mut self, failure: Option<&'static str>) -> Option<&'static str> {
        let Some(reason) = failure else {
            *self = Self::default();
            return None;
        };
        self.failures = self.failures.saturating_add(1);
        if self.failures >= FAILURES_BEFORE_NOTICE && !self.told {
            self.told = true;
            return Some(reason);
        }
        None
    }
}

impl FailureStreak {
    /// The notice [`Self::record`] returned was not delivered (the settings
    /// changed during the sweep): tell it again at the next failure instead
    /// of staying quiet about a check that is still down.
    fn rearm(&mut self) {
        self.told = false;
    }
}

/// How one attempt at a sweep ended, before the streak is consulted.
enum Attempt {
    Done(Option<String>),
    Failed(&'static str),
    /// Shutdown: neither a finding nor a failure.
    Cut,
}

/// How one turn ended, when the session survived it.
enum Turn {
    Done(Option<String>),
    /// The CLI ended the turn with an error (logged out, usage limit, a
    /// broken MCP server).
    Errored,
}

/// The command line the sweep's login shell runs.
fn sweep_command(config: &SweepConfig) -> CoreResult<String> {
    let quote = crate::platform::quote_arg;
    // Only the user's own settings: no project or local settings, hooks or
    // CLAUDE.md files, whatever the working directory holds.
    //
    // The file readers are denied outright; this `--settings` deny is a
    // second layer keeping them out of Podium's MCP config dir, which holds
    // the bearer token and every caller key, should a reader ever be let back.
    let settings = config
        .mcp_dir
        .as_deref()
        .map(|dir| {
            let dir = crate::platform::backend_path_in(crate::platform::wsl(), dir)?;
            let rule = format!("Read({}/**)", crate::agent::absolute_path_rule(&dir));
            let json = serde_json::json!({ "permissions": { "deny": [rule] } }).to_string();
            CoreResult::Ok(format!(" --settings {}", quote(&json)?))
        })
        .transpose()?
        .unwrap_or_default();
    // bypassPermissions: `-p` mode has no TTY for a prompt, so an MCP read
    // would otherwise stall the turn until the timeout. The deny list still
    // wins over it.
    Ok(format!(
        "exec {} -p --input-format stream-json --output-format stream-json --verbose \
         --model {SWEEP_MODEL} --setting-sources user --permission-mode bypassPermissions \
         --disallowedTools {}{settings} --append-system-prompt {}",
        quote(&config.command)?,
        sweep_disallowed_tools(),
        quote(SWEEP_IDENTITY)?,
    ))
}

/// What a stream-json `result` line says about the turn's end.
struct ResultEvent {
    /// Its `result` text: the closing message, when the CLI sent one.
    text: Option<String>,
    /// The turn ended normally (`subtype` `success`) rather than at the turn
    /// limit or in an error.
    success: bool,
}

/// `line` as a `result` event, if it is one.
fn result_event(line: &str) -> Option<ResultEvent> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type").and_then(|t| t.as_str()) != Some("result") {
        return None;
    }
    Some(ResultEvent {
        text: value
            .get("result")
            .and_then(|r| r.as_str())
            .map(str::to_string),
        success: value.get("subtype").and_then(|s| s.as_str()) == Some("success"),
    })
}

/// The turn's closing text: the `result` event's, even when empty. Only a
/// normal end that carried no `result` text falls back to the last prose
/// block; a turn cut off (turn limit) yields nothing.
fn closing_text(closing: Option<ResultEvent>, last_text: Option<String>) -> Option<String> {
    match closing {
        Some(ResultEvent {
            text: Some(text), ..
        }) => Some(text),
        Some(ResultEvent { success: false, .. }) => None,
        _ => last_text,
    }
}

struct SweepSession {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

/// Owns the sweep's one-at-a-time rule and failure streak.
pub struct SweepManager {
    config: StdMutex<SweepConfig>,
    /// A sweep is running; a tick that finds this set is skipped.
    busy: AtomicBool,
    /// Set at shutdown: no further sweep starts.
    closed: AtomicBool,
    /// Failed sweeps in a row, for the one "not working" notice.
    streak: StdMutex<FailureStreak>,
    /// Wakes a running turn at shutdown, so its child dies now rather than
    /// at the timeout.
    stop: Notify,
}

impl Default for SweepManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SweepManager {
    pub fn new() -> Self {
        Self {
            config: StdMutex::new(SweepConfig::default()),
            busy: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            streak: StdMutex::new(FailureStreak::default()),
            stop: Notify::new(),
        }
    }

    pub fn configure(&self, config: SweepConfig) {
        *self.config.lock().expect("sweep config lock") = config;
    }

    /// Point at a binary and the app data dir, keeping the timeout. The child
    /// runs in `{data_dir}/sweep-workspace` (empty, its own) and is denied
    /// `{data_dir}/mcp`, where Podium keeps its MCP token files.
    pub fn set_launch(&self, command: String, data_dir: Option<PathBuf>) {
        let mut config = self.config.lock().expect("sweep config lock");
        config.command = command;
        config.cwd = data_dir.as_ref().map(|dir| dir.join(SWEEP_WORKSPACE));
        config.mcp_dir = data_dir.map(|dir| dir.join("mcp"));
    }

    /// Claim the one sweep slot. `None` while a sweep is still running (the
    /// tick is skipped) or after shutdown. The slot frees when the returned
    /// handle drops.
    pub fn begin(self: &Arc<Self>) -> Option<SweepRun> {
        if self.closed.load(Ordering::Relaxed) || self.busy.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(SweepRun {
            manager: Arc::clone(self),
            events: None,
        })
    }

    /// Stop sweeping (app shutdown); a turn mid-way is woken and its child
    /// killed.
    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.stop.notify_waiters();
    }

    async fn run(&self, prompt: &str) -> SweepOutcome {
        let failure = match self.attempt(prompt).await {
            Attempt::Done(text) => match text.and_then(|t| parse_messages(&t, Utc::now())) {
                Some(messages) => {
                    self.streak.lock().expect("sweep streak lock").record(None);
                    return SweepOutcome::Messages(messages);
                }
                None => {
                    tracing::debug!("the routine check's answer could not be read");
                    FAIL_UNREADABLE
                }
            },
            Attempt::Cut => return SweepOutcome::Quiet,
            Attempt::Failed(reason) => reason,
        };
        match self
            .streak
            .lock()
            .expect("sweep streak lock")
            .record(Some(failure))
        {
            Some(reason) => SweepOutcome::NotWorking(reason),
            None => SweepOutcome::Quiet,
        }
    }

    async fn attempt(&self, prompt: &str) -> Attempt {
        // Registered before the `closed` check, so a shutdown landing in
        // between still wakes this turn.
        let stop = self.stop.notified();
        tokio::pin!(stop);
        stop.as_mut().enable();
        if self.closed.load(Ordering::Relaxed) {
            return Attempt::Cut;
        }
        let config = self.config.lock().expect("sweep config lock").clone();
        // A fresh session per sweep: nothing carries over between sweeps.
        let mut session = match spawn(&config) {
            Ok(session) => session,
            Err(e) => {
                tracing::warn!("could not start the routine-check session: {e}");
                return Attempt::Failed(FAIL_START);
            }
        };
        let outcome = tokio::select! {
            outcome = tokio::time::timeout(config.turn_timeout, turn(&mut session, prompt)) => outcome,
            _ = &mut stop => {
                tracing::debug!("routine check cut short by shutdown");
                Ok(Err(()))
            }
        };
        let SweepSession {
            mut child, stdin, ..
        } = session;
        if matches!(outcome, Ok(Ok(Turn::Done(_)))) {
            // Done: closing stdin lets the CLI exit and stop its MCP servers.
            drop(stdin);
            if tokio::time::timeout(SESSION_EXIT_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        } else {
            let _ = child.kill().await;
        }
        match outcome {
            Ok(Ok(Turn::Done(text))) => Attempt::Done(text),
            Ok(Ok(Turn::Errored)) => Attempt::Failed(FAIL_ERRORED),
            Ok(Err(())) if self.closed.load(Ordering::Relaxed) => Attempt::Cut,
            Ok(Err(())) => Attempt::Failed(FAIL_ENDED),
            Err(_) => {
                tracing::warn!("routine check timed out; its session was killed");
                Attempt::Failed(FAIL_TIMEOUT)
            }
        }
    }
}

/// One turn: write `prompt`, read until the turn's result. `Err` when the
/// session ended under it.
async fn turn(session: &mut SweepSession, prompt: &str) -> Result<Turn, ()> {
    let line = serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": prompt }] },
    });
    let written = async {
        session
            .stdin
            .write_all(format!("{line}\n").as_bytes())
            .await?;
        session.stdin.flush().await
    };
    if let Err(e) = written.await {
        tracing::debug!("could not write the routine check: {e}");
        return Err(());
    }
    // The answer is the `result` event's text — the turn's closing message —
    // not the last prose block seen: a turn that ends on a tool call or at the
    // turn limit has narration ("Let me check Slack") as its last block.
    let mut last_text = None;
    let mut closing = None;
    while let Ok(Some(line)) = session.lines.next_line().await {
        if let Some(result) = result_event(&line) {
            closing = Some(result);
        }
        for event in parse_stream_line(&line) {
            match event {
                AssistantEvent::Message { text } => last_text = Some(text),
                AssistantEvent::TurnEnd { error } => {
                    if error.is_some() {
                        tracing::debug!("the routine check's turn ended in an error");
                        return Ok(Turn::Errored);
                    }
                    return Ok(Turn::Done(closing_text(closing, last_text)));
                }
                _ => {}
            }
        }
    }
    tracing::debug!("the routine-check session ended mid-turn");
    Err(())
}

/// One claimed sweep; dropping it frees the slot.
pub struct SweepRun {
    manager: Arc<SweepManager>,
    /// Set by [`Self::announce`]: the bus that hears it start and finish.
    events: Option<EventBus>,
}

impl SweepRun {
    /// Publish [`AssistantEvent::Sweeping`] `active: true` now and `false`
    /// when this run drops, so the end is announced on every path (answer,
    /// failure, timeout, shutdown, a panicking task).
    pub fn announce(mut self, events: EventBus) -> Self {
        publish_sweeping(&events, true);
        self.events = Some(events);
        self
    }

    /// The [`SweepOutcome::NotWorking`] notice this run returned was dropped
    /// rather than handed on: the next failed sweep raises it again.
    pub fn notice_discarded(&self) {
        self.manager
            .streak
            .lock()
            .expect("sweep streak lock")
            .rearm();
    }

    /// Run `prompt` as one turn of the sweep session.
    pub async fn run(&self, prompt: &str) -> SweepOutcome {
        self.manager.run(prompt).await
    }
}

impl Drop for SweepRun {
    fn drop(&mut self) {
        self.manager.busy.store(false, Ordering::Release);
        if let Some(events) = &self.events {
            publish_sweeping(events, false);
        }
    }
}

fn publish_sweeping(events: &EventBus, active: bool) {
    events.publish(PodiumEvent::AssistantEvent {
        event: AssistantEvent::Sweeping { active },
    });
}

fn spawn(config: &SweepConfig) -> CoreResult<SweepSession> {
    // The same launch path as the assistant's: the login shell (so an
    // nvm/homebrew `claude` resolves), `exec` so the child we hold is the
    // CLI, `--cd` in WSL-backend mode, the launch gate, no console window.
    let (shell, prefix) = match &config.cwd {
        Some(cwd) => crate::platform::login_shell_in(crate::platform::wsl(), cwd)?,
        None => crate::platform::login_shell(),
    };
    // The sweep's own empty directory, so it never starts in one that holds
    // anything (the app data dir has the MCP token files).
    if let Some(cwd) = &config.cwd {
        std::fs::create_dir_all(cwd)?;
    }
    let mut cmd = Command::new(shell);
    cmd.args(prefix)
        .arg(sweep_command(config)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Child output is never logged.
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }
    crate::platform::hide_console_window(cmd.as_std_mut());
    let mut child = crate::platform::spawn_gated(|| cmd.spawn())?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    Ok(SweepSession {
        child,
        stdin,
        lines: BufReader::new(stdout).lines(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SweepConfig {
        SweepConfig {
            command: "/opt/fake/claude".to_string(),
            ..SweepConfig::default()
        }
    }

    #[test]
    fn the_sweep_runs_on_haiku_read_only_without_podiums_mcp() {
        let config = SweepConfig {
            mcp_dir: Some(PathBuf::from("/data/podium/mcp")),
            ..config()
        };
        let line = sweep_command(&config).expect("command");
        assert!(line.starts_with("exec /opt/fake/claude -p "), "{line}");
        assert!(line.contains("--model haiku "), "{line}");
        assert!(line.contains("--input-format stream-json"), "{line}");
        assert!(line.contains("--setting-sources user "), "{line}");
        assert!(
            line.contains("--permission-mode bypassPermissions"),
            "{line}"
        );
        assert!(
            line.contains(
                "--disallowedTools Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task,\
                 SendMessage,AskUserQuestion,WebFetch,WebSearch,Read,Grep,Glob,LS,mcp__podium,mcp__podium__"
            ),
            "{line}"
        );
        // The user's own podium bridge is denied as a server and tool by tool,
        // for every tool the router has.
        for tool in crate::mcp::tools::PodiumTools::all_tool_names() {
            assert!(line.contains(&format!(",mcp__podium__{tool}")), "{tool}");
        }
        // The token files are unreadable to it.
        // Quoted the way the platform's shell wants it (single quotes under
        // POSIX, double quotes with escapes for cmd.exe), so the expectation
        // goes through the same quoting.
        let settings = r#"{"permissions":{"deny":["Read(//data/podium/mcp/**)"]}}"#;
        assert!(
            line.contains(&format!(
                "--settings {}",
                crate::platform::quote_arg(settings).expect("quote")
            )),
            "{line}"
        );
        // Podium's endpoint is not wired in, nothing hides the user's
        // Slack/Teams servers, and no earlier sweep is resumed.
        assert!(!line.contains("--mcp-config"), "{line}");
        assert!(!line.contains("--strict-mcp-config"), "{line}");
        assert!(!line.contains("--resume"), "{line}");
        assert!(line.contains("--append-system-prompt "), "{line}");
        assert!(line.contains("read-only"), "{line}");
        assert!(line.contains("final message"), "{line}");
        assert!(line.contains("JSON"), "{line}");
        assert!(line.contains("overrides any"), "{line}");
        assert!(line.contains("which source is unreachable"), "{line}");
        assert!(line.contains("no TL;DR"), "{line}");
    }

    #[test]
    fn the_sweep_runs_in_an_empty_directory_of_its_own() {
        let manager = SweepManager::new();
        manager.set_launch("claude".into(), Some(PathBuf::from("/data/podium")));
        let config = manager.config.lock().unwrap().clone();
        assert_eq!(
            config.cwd,
            Some(PathBuf::from("/data/podium/sweep-workspace"))
        );
        assert_eq!(config.mcp_dir, Some(PathBuf::from("/data/podium/mcp")));
    }

    #[test]
    fn the_notice_comes_after_three_failures_in_a_row_and_once() {
        let mut streak = FailureStreak::default();
        assert_eq!(streak.record(Some(FAIL_ENDED)), None);
        assert_eq!(streak.record(Some(FAIL_ENDED)), None);
        assert_eq!(streak.record(Some(FAIL_TIMEOUT)), Some(FAIL_TIMEOUT));
        // Quiet while it keeps failing.
        for _ in 0..5 {
            assert_eq!(streak.record(Some(FAIL_TIMEOUT)), None);
        }
        // A sweep that worked ends the streak: three more failures, one more
        // notice.
        assert_eq!(streak.record(None), None);
        assert_eq!(streak.record(Some(FAIL_START)), None);
        assert_eq!(streak.record(Some(FAIL_START)), None);
        assert_eq!(streak.record(Some(FAIL_START)), Some(FAIL_START));
        // And a success in between restarts the count.
        let mut streak = FailureStreak::default();
        assert_eq!(streak.record(Some(FAIL_START)), None);
        assert_eq!(streak.record(Some(FAIL_START)), None);
        assert_eq!(streak.record(None), None);
        assert_eq!(streak.record(Some(FAIL_START)), None);
    }

    #[test]
    fn a_discarded_notice_is_raised_again() {
        let mut streak = FailureStreak::default();
        for _ in 0..2 {
            assert_eq!(streak.record(Some(FAIL_ENDED)), None);
        }
        assert_eq!(streak.record(Some(FAIL_ENDED)), Some(FAIL_ENDED));
        streak.rearm();
        assert_eq!(streak.record(Some(FAIL_ENDED)), Some(FAIL_ENDED));
        // Delivered this time: quiet again.
        assert_eq!(streak.record(Some(FAIL_ENDED)), None);
    }

    /// The watcher drops a notice when the settings changed during the
    /// sweep; the check, still down, must not stay quiet for good.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dropped_not_working_notice_comes_back_with_the_next_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = manager_running(dir.path(), "exit 0", Duration::from_secs(30));
        for _ in 0..2 {
            let run = manager.begin().expect("free slot");
            assert_eq!(run.run("check").await, SweepOutcome::Quiet);
        }
        let run = manager.begin().expect("free slot");
        let outcome = run.run("check").await;
        assert!(
            matches!(outcome, SweepOutcome::NotWorking(_)),
            "{outcome:?}"
        );
        run.notice_discarded();
        drop(run);

        let run = manager.begin().expect("free slot");
        let outcome = run.run("check").await;
        assert!(
            matches!(outcome, SweepOutcome::NotWorking(_)),
            "{outcome:?}"
        );
        drop(run);
        // Handed on this time: quiet from here.
        let run = manager.begin().expect("free slot");
        assert_eq!(run.run("check").await, SweepOutcome::Quiet);
        manager.shutdown().await;
    }

    #[test]
    fn the_not_working_notice_is_podiums_own_one_line() {
        let polling = AssistantPolling {
            interval_minutes: 10,
            slack: true,
            teams: false,
        };
        let text = not_working_text(polling, FAIL_START);
        assert!(text.starts_with("[Podium] The routine check of Slack is not working"));
        assert!(text.contains(FAIL_START), "{text}");
        assert_eq!(text.lines().count(), 1);
    }

    /// A manager whose "CLI" is `body`, a shell script run after the login
    /// shell starts it, with `timeout` per turn.
    #[cfg(unix)]
    fn manager_running(dir: &std::path::Path, body: &str, timeout: Duration) -> Arc<SweepManager> {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-claude");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write fake cli");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let manager = Arc::new(SweepManager::new());
        manager.configure(SweepConfig {
            command: script.to_string_lossy().into_owned(),
            cwd: Some(dir.join("sweep-workspace")),
            mcp_dir: None,
            turn_timeout: timeout,
        });
        manager
    }

    /// A hung session is killed at the timeout and counts as a failure.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_hung_sweep_times_out_as_a_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = manager_running(dir.path(), "exec sleep 30", Duration::from_millis(500));
        let run = manager.begin().expect("free slot");
        assert_eq!(run.run("check").await, SweepOutcome::Quiet);
        assert_eq!(manager.streak.lock().unwrap().failures, 1);
        manager.shutdown().await;
    }

    /// The sweep is refused at least what a lookup sub-agent is (file edits,
    /// shell, messaging, the CLI's own subagents), plus its own extras.
    #[test]
    fn the_sweep_denies_every_cli_tool_a_lookup_does() {
        let denied: Vec<&str> = SWEEP_DISALLOWED_CLI_TOOLS.split(',').collect();
        for tool in crate::subagent::LOOKUP_DISALLOWED_CLI_TOOLS {
            assert!(denied.contains(tool), "{tool} is not denied to the sweep");
        }
    }

    #[test]
    fn the_closing_text_is_the_result_not_the_last_narration() {
        let narration = Some("Let me check Slack".to_string());
        let result = |line: &str| result_event(line);
        // The result text wins over the last prose block.
        let done = result(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"messages\":[]}"}"#,
        );
        assert_eq!(
            closing_text(done, narration.clone()).as_deref(),
            Some(r#"{"messages":[]}"#)
        );
        // Cut off at the turn limit: the narration is no answer.
        let cut = result(r#"{"type":"result","subtype":"error_max_turns","is_error":false}"#);
        assert_eq!(closing_text(cut, narration.clone()), None);
        // A normal end without result text falls back to the last block.
        let bare = result(r#"{"type":"result","subtype":"success","is_error":false}"#);
        assert_eq!(closing_text(bare, narration.clone()), narration);
        // Other lines are not result events.
        assert!(result(r#"{"type":"assistant","message":{"content":[]}}"#).is_none());
        assert!(result("not json").is_none());
    }

    /// One sweep, one fresh session: narration, then the JSON answer, parsed.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_sweep_answer_comes_back_as_messages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let newest = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let answer = serde_json::json!({ "messages": [{
            "source": "slack", "conversation": "D0FAKE1", "thread": "D0FAKE1",
            "sender": "Ada", "place": "Slack, direct message",
            "newest": newest, "text": "Can you approve the deploy?",
        }]})
        .to_string();
        let result = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": false, "result": answer,
        });
        let manager = manager_running(
            dir.path(),
            &format!(
                r#"echo '{{"type":"system","subtype":"init","session_id":"s"}}'
read -r _line
echo '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Let me check Slack"}}]}}}}'
echo '{result}'"#
            ),
            Duration::from_secs(5),
        );
        let run = manager.begin().expect("free slot");
        let SweepOutcome::Messages(messages) = run.run("check").await else {
            panic!("no messages");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].key(), "msg:slack:D0FAKE1:D0FAKE1");
        assert_eq!(messages[0].sender, "Ada");
        manager.shutdown().await;
    }

    fn entry(newest: DateTime<Utc>) -> serde_json::Value {
        serde_json::json!({
            "source": "teams",
            "conversation": "19:fake@thread.v2",
            "thread": "1700000000001",
            "sender": "Grace",
            "place": "Teams, #release",
            "newest": newest.to_rfc3339(),
            "text": "Are the Q3 numbers final?",
        })
    }

    #[test]
    fn a_valid_answer_parses_into_waiting_messages() {
        let now = Utc::now();
        let newest = now - chrono::Duration::hours(3);
        let reply = serde_json::json!({ "messages": [entry(newest)] }).to_string();
        let messages = parse_messages(&reply, now).expect("readable");
        assert_eq!(
            messages,
            vec![WaitingMessage {
                source: "teams",
                conversation: "19:fake@thread.v2".into(),
                thread: "1700000000001".into(),
                sender: "Grace".into(),
                place: "Teams, #release".into(),
                newest: DateTime::parse_from_rfc3339(&newest.to_rfc3339())
                    .unwrap()
                    .with_timezone(&Utc),
                text: "Are the Q3 numbers final?".into(),
            }]
        );
        assert_eq!(
            messages[0].key(),
            "msg:teams:19:fake@thread.v2:1700000000001"
        );
        // An empty list is a real answer: nothing is waiting.
        assert_eq!(parse_messages(r#"{"messages":[]}"#, now), Some(vec![]));
        // Fenced or with a line of prose around it, as Haiku does.
        let fenced = format!("Here you go:\n```json\n{reply}\n```");
        assert_eq!(parse_messages(&fenced, now).map(|m| m.len()), Some(1));
    }

    #[test]
    fn a_malformed_entry_is_dropped_on_its_own() {
        let now = Utc::now();
        let good = entry(now - chrono::Duration::minutes(5));
        let mut no_thread = good.clone();
        no_thread.as_object_mut().unwrap().remove("thread");
        let mut bad_source = good.clone();
        bad_source["source"] = "mail".into();
        let mut bad_time = good.clone();
        bad_time["newest"] = "yesterday".into();
        let mut no_sender = good.clone();
        no_sender.as_object_mut().unwrap().remove("sender");
        let mut blank_text = good.clone();
        blank_text["text"] = "   ".into();
        let mut long_id = good.clone();
        long_id["conversation"] = "x".repeat(MAX_ID_CHARS + 1).into();
        let reply = serde_json::json!({ "messages": [
            good, no_thread, bad_source, bad_time, no_sender, blank_text, long_id, 42,
        ]})
        .to_string();
        let messages = parse_messages(&reply, now).expect("readable");
        assert_eq!(messages.len(), 2, "{messages:?}");
        // No thread: the conversation stands in for it.
        assert_eq!(messages[1].thread, messages[1].conversation);
    }

    #[test]
    fn a_wholly_malformed_answer_is_unreadable() {
        let now = Utc::now();
        for reply in [
            "",
            "NOTHING_TO_REPORT",
            "Slack is unreachable (not signed in).",
            "{not json}",
            r#"{"items":[]}"#,
            r#"{"messages":"none"}"#,
            "} backwards {",
        ] {
            assert_eq!(parse_messages(reply, now), None, "{reply}");
        }
    }

    #[test]
    fn entries_older_than_the_window_are_cut_off() {
        let now = Utc::now();
        let reply = serde_json::json!({ "messages": [
            entry(now - chrono::Duration::days(MESSAGE_WINDOW_DAYS) - chrono::Duration::minutes(1)),
            entry(now - chrono::Duration::days(MESSAGE_WINDOW_DAYS) + chrono::Duration::minutes(1)),
        ]})
        .to_string();
        assert_eq!(parse_messages(&reply, now).map(|m| m.len()), Some(1));
    }

    #[tokio::test]
    async fn one_sweep_at_a_time() {
        let manager = Arc::new(SweepManager::new());
        let first = manager.begin().expect("free slot");
        assert!(manager.begin().is_none(), "a second sweep started");
        drop(first);
        let again = manager.begin().expect("slot freed");
        drop(again);
        manager.shutdown().await;
        assert!(manager.begin().is_none(), "a sweep started after shutdown");
    }

    /// An announced run publishes its start, and its end when it drops, also
    /// after a failed sweep.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_announced_sweep_publishes_its_start_and_end_even_when_it_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = manager_running(dir.path(), "exit 1", Duration::from_secs(5));
        let events = EventBus::new();
        let mut rx = events.subscribe();
        let sweeping = |event: PodiumEvent| match event {
            PodiumEvent::AssistantEvent {
                event: AssistantEvent::Sweeping { active },
            } => active,
            other => panic!("unexpected event {other:?}"),
        };
        let run = manager.begin().expect("free slot").announce(events);
        assert!(sweeping(rx.try_recv().expect("start published")));
        assert_eq!(run.run("check").await, SweepOutcome::Quiet);
        assert!(rx.try_recv().is_err(), "ended before the run dropped");
        drop(run);
        assert!(!sweeping(rx.try_recv().expect("end published")));
        // An unannounced run (as the other tests use) publishes nothing.
        drop(manager.begin().expect("slot freed"));
        assert!(rx.try_recv().is_err());
        manager.shutdown().await;
    }
}
