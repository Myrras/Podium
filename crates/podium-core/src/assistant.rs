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
//! The conversation itself is persisted: an [`AssistantStore`] keeps the
//! transcript and Claude's session id in `assistant.json`, so a restart
//! replays the chat and resumes the same session (`--resume`). A separate
//! `assistant-memory.md` — injected into the system prompt, updated by the
//! assistant over MCP and editable by the user — is what it remembers across
//! conversations.
//!
//! Podium can also speak first: [`AssistantManager::nudge`] writes a tagged
//! `[Podium]` turn into the session (see [`crate::proactivity`] for what
//! triggers one).
//!
//! Only one turn is ever in flight at a time. A nudge and a proposal-outcome
//! notice (`notify`) — Podium's own inputs — always route through one
//! pending-input queue: written while a turn is running (or before any
//! session exists, for a nudge) they wait instead of interleaving into the
//! turn in progress, and are delivered as their own turn once the current one
//! ends (`TurnEnd`) — one waiting item per turn, oldest first, never merged.
//! Whether the user's *own* messages join that same queue is the
//! `queue_messages` setting (Settings → Assistant, default **off**): off, a
//! message typed mid-turn goes straight into the live session as it always
//! did before this queue existed, possibly interleaving into the answer; on,
//! it waits its turn like everything else. Either way, a new user message
//! never cuts ahead of whatever is already fairly waiting (a nudge queued
//! while nobody was chatting, or a message queued before the setting was
//! flipped) — "off" only stops a *new* message from joining that line, it
//! does not let one skip it. Flipping the setting never affects items
//! already waiting — only where a *new* user message goes.
//! [`AssistantManager::interrupt`] cuts a running turn short and delivers the
//! next waiting item immediately rather than for a `TurnEnd` that would never
//! come. The queue is not persisted: a session ending for any reason (crash,
//! a new conversation, shutdown) drops it and reports what was never sent.
//!
//! Proposals still live in memory only, so a pending one is lost on restart.

use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Mutex;

use crate::agent::McpConnectInfo;
use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};
use crate::ids::{PendingId, ProjectId, ProposalId};

/// Tools the assistant is never allowed to run: it delegates implementation
/// work to spawned agents instead of touching the filesystem itself.
const DISALLOWED_TOOLS: &str = "Edit,Write,NotebookEdit,Bash";

const LOCK_POISONED: &str = "assistant config lock poisoned";

/// Why a proposal was dropped without the user deciding on it.
const PROJECT_CLOSED: &str = "its project was closed";

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
concrete; the panel is narrow. When a choice materially affects what happens \
(e.g. which model to run a spawned agent with), ask the user in the chat \
instead of silently picking a default. If something the user wants cannot be \
done with the tools available, say so and ask rather than quietly doing \
something else. What you learn about the user and their work \
belongs in the memory below: call update_assistant_memory with the full new \
text (it replaces what is there) whenever something is worth having next \
time.";

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
    "remove_process",
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantSettings {
    #[serde(default)]
    pub autonomy: AutonomyMode,
    /// Whether Podium may nudge the assistant unprompted (an agent finished,
    /// one looks stuck, to-dos are lingering, a new day started). Defaults to
    /// on — also for an `agents.json` written before this field existed.
    #[serde(default = "default_true")]
    pub proactive: bool,
    /// Whether a message typed while a turn is running waits its turn
    /// (queued, FIFO) instead of going straight into the live session.
    /// Defaults to **off** — the behaviour from before this queue existed —
    /// so an `agents.json` written before this field existed reads the same
    /// way it always ran. Governs only the user's own messages: a
    /// proactivity nudge or a proposal-outcome notice always queues,
    /// regardless of this setting.
    #[serde(default)]
    pub queue_messages: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            autonomy: AutonomyMode::default(),
            proactive: true,
            queue_messages: false,
        }
    }
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

/// Longest the pending-input queue may grow: newest kept, oldest dropped with
/// a chat notice — what happened hours ago is not worth replaying.
// ponytail: flat cap, oldest dropped.
const MAX_PENDING: usize = 20;

/// Whether a queued item can be edited or withdrawn before it is delivered.
/// Only a user message can; a nudge or a proposal-outcome notice is Podium's
/// own text, delivered verbatim once its turn comes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PendingKind {
    User,
    Nudge,
    Notice,
}

/// One turn waiting for the session to become free. FIFO — the oldest is
/// delivered next, as its own turn, never merged with another.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingInput {
    id: PendingId,
    kind: PendingKind,
    text: String,
}

/// UI-facing snapshot of one waiting item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessage {
    pub id: PendingId,
    pub kind: PendingKind,
    pub text: String,
}

impl From<&PendingInput> for QueuedMessage {
    fn from(p: &PendingInput) -> Self {
        Self {
            id: p.id,
            kind: p.kind,
            text: p.text.clone(),
        }
    }
}

/// A parked proposal: what the user sees, the project it acts on (`None` for
/// a workspace-wide call) and the call itself.
struct PendingProposal {
    info: ProposalInfo,
    project: Option<ProjectId>,
    action: ProposalAction,
}

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
    /// Podium said something in the chat itself — a proactive nudge it just
    /// handed the session. Shown as-is, so the user knows why the assistant
    /// is about to speak unprompted.
    Notice { text: String },
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
    /// The pending-input queue changed (something was queued, delivered,
    /// edited, removed, dropped by the cap, or emptied when the session
    /// ended). Carries the whole queue, oldest first — a snapshot, not a
    /// delta, so the UI never has to reconcile.
    QueueChanged { items: Vec<QueuedMessage> },
    /// A plain notice for the *user* — not routed to the assistant, unlike
    /// [`AssistantEvent::Notice`] — e.g. a queued message was dropped by the
    /// cap, a turn was interrupted, or messages were left undelivered when
    /// the session ended.
    QueueNotice { text: String },
    /// A user message that was waiting in the queue was just delivered as
    /// its own turn. Unlike the very first, immediately-delivered message
    /// (which the caller already shows), nothing else announces this one
    /// starting, so the UI needs it to promote the waiting row into the
    /// ordinary log.
    QueueItemDelivered { text: String },
}

/// Who spoke in one persisted transcript row. A subset of the live event
/// stream: proposals are runtime-only, and turn/session bookkeeping is not
/// conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptRole {
    User,
    Assistant,
    /// An action the assistant ran (the tool name).
    Tool,
    /// Something Podium reports, e.g. an action that failed.
    Notice,
    /// A proactivity nudge Podium sent the assistant (an internal
    /// instruction, not a user-facing notice). Kept distinct from `Notice`
    /// so a replayed row renders the same muted way a live nudge does.
    Nudge,
}

/// One persisted chat row, replayed into the panel after a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub role: TranscriptRole,
    pub text: String,
    pub at: DateTime<Utc>,
}

/// Rows kept in `assistant.json`. The conversation itself lives in Claude's
/// own session (which `--resume` picks up); this is only what the panel
/// replays, so dropping the oldest rows loses nothing the assistant knows.
// ponytail: flat cap, oldest-first drop. Paginate if a user ever wants the
// full history back.
const MAX_TRANSCRIPT_ENTRIES: usize = 500;

/// The conversation as persisted: Claude's session id (to `--resume` after a
/// restart) plus the rows the panel replays.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConversation {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript: Vec<TranscriptEntry>,
    /// The last day Podium asked for a day-start summary, so restarting the
    /// app three times in one afternoon does not ask three more times.
    #[serde(default)]
    last_day_start: Option<NaiveDate>,
}

struct StoreInner {
    /// `assistant.json`; `None` keeps the conversation in memory (tests, or a
    /// data dir that failed to resolve).
    path: Option<PathBuf>,
    /// `assistant-memory.md`; `None` means the assistant has no memory file.
    memory_path: Option<PathBuf>,
    conversation: StoredConversation,
}

/// The assistant's persisted state: the transcript + session id (one JSON
/// file, atomic writes, exactly like [`crate::todo::TodoStore`]) and the
/// memory the user can also edit by hand (one markdown file, always read
/// fresh from disk rather than cached).
pub struct AssistantStore {
    inner: StdMutex<StoreInner>,
}

impl AssistantStore {
    fn new() -> Self {
        Self {
            inner: StdMutex::new(StoreInner {
                path: None,
                memory_path: None,
                conversation: StoredConversation::default(),
            }),
        }
    }

    /// Point the store at `dir` (the app data dir) and load what is there.
    /// Anything accumulated in memory before this call is replaced.
    fn set_dir(&self, dir: &Path) {
        let path = dir.join("assistant.json");
        let conversation = load_conversation(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.memory_path = Some(dir.join("assistant-memory.md"));
        inner.conversation = conversation;
    }

    /// The rows the panel replays, oldest first.
    pub fn transcript(&self) -> Vec<TranscriptEntry> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .conversation
            .transcript
            .clone()
    }

    /// Claude's session id for the ongoing conversation, if one was started.
    fn session_id(&self) -> Option<String> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .conversation
            .session_id
            .clone()
    }

    /// Forget the session id, keeping the transcript: used when a session
    /// never came up, so a stale id cannot wedge every later turn.
    fn forget_session(&self) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if inner.conversation.session_id.take().is_some() {
            let _ = save_conversation(&inner);
        }
    }

    /// Start over: no transcript, no session to resume. The day-start claim
    /// survives — it is about the day, not the conversation.
    fn clear(&self) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.conversation = StoredConversation {
            last_day_start: inner.conversation.last_day_start,
            ..StoredConversation::default()
        };
        let _ = save_conversation(&inner);
    }

    fn append(&self, role: TranscriptRole, text: String) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let transcript = &mut inner.conversation.transcript;
        transcript.push(TranscriptEntry {
            role,
            text,
            at: Utc::now(),
        });
        if transcript.len() > MAX_TRANSCRIPT_ENTRIES {
            let drop = transcript.len() - MAX_TRANSCRIPT_ENTRIES;
            transcript.drain(..drop);
        }
        // Best effort: a chat row that cannot be saved must not fail the turn.
        let _ = save_conversation(&inner);
    }

    /// Record whatever of `event` belongs in the persisted conversation.
    fn record(&self, event: &AssistantEvent) {
        match event {
            AssistantEvent::Started { session_id } => {
                let mut inner = self.inner.lock().expect(LOCK_POISONED);
                if inner.conversation.session_id.as_deref() != Some(session_id.as_str()) {
                    inner.conversation.session_id = Some(session_id.clone());
                    let _ = save_conversation(&inner);
                }
            }
            AssistantEvent::Message { text } => {
                self.append(TranscriptRole::Assistant, text.clone())
            }
            AssistantEvent::Tool { name } => self.append(TranscriptRole::Tool, name.clone()),
            AssistantEvent::ToolFailed { message } => {
                self.append(TranscriptRole::Notice, message.clone())
            }
            AssistantEvent::Notice { text } => self.append(TranscriptRole::Nudge, text.clone()),
            AssistantEvent::QueueNotice { text } => {
                self.append(TranscriptRole::Notice, text.clone())
            }
            // Proposals, turn ends, exits and queue snapshots are runtime
            // signals, not chat.
            _ => {}
        }
    }

    /// Claim today's day-start summary: true the first time it is called on a
    /// given (local) day, false afterwards — including after a restart.
    pub fn claim_day_start(&self) -> bool {
        let today = chrono::Local::now().date_naive();
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if inner.conversation.last_day_start == Some(today) {
            return false;
        }
        inner.conversation.last_day_start = Some(today);
        let _ = save_conversation(&inner);
        true
    }

    /// What the assistant remembers across conversations (empty when there is
    /// no memory file yet). Read from disk every time — the user may edit the
    /// file by hand.
    pub fn memory(&self) -> String {
        let path = self.inner.lock().expect(LOCK_POISONED).memory_path.clone();
        path.and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default()
    }

    /// Replace the memory; an empty `content` clears it. Takes effect in the
    /// running session's prompt only on its next (re)spawn.
    pub fn set_memory(&self, content: &str) -> CoreResult<()> {
        let Some(path) = self.inner.lock().expect(LOCK_POISONED).memory_path.clone() else {
            return Err(CoreError::InvalidInput(
                "the assistant has no memory file".to_string(),
            ));
        };
        write_atomically(&path, content.as_bytes())
    }
}

/// Read the conversation from disk; a missing or corrupt file starts empty.
fn load_conversation(path: &Path) -> StoredConversation {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return StoredConversation::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("assistant conversation file is corrupt, starting fresh: {e}");
        StoredConversation::default()
    })
}

/// Persist the conversation atomically. A store without a path is a no-op.
fn save_conversation(inner: &StoreInner) -> CoreResult<()> {
    let Some(path) = &inner.path else {
        return Ok(());
    };
    let json = serde_json::to_string_pretty(&inner.conversation).map_err(|e| {
        CoreError::InvalidInput(format!("failed to encode the assistant conversation: {e}"))
    })?;
    write_atomically(path, json.as_bytes())
}

/// Temp file + rename, so a crash mid-write can never tear the file.
fn write_atomically(path: &Path, bytes: &[u8]) -> CoreResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Where the assistant session comes from. Injectable so tests can point at a
/// fake `claude` binary instead of the real CLI.
#[derive(Debug, Clone)]
pub struct AssistantConfig {
    /// Binary speaking Claude Code's headless `stream-json` protocol.
    pub command: String,
    /// Working directory for the child; `None` inherits Podium's own.
    pub cwd: Option<PathBuf>,
    /// Where `assistant.json` (transcript + session id) and
    /// `assistant-memory.md` live; `None` keeps the conversation in memory
    /// and leaves the assistant without a memory file.
    pub data_dir: Option<PathBuf>,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            cwd: None,
            data_dir: None,
        }
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    /// Set before Podium kills the child on purpose (starting a new
    /// conversation, or interrupting a turn), so its output task stays quiet
    /// instead of reporting the session as having ended under the user. Also
    /// this session's identity: compared by pointer to tell whether a
    /// background task's session is still the one installed in
    /// [`SessionState`], so a task cleaning up after an old session can never
    /// clobber one that replaced it in the meantime.
    quiet_exit: Arc<AtomicBool>,
}

/// The session plus turn-in-flight bookkeeping plus the pending-input queue,
/// all under one lock — so a queued item can never slip past a turn that
/// starts between two lock acquisitions.
struct SessionState {
    session: Option<Session>,
    /// Whether a turn is currently running: from the moment it is written to
    /// stdin until its `TurnEnd` (or the session dying without one).
    in_flight: bool,
    /// User messages, nudges and proposal-outcome notices waiting for the
    /// current turn to end. FIFO — oldest delivered next, one per `TurnEnd`,
    /// never merged.
    pending: VecDeque<PendingInput>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            session: None,
            in_flight: false,
            pending: VecDeque::new(),
        }
    }
}

/// Owns the assistant session, its lazy (re)spawn, and the proposals its
/// gated tool calls are parked in.
pub struct AssistantManager {
    config: StdMutex<AssistantConfig>,
    /// `Arc` so the background output task (spawned per session, `'static`)
    /// can share the same lock as the manager without borrowing `self`.
    state: Arc<Mutex<SessionState>>,
    /// Gated tool calls awaiting the user, oldest first, each with the
    /// project it acts on (so closing that project can invalidate it). In
    /// memory only: a proposal never expires on its own, but does not survive
    /// a restart.
    proposals: StdMutex<Vec<PendingProposal>>,
    /// The persisted conversation (transcript + session id) and the memory
    /// file. Shared with the output task, which records what it publishes.
    store: Arc<AssistantStore>,
    events: EventBus,
}

impl AssistantManager {
    pub fn new(events: EventBus) -> Self {
        Self {
            config: StdMutex::new(AssistantConfig::default()),
            state: Arc::new(Mutex::new(SessionState::new())),
            proposals: StdMutex::new(Vec::new()),
            store: Arc::new(AssistantStore::new()),
            events,
        }
    }

    /// The persisted conversation and memory.
    pub fn store(&self) -> &AssistantStore {
        &self.store
    }

    /// Point the manager at a binary / working directory (the app data dir in
    /// the real app, a fake CLI in tests).
    pub fn configure(&self, config: AssistantConfig) {
        if let Some(dir) = &config.data_dir {
            self.store.set_dir(dir);
        }
        *self.config.lock().expect(LOCK_POISONED) = config;
    }

    /// Send a chat turn, spawning the session on first use (or after it
    /// died). `mcp` is how the new session reaches Podium's own MCP
    /// endpoint; a session spawned before the server is up simply has no
    /// Podium tools. With `queue_messages` on, delivered immediately if the
    /// session is idle and queued (see the module docs) otherwise; off (the
    /// default), it goes straight into the live session whenever nothing is
    /// already fairly waiting ahead of it — exactly as it did before the
    /// queue existed. Either way, the setting governs only the user's own
    /// messages, never a nudge or a proposal-outcome notice.
    pub async fn send(
        &self,
        text: &str,
        mcp: Option<McpConnectInfo>,
        queue_messages: bool,
    ) -> CoreResult<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "message must not be blank".to_string(),
            ));
        }
        let mut state = self.state.lock().await;
        // A session that exited (crash, missing CLI) is replaced rather than
        // written to: the write would only fail with a broken pipe.
        if state
            .session
            .as_mut()
            .is_some_and(|s| matches!(s.child.try_wait(), Ok(Some(_)) | Err(_)))
        {
            state.session = None;
            state.in_flight = false;
        }
        if state.session.is_none() {
            let config = self.config.lock().expect(LOCK_POISONED).clone();
            state.session = Some(self.spawn(&config, mcp.as_ref())?);
        }
        let item = PendingInput {
            id: PendingId::new(),
            kind: PendingKind::User,
            text: text.to_string(),
        };
        // Off still respects whatever is already fairly waiting (e.g. a
        // nudge queued while nobody was chatting) — it only stops a *new*
        // message from joining the back of that line. Only with nothing
        // ahead of it does it bypass straight to the session.
        if queue_messages || !state.pending.is_empty() {
            deliver_or_queue(&mut state, item, &self.store, &self.events).await;
        } else {
            deliver_bypassing_queue(&mut state, item, &self.store, &self.events).await;
        }
        Ok(())
    }

    /// Drop the conversation: kill the session, forget the transcript, the
    /// session id and the pending queue, and discard proposals that belonged
    /// to it. Whatever was still waiting is reported as undelivered in the
    /// fresh (otherwise empty) transcript. The next turn starts a session
    /// with no `--resume`.
    pub async fn new_conversation(&self) {
        let dropped = {
            let mut state = self.state.lock().await;
            if let Some(mut session) = state.session.take() {
                // A deliberate kill: the panel is being cleared, so the
                // output task must not report the session as having ended by
                // itself.
                session.quiet_exit.store(true, Ordering::Relaxed);
                let _ = session.child.kill().await;
            }
            state.in_flight = false;
            std::mem::take(&mut state.pending)
        };
        self.proposals.lock().expect(LOCK_POISONED).clear();
        self.store.clear();
        // After clearing, so the notice survives as the first line of the
        // fresh conversation rather than being wiped along with the old one.
        report_undelivered(&self.store, &self.events, dropped);
    }

    /// Stop the turn in progress, if any, and immediately hand the next
    /// waiting message over — a `TurnEnd` for the killed turn will never
    /// arrive, so the normal one-per-turn drain would wait forever. With
    /// nothing running this is a no-op: nothing to stop, nothing to report.
    pub async fn interrupt(&self, mcp: Option<McpConnectInfo>) -> CoreResult<()> {
        let mut state = self.state.lock().await;
        if !state.in_flight {
            return Ok(());
        }
        if let Some(mut session) = state.session.take() {
            session.quiet_exit.store(true, Ordering::Relaxed);
            let _ = session.child.kill().await;
        }
        state.in_flight = false;
        let event = AssistantEvent::QueueNotice {
            text: "[Podium] The current turn was interrupted.".to_string(),
        };
        self.store.record(&event);
        self.events.publish(PodiumEvent::AssistantEvent { event });
        if !state.pending.is_empty() {
            let config = self.config.lock().expect(LOCK_POISONED).clone();
            state.session = Some(self.spawn(&config, mcp.as_ref())?);
            drain_next(&mut state, &self.store, &self.events).await;
        }
        Ok(())
    }

    /// Change a waiting user message's text before it is delivered. Nudges
    /// and proposal-outcome notices are not editable — they are Podium's own
    /// text.
    pub async fn edit_pending(&self, id: PendingId, text: &str) -> CoreResult<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "message must not be blank".to_string(),
            ));
        }
        let mut state = self.state.lock().await;
        let item = state
            .pending
            .iter_mut()
            .find(|p| p.id == id && p.kind == PendingKind::User)
            .ok_or(CoreError::QueuedMessageNotFound)?;
        item.text = text.to_string();
        publish_queue(&state, &self.events);
        Ok(())
    }

    /// Withdraw a waiting user message so nothing is delivered for it.
    pub async fn remove_pending(&self, id: PendingId) -> CoreResult<()> {
        let mut state = self.state.lock().await;
        let before = state.pending.len();
        state
            .pending
            .retain(|p| !(p.id == id && p.kind == PendingKind::User));
        if state.pending.len() == before {
            return Err(CoreError::QueuedMessageNotFound);
        }
        publish_queue(&state, &self.events);
        Ok(())
    }

    /// Publish one event and persist whatever of it belongs in the transcript.
    fn publish(&self, event: AssistantEvent) {
        self.store.record(&event);
        self.events.publish(PodiumEvent::AssistantEvent { event });
    }

    /// Park a gated tool call and tell the UI about it. `project` is the
    /// project the call acts on, if any, so closing it can invalidate the
    /// proposal. Returns the id the user's confirm/reject refers to.
    pub fn propose(
        &self,
        tool: String,
        summary: String,
        project: Option<ProjectId>,
        action: ProposalAction,
    ) -> ProposalId {
        let info = ProposalInfo {
            id: ProposalId::new(),
            tool,
            summary,
            created_at: Utc::now(),
        };
        let id = info.id;
        self.publish(AssistantEvent::Proposal {
            id,
            tool: info.tool.clone(),
            summary: info.summary.clone(),
        });
        self.proposals
            .lock()
            .expect(LOCK_POISONED)
            .push(PendingProposal {
                info,
                project,
                action,
            });
        id
    }

    /// The proposals still awaiting the user, oldest first.
    pub fn proposals(&self) -> Vec<ProposalInfo> {
        self.proposals
            .lock()
            .expect(LOCK_POISONED)
            .iter()
            .map(|p| p.info.clone())
            .collect()
    }

    /// Drop the proposals that act on `project`, because it just closed:
    /// confirming one could only fail. The user sees the cards resolve with
    /// the reason, and the assistant is told so it stops waiting.
    pub async fn invalidate_proposals_for_project(&self, project: ProjectId) {
        let dropped: Vec<ProposalInfo> = {
            let mut pending = self.proposals.lock().expect(LOCK_POISONED);
            let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *pending)
                .into_iter()
                .partition(|p| p.project == Some(project));
            *pending = kept;
            gone.into_iter().map(|p| p.info).collect()
        };
        for info in dropped {
            self.publish(AssistantEvent::ProposalResolved {
                id: info.id,
                approved: false,
                error: Some(PROJECT_CLOSED.to_string()),
            });
            self.notify(&format!(
                "[Podium] Your proposed `{}` action was dropped: {PROJECT_CLOSED}.",
                info.tool
            ))
            .await;
        }
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
                .position(|p| p.info.id == id)
                .ok_or(CoreError::ProposalNotFound)?;
            pending.remove(at)
        };
        let PendingProposal { info, action, .. } = taken;
        let outcome = if approved { Some(action().await) } else { None };
        let error = match &outcome {
            Some(Err(message)) => Some(message.clone()),
            _ => None,
        };
        self.publish(AssistantEvent::ProposalResolved {
            id,
            approved,
            error,
        });
        self.notify(&outcome_notice(&info.tool, outcome)).await;
        Ok(())
    }

    /// Hand the assistant something Podium noticed (an agent finished, a
    /// to-do is lingering, …). Delivered straight away if a live session is
    /// idle; queued (busy, or no session at all) otherwise — Podium never
    /// starts the CLI just to deliver a nudge.
    pub async fn nudge(&self, text: String) {
        let mut state = self.state.lock().await;
        let item = PendingInput {
            id: PendingId::new(),
            kind: PendingKind::Nudge,
            text,
        };
        deliver_or_queue(&mut state, item, &self.store, &self.events).await;
    }

    /// Write a Podium-side message into a *live* session, queuing it if a
    /// turn is already running. Unlike [`send`] this never spawns one: a
    /// fresh session would have no memory of what the notice is about, so
    /// with no session at all it is silently skipped, exactly as before this
    /// queue existed.
    ///
    /// [`send`]: Self::send
    async fn notify(&self, text: &str) {
        let mut state = self.state.lock().await;
        if state.session.is_none() {
            return;
        }
        let item = PendingInput {
            id: PendingId::new(),
            kind: PendingKind::Notice,
            text: text.to_string(),
        };
        deliver_or_queue(&mut state, item, &self.store, &self.events).await;
    }

    /// Kill the session, if any (app shutdown). Whatever is still waiting is
    /// reported as undelivered once the output task notices the session is
    /// gone.
    pub async fn shutdown(&self) {
        if let Some(mut session) = self.state.lock().await.session.take() {
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
        // Pick the stored conversation back up, so a restart continues where
        // the panel left off instead of starting a stranger.
        let resume = self
            .store
            .session_id()
            .map(|id| {
                Ok::<String, CoreError>(format!(" --resume {}", crate::platform::quote_arg(&id)?))
            })
            .transpose()?
            .unwrap_or_default();
        let resumed = !resume.is_empty();
        // `-p`/stream-json mode has no TTY for an interactive permission
        // prompt, so without --permission-mode the CLI would hang asking
        // approval for every Podium MCP tool it never gets to answer.
        // bypassPermissions is safe here because deny rules still win:
        // DISALLOWED_TOOLS above keeps Edit/Write/NotebookEdit/Bash refused
        // regardless of permission mode, so the assistant still can't touch
        // the filesystem itself (autonomy gating happens server-side, in
        // Podium's own MCP layer, not via CLI prompts).
        let command = format!(
            "exec {} -p --input-format stream-json --output-format stream-json \
             --verbose --permission-mode bypassPermissions --disallowedTools {}{}{} \
             --append-system-prompt {}",
            crate::platform::quote_arg(&config.command)?,
            DISALLOWED_TOOLS,
            mcp_config,
            resume,
            crate::platform::quote_arg(&identity_prompt(&self.store.memory()))?,
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
        let store = Arc::clone(&self.store);
        let state = Arc::clone(&self.state);
        let quiet_exit = Arc::new(AtomicBool::new(false));
        let quiet = Arc::clone(&quiet_exit);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut started = false;
            // Non-JSON noise (a chatty shell profile) is skipped, not fatal.
            while let Ok(Some(line)) = lines.next_line().await {
                for event in parse_stream_line(&line) {
                    started |= matches!(event, AssistantEvent::Started { .. });
                    let is_turn_end = matches!(event, AssistantEvent::TurnEnd { .. });
                    store.record(&event);
                    events.publish(PodiumEvent::AssistantEvent { event });
                    if is_turn_end {
                        // The turn is over, so the next waiting item (if any)
                        // becomes its own turn right away — never merged, and
                        // this advances the queue even when the turn ended in
                        // error.
                        let mut st = state.lock().await;
                        st.in_flight = false;
                        drain_next(&mut st, &store, &events).await;
                    }
                }
            }
            // A session that never announced itself is a session that never
            // came up. If we asked to resume one, the stored id is the likely
            // culprit (Claude prunes old sessions) — drop it so the next turn
            // starts fresh instead of failing forever.
            if !started && resumed {
                store.forget_session();
            }
            // Only clean up `state` if it still holds *this* session: a quiet
            // kill (new conversation, interrupt) may already have replaced or
            // cleared it by the time this task notices the pipe closed, and
            // must not be clobbered.
            let dropped = {
                let mut st = state.lock().await;
                if st
                    .session
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(&s.quiet_exit, &quiet))
                {
                    st.session = None;
                    st.in_flight = false;
                    std::mem::take(&mut st.pending)
                } else {
                    VecDeque::new()
                }
            };
            // A quiet exit (new conversation, interrupt) is handled by its
            // caller, including reporting its own dropped items — this is
            // only for an exit nobody asked for (crash, missing CLI, or
            // shutdown).
            if !quiet.load(Ordering::Relaxed) {
                report_undelivered(&store, &events, dropped);
                events.publish(PodiumEvent::AssistantEvent {
                    event: AssistantEvent::Exited,
                });
            }
        });
        Ok(Session {
            child,
            stdin,
            quiet_exit,
        })
    }
}

/// Write `item` straight into the live session, regardless of whether a turn
/// is already running — the behaviour from before the pending-input queue
/// existed, kept as the default (`queue_messages` off) for the user's own
/// messages. Callers only reach this with `state.pending` already empty, so
/// there is nothing to queue-jump: an in-progress turn's output can
/// interleave with this new one, which is the trade-off of the setting being
/// off. A write that fails, or no live session at all, falls back to queuing
/// rather than losing the item outright.
async fn deliver_bypassing_queue(
    state: &mut SessionState,
    item: PendingInput,
    store: &AssistantStore,
    events: &EventBus,
) {
    if let Some(session) = state.session.as_mut() {
        match write_turn(session, &item.text).await {
            Ok(()) => {
                state.in_flight = true;
                record_delivered(store, events, &item);
                return;
            }
            Err(e) => tracing::warn!("could not deliver an assistant message: {e}"),
        }
    }
    state.pending.push_back(item);
    publish_queue(state, events);
}

/// Deliver `item` right now if the session is idle and nothing is already
/// waiting, otherwise queue it (FIFO) — never queue-jumping, even a message
/// that could itself be written immediately still waits behind anything
/// already ahead of it. A write that fails (a session that died between the
/// caller's checks and this one) falls back to queuing rather than losing the
/// item outright.
async fn deliver_or_queue(
    state: &mut SessionState,
    item: PendingInput,
    store: &AssistantStore,
    events: &EventBus,
) {
    if !state.in_flight && state.pending.is_empty() {
        if let Some(session) = state.session.as_mut() {
            match write_turn(session, &item.text).await {
                Ok(()) => {
                    state.in_flight = true;
                    record_delivered(store, events, &item);
                    return;
                }
                Err(e) => tracing::warn!("could not deliver an assistant message: {e}"),
            }
        }
    }
    state.pending.push_back(item);
    if state.pending.len() > MAX_PENDING {
        state.pending.pop_front();
        let event = AssistantEvent::QueueNotice {
            text: format!(
                "[Podium] The waiting-message queue is full ({MAX_PENDING}); the oldest \
                 waiting message was dropped."
            ),
        };
        store.record(&event);
        events.publish(PodiumEvent::AssistantEvent { event });
    }
    publish_queue(state, events);
    if !state.in_flight {
        // Not actually busy — just deferred to something already queued (or
        // a session that only just came up) — so there is no `TurnEnd`
        // coming to drain it. Get the front moving now.
        drain_next(state, store, events).await;
    }
}

/// Deliver the oldest waiting item into an already-live, already-idle
/// session — what happens right after a `TurnEnd`, or once
/// [`AssistantManager::interrupt`] has respawned. One item per call, so
/// several queued items always become separate turns.
async fn drain_next(state: &mut SessionState, store: &AssistantStore, events: &EventBus) {
    if state.in_flight {
        return;
    }
    let Some(session) = state.session.as_mut() else {
        return;
    };
    let Some(item) = state.pending.pop_front() else {
        return;
    };
    match write_turn(session, &item.text).await {
        Ok(()) => {
            state.in_flight = true;
            record_delivered(store, events, &item);
            // The direct-write path (an immediately-delivered message) is
            // already shown by its own caller; this one was waiting and
            // nothing else announces its turn starting.
            if item.kind == PendingKind::User {
                events.publish(PodiumEvent::AssistantEvent {
                    event: AssistantEvent::QueueItemDelivered {
                        text: item.text.clone(),
                    },
                });
            }
        }
        Err(e) => {
            tracing::warn!("could not deliver a queued assistant message: {e}");
            state.pending.push_front(item);
        }
    }
    publish_queue(state, events);
}

/// Record/publish a just-delivered item the same way it always would have
/// been, had it never needed to wait: a user turn joins the transcript, a
/// nudge shows as a muted row, and a proposal-outcome notice stays invisible
/// (it is for the assistant, not the user).
fn record_delivered(store: &AssistantStore, events: &EventBus, item: &PendingInput) {
    match item.kind {
        PendingKind::User => store.append(TranscriptRole::User, item.text.clone()),
        PendingKind::Nudge => {
            let event = AssistantEvent::Notice {
                text: item.text.clone(),
            };
            store.record(&event);
            events.publish(PodiumEvent::AssistantEvent { event });
        }
        PendingKind::Notice => {}
    }
}

/// Publish the current queue as a snapshot for the UI.
fn publish_queue(state: &SessionState, events: &EventBus) {
    let items = state.pending.iter().map(QueuedMessage::from).collect();
    events.publish(PodiumEvent::AssistantEvent {
        event: AssistantEvent::QueueChanged { items },
    });
}

/// Tell the user which waiting messages never made it out, as a chat notice
/// — never a silent drop — and clear the queue on screen. A no-op when
/// nothing was waiting.
fn report_undelivered(store: &AssistantStore, events: &EventBus, dropped: VecDeque<PendingInput>) {
    if dropped.is_empty() {
        return;
    }
    let list = dropped
        .iter()
        .map(|p| format!("\"{}\"", truncate(&p.text, 60)))
        .collect::<Vec<_>>()
        .join(", ");
    let phrase = if dropped.len() == 1 {
        "this waiting message"
    } else {
        "these waiting messages"
    };
    let event = AssistantEvent::QueueNotice {
        text: format!(
            "[Podium] The conversation ended before {phrase} could be delivered: {list}."
        ),
    };
    store.record(&event);
    events.publish(PodiumEvent::AssistantEvent { event });
    events.publish(PodiumEvent::AssistantEvent {
        event: AssistantEvent::QueueChanged { items: Vec::new() },
    });
}

/// The system prompt the session is spawned with: who it is, plus whatever it
/// has written into its memory. Memory changes only reach a *new* session —
/// the running one already knows what it just wrote.
fn identity_prompt(memory: &str) -> String {
    let memory = memory.trim();
    if memory.is_empty() {
        return IDENTITY_PROMPT.to_string();
    }
    format!("{IDENTITY_PROMPT}\n\n# Your memory\n\n{memory}")
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

    /// A store backed by a real (temporary) data dir.
    fn store_in(dir: &Path) -> AssistantStore {
        let store = AssistantStore::new();
        store.set_dir(dir);
        store
    }

    #[test]
    fn the_store_records_prose_actions_and_failures_but_not_proposals() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());

        store.append(TranscriptRole::User, "what is open?".to_string());
        store.record(&AssistantEvent::Message {
            text: "two to-dos".to_string(),
        });
        store.record(&AssistantEvent::Tool {
            name: "list_todos".to_string(),
        });
        store.record(&AssistantEvent::ToolFailed {
            message: "project not found".to_string(),
        });
        store.record(&AssistantEvent::Proposal {
            id: ProposalId::new(),
            tool: "set_todo_archived".to_string(),
            summary: "{}".to_string(),
        });
        store.record(&AssistantEvent::TurnEnd { error: None });
        store.record(&AssistantEvent::Exited);

        let rows: Vec<_> = store
            .transcript()
            .into_iter()
            .map(|e| (e.role, e.text))
            .collect();
        assert_eq!(
            rows,
            vec![
                (TranscriptRole::User, "what is open?".to_string()),
                (TranscriptRole::Assistant, "two to-dos".to_string()),
                (TranscriptRole::Tool, "list_todos".to_string()),
                (TranscriptRole::Notice, "project not found".to_string()),
            ]
        );
    }

    #[test]
    fn a_reloaded_store_sees_the_persisted_conversation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        store.append(TranscriptRole::User, "remember this".to_string());
        store.record(&AssistantEvent::Started {
            session_id: "sess-7".to_string(),
        });

        let reloaded = store_in(dir.path());
        assert_eq!(reloaded.transcript().len(), 1);
        assert_eq!(reloaded.session_id(), Some("sess-7".to_string()));

        // Forgetting the session keeps the rows; clearing drops both.
        reloaded.forget_session();
        assert_eq!(reloaded.session_id(), None);
        assert_eq!(reloaded.transcript().len(), 1);
        reloaded.clear();
        assert!(store_in(dir.path()).transcript().is_empty());
    }

    #[test]
    fn the_transcript_keeps_only_the_most_recent_rows() {
        let store = AssistantStore::new();
        for i in 0..MAX_TRANSCRIPT_ENTRIES + 3 {
            store.append(TranscriptRole::User, i.to_string());
        }
        let rows = store.transcript();
        assert_eq!(rows.len(), MAX_TRANSCRIPT_ENTRIES);
        assert_eq!(rows[0].text, "3");
    }

    #[test]
    fn a_corrupt_conversation_file_starts_empty_instead_of_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("assistant.json"), "{not json").expect("write");
        let store = store_in(dir.path());
        assert!(store.transcript().is_empty());
        assert_eq!(store.session_id(), None);
    }

    #[test]
    fn memory_round_trips_through_the_file_and_can_be_cleared() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        assert_eq!(store.memory(), "");

        store.set_memory("Prefers short answers.").expect("set");
        assert_eq!(store.memory(), "Prefers short answers.");
        store.set_memory("").expect("clear");
        assert_eq!(store.memory(), "");
    }

    #[test]
    fn a_store_without_a_data_dir_cannot_hold_memory() {
        let store = AssistantStore::new();
        assert!(store.set_memory("anything").is_err());
    }

    #[test]
    fn memory_is_appended_to_the_identity_prompt_when_there_is_some() {
        assert_eq!(identity_prompt("   "), IDENTITY_PROMPT);
        let prompt = identity_prompt("Ships on Fridays.");
        assert!(prompt.starts_with(IDENTITY_PROMPT));
        assert!(prompt.contains("# Your memory"));
        assert!(prompt.ends_with("Ships on Fridays."));
    }

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
        let err = manager
            .send("   ", None, false)
            .await
            .expect_err("blank rejected");
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
        assert!(requires_proposal(mode, "remove_process"));
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
            None,
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
            None,
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
            None,
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
    fn proactivity_defaults_to_on_including_for_older_settings_files() {
        assert!(AssistantSettings::default().proactive);
        // An `agents.json` written before the field existed must not read as
        // "the user switched nudges off".
        let legacy: AssistantSettings =
            serde_json::from_str(r#"{"autonomy":"full"}"#).expect("legacy settings");
        assert!(legacy.proactive);
        assert_eq!(legacy.autonomy, AutonomyMode::Full);
    }

    #[test]
    fn queue_messages_defaults_off_including_for_older_settings_files() {
        assert!(!AssistantSettings::default().queue_messages);
        // An `agents.json` written before the field existed must read as the
        // pre-queue behaviour, not as "the user turned queuing on".
        let legacy: AssistantSettings =
            serde_json::from_str(r#"{"autonomy":"full"}"#).expect("legacy settings");
        assert!(!legacy.queue_messages);
    }

    #[test]
    fn the_day_start_summary_is_claimed_once_per_day() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        assert!(store.claim_day_start());
        assert!(!store.claim_day_start());

        // A restart must not re-ask, and neither must a new conversation.
        let reloaded = store_in(dir.path());
        assert!(!reloaded.claim_day_start());
        reloaded.clear();
        assert!(!reloaded.claim_day_start());
    }

    #[test]
    fn a_nudge_is_recorded_distinctly_from_a_real_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = store_in(dir.path());
        store.record(&AssistantEvent::Notice {
            text: "[Podium] The agent \"fix-login\" finished.".to_string(),
        });
        store.record(&AssistantEvent::ToolFailed {
            message: "project not found".to_string(),
        });
        let rows: Vec<_> = store
            .transcript()
            .into_iter()
            .map(|e| (e.role, e.text))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    TranscriptRole::Nudge,
                    "[Podium] The agent \"fix-login\" finished.".to_string()
                ),
                (TranscriptRole::Notice, "project not found".to_string()),
            ],
            "a nudge must replay as muted (Nudge), a real tool failure as a visible Notice"
        );
    }

    #[tokio::test]
    async fn a_nudge_without_a_session_is_queued_rather_than_dropped() {
        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();

        // No session, so nothing is spawned for it — but the UI still learns
        // it is waiting.
        manager
            .nudge("[Podium] an agent finished".to_string())
            .await;
        assert_eq!(
            manager.state.lock().await.pending.len(),
            1,
            "the nudge must wait for a session, not vanish"
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(PodiumEvent::AssistantEvent {
                event: AssistantEvent::QueueChanged { items }
            }) if items.len() == 1
        ));

        // …and it is dropped along with the conversation it was meant for.
        manager.new_conversation().await;
        assert!(manager.state.lock().await.pending.is_empty());
    }

    #[test]
    fn truncate_clips_on_a_char_boundary() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("héllo", 3), "hél…");
    }

    /// Push a bare pending item straight onto the queue, bypassing
    /// `send`/`nudge` — for tests that only care about edit/remove/cap
    /// behaviour, not delivery.
    async fn seed_pending(manager: &AssistantManager, kind: PendingKind, text: &str) -> PendingId {
        let mut state = manager.state.lock().await;
        let id = PendingId::new();
        state.pending.push_back(PendingInput {
            id,
            kind,
            text: text.to_string(),
        });
        id
    }

    #[tokio::test]
    async fn editing_a_waiting_user_message_changes_its_text() {
        let manager = AssistantManager::new(EventBus::new());
        let id = seed_pending(&manager, PendingKind::User, "first draft").await;

        manager.edit_pending(id, "final draft").await.expect("edit");

        let state = manager.state.lock().await;
        assert_eq!(state.pending[0].text, "final draft");
    }

    #[tokio::test]
    async fn a_nudge_or_notice_cannot_be_edited_or_removed() {
        let manager = AssistantManager::new(EventBus::new());
        let nudge_id = seed_pending(&manager, PendingKind::Nudge, "an agent finished").await;
        let notice_id = seed_pending(&manager, PendingKind::Notice, "proposal outcome").await;

        assert!(matches!(
            manager.edit_pending(nudge_id, "rewritten").await,
            Err(CoreError::QueuedMessageNotFound)
        ));
        assert!(matches!(
            manager.remove_pending(notice_id).await,
            Err(CoreError::QueuedMessageNotFound)
        ));
        // Both untouched.
        assert_eq!(manager.state.lock().await.pending.len(), 2);
    }

    #[tokio::test]
    async fn removing_a_waiting_message_means_nothing_is_delivered_for_it() {
        let manager = AssistantManager::new(EventBus::new());
        let id = seed_pending(&manager, PendingKind::User, "withdraw me").await;

        manager.remove_pending(id).await.expect("remove");
        assert!(manager.state.lock().await.pending.is_empty());

        // A second removal finds nothing left.
        assert!(matches!(
            manager.remove_pending(id).await,
            Err(CoreError::QueuedMessageNotFound)
        ));
    }

    #[tokio::test]
    async fn editing_or_removing_an_unknown_id_fails() {
        let manager = AssistantManager::new(EventBus::new());
        let unknown = PendingId::new();
        assert!(matches!(
            manager.edit_pending(unknown, "anything").await,
            Err(CoreError::QueuedMessageNotFound)
        ));
        assert!(matches!(
            manager.remove_pending(unknown).await,
            Err(CoreError::QueuedMessageNotFound)
        ));
    }

    #[tokio::test]
    async fn the_cap_drops_the_oldest_waiting_message_and_reports_it() {
        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();
        // Busy with no live session: every nudge just queues.
        for i in 0..MAX_PENDING + 1 {
            manager.nudge(format!("nudge {i}")).await;
        }
        let state = manager.state.lock().await;
        assert_eq!(state.pending.len(), MAX_PENDING);
        assert_eq!(state.pending.front().unwrap().text, "nudge 1");
        drop(state);

        let mut saw_drop_notice = false;
        while let Ok(PodiumEvent::AssistantEvent { event }) = rx.try_recv() {
            if let AssistantEvent::QueueNotice { text } = event {
                assert!(text.contains("dropped"), "{text}");
                saw_drop_notice = true;
            }
        }
        assert!(saw_drop_notice, "the drop must be reported in the chat");
    }

    #[tokio::test]
    async fn a_turn_end_with_nothing_queued_publishes_no_queue_event() {
        // `drain_next` on an empty queue must be a true no-op — otherwise
        // every ordinary turn would spam a pointless `QueueChanged`.
        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();
        drain_next(
            &mut *manager.state.lock().await,
            &manager.store,
            &manager.events,
        )
        .await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn interrupt_with_nothing_running_is_a_no_op() {
        let manager = AssistantManager::new(EventBus::new());
        let mut rx = manager.events.subscribe();
        manager.interrupt(None).await.expect("interrupt");
        assert!(rx.try_recv().is_err(), "nothing to stop, nothing to report");
    }
}
