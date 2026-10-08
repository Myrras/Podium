//! The single public API surface over projects and PTY-backed processes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::agent::session_memory::{self, RememberedSession, SessionMemoryStore};
use crate::agent::settings::{self, AgentSettings, AgentSettingsStore, MergeMode};
use crate::agent::{AdapterInfo, AdapterRegistry, AgentLaunchCtx, McpConnectInfo, RestartCommands};
use crate::assignment::AssignedAgent;
use crate::assistant::{
    AssistantConfig, AssistantManager, AssistantPolling, Attachment, AutonomyMode, ProposalAction,
    ProposalInfo, TranscriptEntry,
};
use crate::boards::{BoardStore, CachedBoard};
use crate::config::AgentsConfig;
use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};
use crate::ids::{CommentId, LinkId, TodoId};
use crate::ids::{PendingId, ProcessId, ProjectId, ProposalId, ResponseId, ScratchpadId};
use crate::platform::WslBackend;
use crate::process::pty::{ExitCallback, PtyProcess, TermChunk};
use crate::process::scrollback::{OutputActivity, ScrollbackBuffer};
use crate::process::supervisor::{RestartState, SupervisorConfig};
use crate::process::{ProcessInfo, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy};
use crate::project::{self, ConfiguredProcess, ProjectSections};
use crate::responses::{ResponseItem, ResponseKind, ResponseStore};
use crate::scratchpad::{ScratchpadInfo, ScratchpadStore};
use crate::todo::{TodoInfo, TodoStore};
use crate::worktree::WorktreeInfo;

/// Capacity (in chunks) of each process's raw-output broadcast channel.
const CHUNK_CHANNEL_CAPACITY: usize = 1024;
/// How long `restart_process` waits for the old instance to exit.
const RESTART_STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Recursion guard: agents can spawn agents over MCP, so cap how many can be
/// active (running or stopping) in one project at a time.
const MAX_AGENTS_PER_PROJECT: usize = 8;

/// Built-in fallback adapter when neither the project nor the global settings
/// pin a default.
const DEFAULT_ADAPTER_ID: &str = "claude-code";

const LOCK_POISONED: &str = "orchestrator lock poisoned";

/// Longest Responses item text kept; the rest is clipped.
const MAX_RESPONSE_TEXT_CHARS: usize = 500;

/// A resumed agent that crashes within this long after starting is taken to
/// have failed to resume (the CLI refusing the session), so its next start is
/// fresh instead of the same `--resume` again.
const RESUME_FAILURE_WINDOW: Duration = Duration::from_secs(5);

/// Read-only snapshot of an open project, for listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
    /// Sidebar badge initials (from `podium.yml` or derived from the name).
    pub icon_initials: String,
    /// Readable `podium.yml` error, if the last (re)load failed.
    pub config_error: Option<String>,
    /// True when a user-set display-name override is in effect (the command
    /// layer persists the name in this case, or clears it when false).
    pub renamed: bool,
    /// Which optional sidebar sections the project shows.
    pub sections: ProjectSections,
    /// User-picked badge colour (`#rrggbb`), persisted in `workspace.json`;
    /// `None` keeps the theme accent.
    pub color: Option<String>,
    /// True only in WSL-backend mode, for a Windows-drive project that runs
    /// natively because of the `nativeDriveProjects` opt-in. The terminal
    /// shell setting is meant for the backend's shell, so the UI leaves it
    /// out for these and lets the native default (PowerShell) open instead.
    pub native_in_wsl_mode: bool,
}

struct ProjectHandle {
    /// The `podium.yml`/folder-derived name (updated on config reload).
    name: String,
    root: PathBuf,
    /// Badge initials from `podium.yml` or derived from `name`.
    icon_initials: String,
    config_error: Option<String>,
    agents: AgentsConfig,
    /// User-set display-name override (persisted in `workspace.json`). When
    /// set it wins over `name` and drives the badge initials; a config reload
    /// leaves it untouched.
    name_override: Option<String>,
    /// Which optional sidebar sections are shown (persisted in `workspace.json`).
    sections: ProjectSections,
    /// User-picked badge colour (`#rrggbb`), persisted in `workspace.json`.
    color: Option<String>,
    /// The backend this project runs on, decided once at open from its root
    /// ([`crate::platform::wsl_for`]) and used for everything in it — agent
    /// launch quoting, the PTY spawn, the launch gate, worktree git, the
    /// adapter probe — so one project can never mix backends.
    wsl: Option<&'static WslBackend>,
}

impl ProjectHandle {
    fn info(&self, id: ProjectId) -> ProjectInfo {
        let (name, icon_initials) = match &self.name_override {
            Some(n) => (n.clone(), project::derive_icon_initials(n)),
            None => (self.name.clone(), self.icon_initials.clone()),
        };
        ProjectInfo {
            id,
            name,
            root: self.root.clone(),
            icon_initials,
            config_error: self.config_error.clone(),
            renamed: self.name_override.is_some(),
            sections: self.sections,
            color: self.color.clone(),
            native_in_wsl_mode: crate::platform::wsl().is_some() && self.wsl.is_none(),
        }
    }
}

struct ManagedProcess {
    spec: ProcessSpec,
    project_id: ProjectId,
    scrollback: Arc<Mutex<ScrollbackBuffer>>,
    pty: Option<Arc<PtyProcess>>,
    status: ProcessStatus,
    user_stopped: bool,
    chunk_tx: broadcast::Sender<TermChunk>,
    /// Defined in `podium.yml` (replaced on config reload) vs added manually.
    from_config: bool,
    /// Supervisor bookkeeping: backoff, breaker window, pending restart.
    restart: RestartState,
    /// When the current incarnation started, for backoff reset.
    started_at: Option<Instant>,
    /// When the UI last saw an input prompt on this agent's screen — see
    /// [`Orchestrator::report_agent_waiting`]. `None` until it reports one.
    waiting_since: Option<Instant>,
    /// The per-spawn args it was started with (`args_override`), remembered
    /// with its session so a restore runs on the same model.
    spawn_args: Option<Vec<String>>,
    /// The `worktree` flag and name it was spawned with, remembered with its
    /// session so a restore gets the same identity prompt.
    spawn_worktree: bool,
    spawn_worktree_name: Option<String>,
    /// For an agent: how a manual start after it exited is spelled, and the
    /// CLI session it is on (see [`AgentSession`]). `None` for everything
    /// else.
    session: Option<AgentSession>,
    /// The colour an exited agent gave up, which its next start takes back
    /// unless another live agent holds it by then.
    last_color: Option<String>,
}

/// An agent's CLI session: the commands a restart picks from
/// ([`LaunchPlan::restart`]) and the id of the session it is on — the
/// process id at spawn, a new one after each fresh restart. `spec.command`
/// itself is never swapped, so the choice is made afresh every start.
#[derive(Clone)]
struct AgentSession {
    commands: RestartCommands,
    id: String,
    /// The current run resumed `id` (rather than starting it fresh).
    resumed: bool,
    /// Resuming `id` crashed right away (see [`RESUME_FAILURE_WINDOW`]), so
    /// it is not tried again: the next start is fresh.
    resume_failed: bool,
}

/// What a restart of an exited agent runs, from [`restart_launch`]. The
/// session it names becomes the agent's current one only once the start has
/// succeeded, so a start that loses a race cannot overwrite the winner's.
struct Relaunch {
    command: String,
    session_id: String,
    resumed: bool,
}

impl ManagedProcess {
    fn info(&self, id: ProcessId) -> ProcessInfo {
        ProcessInfo {
            id,
            project_id: self.project_id,
            name: self.spec.name.clone(),
            kind: self.spec.kind.clone(),
            status: self.status.clone(),
            restart_policy: self.spec.restart_policy,
            command: self.spec.command.clone(),
            worktree: crate::process::worktree_name_from_cwd(&self.spec.cwd),
            color: self.spec.color.clone(),
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.status,
            ProcessStatus::Running { .. } | ProcessStatus::Stopping
        )
    }
}

#[derive(Default)]
struct Inner {
    projects: HashMap<ProjectId, ProjectHandle>,
    /// Sidebar display order — the source of truth for [`list_projects`]
    /// ordering (the `projects` map itself is unordered).
    order: Vec<ProjectId>,
    processes: HashMap<ProcessId, ManagedProcess>,
    /// Which agent process is working on which to-do. Runtime-only (process
    /// ids do not survive a restart), so it is never persisted. An agent that
    /// exits keeps its entries (a restart picks the work back up); they are
    /// cleared when the agent is removed or its project closes.
    todo_assignments: HashMap<TodoId, ProcessId>,
    /// Which agent process is working on which scratchpad. Mirrors
    /// `todo_assignments` exactly (runtime-only, cleared the same way).
    scratchpad_assignments: HashMap<ScratchpadId, ProcessId>,
}

/// Who a verified cross-session message came from, as resolved by
/// [`Orchestrator::verify_peer_sender`].
#[derive(Debug, Clone)]
pub enum PeerSender {
    /// The personal assistant's own session.
    Assistant,
    /// A live agent session Podium spawned.
    Agent {
        // Boxed: `ProcessInfo` dwarfs the unit variant otherwise.
        process: Box<ProcessInfo>,
        /// Display name of the project it runs in; `None` if that project has
        /// since been closed.
        project: Option<String>,
    },
}

/// Owns all projects and processes; every UI/adapter goes through this.
pub struct Orchestrator {
    inner: Arc<Mutex<Inner>>,
    events: EventBus,
    supervisor: SupervisorConfig,
    adapters: AdapterRegistry,
    /// How agents reach the built-in MCP server; `None` until it is running.
    mcp: Mutex<Option<McpConnectInfo>>,
    /// Per-project to-do lists, keyed by project root (survives restarts).
    todos: TodoStore,
    /// Per-project scratchpads, keyed by project root (survives restarts).
    scratchpads: ScratchpadStore,
    /// The last Git board per project and scope (survives restarts).
    boards: BoardStore,
    /// Global, cross-project agent settings (command override + default args
    /// per adapter, plus the merge mode).
    agent_settings: AgentSettingsStore,
    /// Agent sessions to resume on the next open of their project, keyed by
    /// root (`AgentSettings.remember_sessions`).
    session_memory: SessionMemoryStore,
    /// Set as [`Self::shutdown`] begins, so a restore still running on a
    /// project open spawns nothing after the stop sweep.
    shutting_down: AtomicBool,
    /// The workspace-wide personal assistant session (chat panel). Also
    /// holds the Responses list ([`Self::responses`]).
    assistant: AssistantManager,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl Orchestrator {
    pub fn new() -> Self {
        Self::with_supervisor_config(SupervisorConfig::default())
    }

    /// Build with custom supervisor timings (tests use millisecond backoffs).
    pub fn with_supervisor_config(supervisor: SupervisorConfig) -> Self {
        let events = EventBus::new();
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            assistant: AssistantManager::new(events.clone()),
            events,
            supervisor,
            adapters: AdapterRegistry::default(),
            mcp: Mutex::new(None),
            todos: TodoStore::new(),
            scratchpads: ScratchpadStore::new(),
            boards: BoardStore::new(),
            agent_settings: AgentSettingsStore::new(),
            session_memory: SessionMemoryStore::new(),
            shutting_down: AtomicBool::new(false),
        }
    }

    /// Replace the agent adapter registry (tests inject fakes).
    pub fn with_adapters(mut self, adapters: AdapterRegistry) -> Self {
        self.adapters = adapters;
        self
    }

    /// Record how spawned agents reach the built-in MCP server. Until this
    /// is called, agents launch without MCP wiring.
    pub fn set_mcp_connect_info(&self, info: McpConnectInfo) {
        *self.mcp.lock().expect(LOCK_POISONED) = Some(info);
    }

    /// Point the to-do store at its backing file (app data dir) and load it.
    /// Until this is called, to-dos are held in memory only.
    pub fn set_todos_path(&self, path: PathBuf) {
        self.todos.set_path(path);
    }

    /// Point the Git board cache at its backing file (app data dir) and load
    /// it. Until this is called, boards are cached in memory only.
    pub fn set_boards_path(&self, path: PathBuf) {
        self.boards.set_path(path);
    }

    /// Point the scratchpad store at its backing file (app data dir) and
    /// load it. Until this is called, scratchpads are held in memory only.
    pub fn set_scratchpads_path(&self, path: PathBuf) {
        self.scratchpads.set_path(path);
    }

    /// Point the Responses list at its backing file (app data dir) and load
    /// it. Until this is called, items are held in memory only.
    pub fn set_responses_path(&self, path: PathBuf) {
        self.responses().set_path(path);
    }

    /// The assistant's Responses list (survives restarts and new
    /// conversations). Kept in the assistant's store so its queue can put
    /// back the item of a reply it never delivered.
    fn responses(&self) -> &ResponseStore {
        self.assistant.store().responses()
    }

    /// Point the assistant at its CLI binary, working directory and the dir
    /// holding its transcript + memory. Until this is called it uses the
    /// `claude` on `PATH`, Podium's own cwd, and keeps the conversation in
    /// memory only.
    pub fn configure_assistant(&self, config: AssistantConfig) {
        self.assistant.configure(config);
    }

    /// Send a chat turn to the personal assistant, spawning its session on
    /// first use. Replies arrive as [`PodiumEvent::AssistantEvent`]s.
    ///
    /// Returns whether it went straight into the session (`false` = it is
    /// waiting behind a turn already running).
    pub async fn assistant_send(&self, text: &str) -> CoreResult<bool> {
        self.assistant_send_with_attachments(text, Vec::new()).await
    }

    /// Send a chat turn with attachments: an image always rides along inline
    /// (read from disk first when only a path was given), anything else is
    /// an absolute path for the assistant's own `Read` tool, or — with no
    /// file on disk — inline text. An attachment over the limits is refused
    /// on its own (a chat notice names it); the rest of the message still
    /// goes.
    pub async fn assistant_send_with_attachments(
        &self,
        text: &str,
        attachments: Vec<Attachment>,
    ) -> CoreResult<bool> {
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        self.assistant.send(text, attachments, mcp).await
    }

    /// The persisted chat rows the panel replays after a restart.
    pub fn assistant_transcript(&self) -> Vec<TranscriptEntry> {
        self.assistant.store().transcript()
    }

    /// Start a fresh conversation: the session is killed and the transcript,
    /// session id and pending proposals are dropped. Memory is kept — that is
    /// the point of it.
    pub async fn assistant_new_conversation(&self) {
        self.assistant.new_conversation().await;
    }

    /// What the assistant remembers across conversations (`assistant-memory.md`).
    pub fn assistant_memory(&self) -> String {
        self.assistant.store().memory()
    }

    /// Replace that memory; an empty `content` clears it. The running session
    /// keeps the prompt it started with — the new text lands on its next spawn.
    pub fn set_assistant_memory(&self, content: &str) -> CoreResult<()> {
        self.assistant.store().set_memory(content)
    }

    /// Tag identifying the assistant's own CLI session in an inbound
    /// cross-session message's sender name (see [`crate::agent::peer_tag`]).
    pub fn assistant_peer_tag(&self) -> String {
        self.assistant.peer_tag().to_string()
    }

    /// Resolve a cross-session message's sender name to the Podium session it
    /// belongs to, or `None` when no live session of Podium's own answers to
    /// it.
    ///
    /// This is the whole of Podium's sender verification, and it deliberately
    /// lives here rather than in a prompt: the answer comes from Podium's own
    /// process registry plus the assistant's per-run tag, reached over the
    /// bearer-token-authenticated MCP channel. Nothing the message asserts
    /// about its origin is consulted, so a message cannot argue its way into
    /// being verified.
    pub fn verify_peer_sender(&self, sender_name: &str) -> Option<PeerSender> {
        if crate::agent::peer_name_carries(sender_name, &self.assistant_peer_tag()) {
            return Some(PeerSender::Assistant);
        }
        self.list_processes(None)
            .into_iter()
            // Only agents get a Podium-stamped session name, and only a live
            // one can be talking: a tag from an exited session is stale.
            .find(|p| {
                matches!(p.kind, ProcessKind::Agent { .. })
                    && matches!(p.status, ProcessStatus::Running { .. })
                    && crate::agent::peer_name_carries(
                        sender_name,
                        &crate::agent::peer_tag(&p.id.to_string()),
                    )
            })
            .map(|p| PeerSender::Agent {
                project: self
                    .list_projects()
                    .into_iter()
                    .find(|proj| proj.id == p.project_id)
                    .map(|proj| proj.name),
                process: Box::new(p),
            })
    }

    /// How much the assistant may do without asking (Settings → Assistant).
    pub fn assistant_autonomy(&self) -> AutonomyMode {
        self.agent_settings.get().assistant.autonomy
    }

    /// Change the assistant's autonomy mode; returns what was stored.
    ///
    /// Moving into or out of `propose-all` also flips whether the assistant's
    /// CLI session may use `SendMessage` — the one way it could reach an
    /// agent without passing the gate — which replaces the session (see
    /// [`AssistantManager::set_send_message_blocked`]).
    pub async fn set_assistant_autonomy(&self, mode: AutonomyMode) -> CoreResult<AutonomyMode> {
        let stored = self
            .agent_settings
            .set_assistant_autonomy(mode)?
            .assistant
            .autonomy;
        self.assistant
            .set_send_message_blocked(stored == AutonomyMode::ProposeAll)
            .await;
        Ok(stored)
    }

    /// Whether Podium may nudge the assistant unprompted (Settings →
    /// Assistant). Off silences [`crate::proactivity`] entirely.
    pub fn assistant_proactivity(&self) -> bool {
        self.agent_settings.get().assistant.proactive
    }

    /// Turn proactive nudges on or off; returns what was stored.
    pub fn set_assistant_proactivity(&self, enabled: bool) -> CoreResult<bool> {
        Ok(self
            .agent_settings
            .set_assistant_proactive(enabled)?
            .assistant
            .proactive)
    }

    /// Hand the assistant something Podium noticed. Delivered to a live
    /// session, or — with none — into one started for it, so the user hears
    /// about a finished agent when it finishes rather than whenever they next
    /// happen to type.
    pub async fn assistant_nudge(&self, text: String) {
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        self.assistant.nudge(text, mcp).await;
    }

    /// Ask the assistant to sweep the user's external sources. Like a nudge
    /// it starts the session if there is none, but it stays out of the chat:
    /// only an answer worth having is ever shown (see
    /// [`crate::assistant::poll_prompt`]).
    pub async fn assistant_poll(&self, text: String) {
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        self.assistant.poll(text, mcp).await;
    }

    /// The assistant's external-sweep settings (Settings → Assistant).
    pub fn assistant_polling(&self) -> AssistantPolling {
        self.agent_settings.get().assistant.into()
    }

    /// Change them; returns what was stored. A new interval is picked up by
    /// the running watcher on its next tick — no restart.
    pub fn set_assistant_polling(&self, polling: AssistantPolling) -> CoreResult<AssistantPolling> {
        Ok(self
            .agent_settings
            .set_assistant_polling(polling)?
            .assistant
            .into())
    }

    /// Ask the assistant for a day-start summary (the panel's button). Goes
    /// in as a normal user turn rather than a nudge: it is something the user
    /// asked for, so it belongs in the transcript and always wants an answer.
    pub async fn assistant_day_start(&self) -> CoreResult<bool> {
        self.assistant_send(crate::assistant::DAY_START_PROMPT)
            .await
    }

    /// Running agents that have printed nothing for at least `min_idle` —
    /// what [`crate::proactivity`] reads as "probably waiting on input".
    pub fn idle_agents(&self, min_idle: Duration) -> Vec<ProcessInfo> {
        let now = Instant::now();
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .processes
            .iter()
            .filter(|(_, p)| matches!(p.spec.kind, ProcessKind::Agent { .. }))
            .filter(|(_, p)| matches!(p.status, ProcessStatus::Running { .. }))
            .filter_map(|(id, p)| {
                // An agent that never printed anything is measured from its
                // start, so a session stuck on its first prompt still counts.
                let last = p
                    .scrollback
                    .lock()
                    .expect(LOCK_POISONED)
                    .last_append_at()
                    .or(p.started_at)?;
                (now.duration_since(last) >= min_idle).then(|| p.info(*id))
            })
            .collect()
    }

    /// How recently and how steadily each running process has been printing,
    /// keyed by process id — what the UI polls for its "working…" indicator.
    /// Read off the scrollback, so it covers processes whose terminal was
    /// never opened; a process that never printed is left out.
    pub fn output_activity(&self) -> HashMap<ProcessId, OutputActivity> {
        let now = Instant::now();
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .processes
            .iter()
            .filter(|(_, p)| matches!(p.status, ProcessStatus::Running { .. }))
            .filter_map(|(id, p)| {
                let activity = p
                    .scrollback
                    .lock()
                    .expect(LOCK_POISONED)
                    .output_activity(now)?;
                Some((*id, activity))
            })
            .collect()
    }

    /// The UI saw an input prompt on this agent's screen (a permission
    /// question, a credential request). Recorded so [`crate::proactivity`]
    /// can tell the user *now* instead of waiting out its silence timer —
    /// which is a blunt proxy: it cannot tell "waiting for you" from
    /// "finished thinking". Anything that is not a running agent is ignored.
    ///
    /// An agent blocked on a permission prompt cannot call
    /// `report_to_assistant` itself — it is blocked *inside* the harness that
    /// would make the call — so this screen reading is the only signal there
    /// is.
    pub fn report_agent_waiting(&self, id: ProcessId) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let Some(process) = inner.processes.get_mut(&id) else {
            return;
        };
        if matches!(process.spec.kind, ProcessKind::Agent { .. })
            && matches!(process.status, ProcessStatus::Running { .. })
        {
            process.waiting_since = Some(Instant::now());
        }
    }

    /// Running agents with a reported prompt that have printed nothing
    /// since. Output after the report means the prompt was answered (or
    /// misread), so the flag simply stops counting — no clearing needed. An
    /// agent that redraws its prompt would fall out the same way and land
    /// back on the silence timer, which is the safe direction to fail.
    pub fn agents_awaiting_input(&self) -> Vec<ProcessInfo> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .processes
            .iter()
            .filter(|(_, p)| matches!(p.spec.kind, ProcessKind::Agent { .. }))
            .filter(|(_, p)| matches!(p.status, ProcessStatus::Running { .. }))
            .filter_map(|(id, p)| {
                let since = p.waiting_since?;
                let last = p.scrollback.lock().expect(LOCK_POISONED).last_append_at();
                match last {
                    Some(last) if last > since => None,
                    _ => Some(p.info(*id)),
                }
            })
            .collect()
    }

    /// Park a tool call the autonomy gate held back, for the user to decide
    /// on. Called from the MCP layer, which owns the deferred action.
    /// `project` is the project the call acts on, if any, so closing that
    /// project can invalidate the proposal.
    pub fn propose_assistant_action(
        &self,
        tool: String,
        summary: String,
        project: Option<ProjectId>,
        action: ProposalAction,
    ) -> ProposalId {
        self.assistant.propose(tool, summary, project, action)
    }

    /// The assistant's proposals still awaiting the user, oldest first.
    pub fn assistant_proposals(&self) -> Vec<ProposalInfo> {
        self.assistant.proposals()
    }

    /// Confirm or reject one assistant proposal. A confirmed proposal runs its
    /// stored action now; the assistant is told the outcome either way.
    pub async fn resolve_assistant_proposal(
        &self,
        id: ProposalId,
        approved: bool,
    ) -> CoreResult<()> {
        self.assistant.resolve_proposal(id, approved).await
    }

    /// Change a waiting user message's text before it is delivered.
    pub async fn assistant_queue_edit(&self, id: PendingId, text: &str) -> CoreResult<()> {
        self.assistant.edit_pending(id, text).await
    }

    /// Push a waiting user message into the running turn now, instead of
    /// letting it wait for its own.
    pub async fn assistant_queue_send_now(&self, id: PendingId) -> CoreResult<()> {
        self.assistant.send_pending_now(id).await
    }

    /// Withdraw a waiting user message so nothing is delivered for it.
    pub async fn assistant_queue_remove(&self, id: PendingId) -> CoreResult<()> {
        self.assistant.remove_pending(id).await
    }

    /// Stop the assistant's current turn, if any, and immediately hand the
    /// next waiting message over rather than for a `TurnEnd` that would never
    /// come. With nothing running this is a no-op.
    pub async fn assistant_interrupt(&self) -> CoreResult<()> {
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        self.assistant.interrupt(mcp).await
    }

    /// The assistant's Responses list, newest first.
    pub fn assistant_responses(&self) -> Vec<ResponseItem> {
        self.responses().list()
    }

    /// File one Responses item (the assistant's `add_response`). Agent,
    /// project and to-do names are resolved now, so the item still reads
    /// right after the agent exits or the project closes. The project comes
    /// from the agent when it is live, else from `project_id`. A `todo_id`
    /// is refused without a `project_id` or `process_id` to find it by; a
    /// project or to-do that is gone (including an agent that exited, so
    /// there is no project to look in) is dropped from the item rather than
    /// refusing it.
    /// A newer item about the same agent or to-do replaces the open one.
    /// An agent that is already gone keeps its id as the supersede key but
    /// loses its name: it may well have exited between the nudge and this
    /// call, which is no reason to drop the item.
    pub fn add_assistant_response(
        &self,
        kind: ResponseKind,
        text: &str,
        process_id: Option<ProcessId>,
        project_id: Option<ProjectId>,
        todo_id: Option<TodoId>,
        source: Option<&str>,
    ) -> CoreResult<ResponseItem> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "response text must not be blank".to_string(),
            ));
        }
        if todo_id.is_some() && project_id.is_none() && process_id.is_none() {
            return Err(CoreError::InvalidInput(
                "todo_id needs project_id (or the process_id of an agent)".to_string(),
            ));
        }
        let process =
            process_id.and_then(|id| self.list_processes(None).into_iter().find(|p| p.id == id));
        // Responses are about agents: a terminal's or service's id would key
        // and label the item after the wrong thing. (An id that is gone is
        // fine — see above.)
        if process
            .as_ref()
            .is_some_and(|p| !matches!(p.kind, ProcessKind::Agent { .. }))
        {
            return Err(CoreError::InvalidInput(
                "process_id must be an agent's".to_string(),
            ));
        }
        // A live agent's own project wins over `project_id`: the agent is
        // what the item is about, so a mismatched id would mislabel it.
        let project_id = process.as_ref().map(|p| p.project_id).or(project_id);
        // Context that has gone (project closed, to-do removed) is dropped,
        // not an error — like a vanished agent, it is no reason to lose the
        // item; the ids still work as supersede keys.
        let project =
            project_id.and_then(|id| self.list_projects().into_iter().find(|p| p.id == id));
        let todo = match (todo_id, &project) {
            (Some(id), Some(project)) => self.todos.get(project.id, &project.root, id),
            _ => None,
        };
        let item = ResponseItem {
            id: ResponseId::new(),
            kind,
            // Capped like the list itself: the assistant writes these from
            // content it read, and the panel row is meant to be a line or two.
            text: crate::assistant::truncate(text, MAX_RESPONSE_TEXT_CHARS),
            // Names are capped like the to-do and source: they travel into
            // the label a reply carries back to the session.
            project: project.map(|p| crate::assistant::truncate(&p.name, 40)),
            agent: process
                .as_ref()
                .map(|p| crate::assistant::truncate(&p.name, 40)),
            agent_color: process.as_ref().and_then(|p| p.color.clone()),
            todo: todo.map(|t| crate::assistant::truncate(&t.text, 60)),
            source: source
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| crate::assistant::truncate(s, 40)),
            process_id,
            todo_id,
            created_at: Utc::now(),
        };
        self.responses()
            .add(item.clone(), |items| self.publish_responses(items))?;
        Ok(item)
    }

    /// Mark one Responses item as read: it is simply removed.
    pub fn dismiss_assistant_response(&self, id: ResponseId) -> CoreResult<()> {
        self.responses()
            .remove(id, |items| self.publish_responses(items))?;
        Ok(())
    }

    /// Answer one Responses item: `text` goes to the assistant as the user's
    /// message, carrying which item it answers, and the item is removed.
    /// Taken off the list *before* sending, so a double submit finds nothing
    /// the second time instead of sending the reply twice; a send that fails
    /// puts it back to try again. Returns delivered-or-queued, as
    /// [`assistant_send`](Self::assistant_send).
    pub async fn reply_assistant_response(&self, id: ResponseId, text: &str) -> CoreResult<bool> {
        if text.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "reply must not be blank".to_string(),
            ));
        }
        let item = self
            .responses()
            .remove(id, |items| self.publish_responses(items))?;
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        match self.assistant.send_reply(text, item.clone(), mcp).await {
            Ok(delivered) => Ok(delivered),
            Err(e) => {
                self.responses()
                    .restore(item, |items| self.publish_responses(items));
                Err(e)
            }
        }
    }

    fn publish_responses(&self, items: &[ResponseItem]) {
        crate::assistant::publish_responses(&self.events, items);
    }

    /// Point the global agent settings at their backing file (app data dir)
    /// and load them. Until this is called, settings are held in memory only.
    pub fn set_agent_settings_path(&self, path: PathBuf) {
        self.agent_settings.set_path(path);
        // The assistant's first session must already match the stored mode.
        self.assistant
            .preset_send_message_blocked(self.assistant_autonomy() == AutonomyMode::ProposeAll);
    }

    /// Point the remembered-sessions store at its backing file (app data
    /// dir) and load it. Until this is called it is in-memory only.
    pub fn set_session_memory_path(&self, path: PathBuf) {
        self.session_memory.set_path(path);
    }

    /// A snapshot of the global agent settings.
    pub fn agent_settings(&self) -> AgentSettings {
        self.agent_settings.get()
    }

    /// Set how global default args combine with a project's `agents.extra_args`.
    pub fn set_agent_merge_mode(&self, mode: MergeMode) -> CoreResult<AgentSettings> {
        self.agent_settings.set_merge_mode(mode)
    }

    /// Set whether agents are told to offer an isolated git worktree before
    /// their first code change.
    pub fn set_agent_suggest_worktree(&self, enabled: bool) -> CoreResult<AgentSettings> {
        self.agent_settings.set_suggest_worktree(enabled)
    }

    /// Toggle whether agent sessions are remembered across app restarts.
    /// Turning it off forgets every session captured so far, so none of them
    /// can come back when it is turned on again.
    pub fn set_agent_remember_sessions(&self, enabled: bool) -> CoreResult<AgentSettings> {
        let settings = self.agent_settings.set_remember_sessions(enabled)?;
        if !enabled {
            self.session_memory.clear_all()?;
        }
        Ok(settings)
    }

    /// Set (or clear) the global default adapter used by bare spawns. A blank
    /// id clears it (back to the built-in default).
    pub fn set_agent_default_adapter(
        &self,
        adapter_id: Option<String>,
    ) -> CoreResult<AgentSettings> {
        self.agent_settings.set_default_adapter(adapter_id)
    }

    /// Set (or clear) one adapter's global command override + default args.
    pub fn set_agent_override(
        &self,
        adapter_id: &str,
        command: Option<String>,
        default_args: Vec<String>,
    ) -> CoreResult<AgentSettings> {
        self.agent_settings
            .set_override(adapter_id, command, default_args)
    }

    /// Open the directory at `path` as a project, loading `podium.yml` when
    /// present. A broken config still opens the project (with the folder
    /// name) and surfaces the error via [`ProjectInfo::config_error`].
    pub async fn open_project(&self, path: PathBuf) -> CoreResult<ProjectId> {
        if !path.is_dir() {
            return Err(CoreError::InvalidInput(format!(
                "not a directory: {}",
                path.display()
            )));
        }
        // Before switching to `dunce` below, note this folder's old (pre-fix)
        // canonical identity, if different — any to-dos/scratchpads stored
        // under it are migrated onto the new one once `path` is known.
        let legacy_root = project::legacy_canonical_root(&path);
        // Canonicalize so the same folder reached via different spellings
        // (symlinks, trailing slash, `..` segments) maps to one identity.
        // `dunce::canonicalize`, not `std::fs::canonicalize`: on Windows the
        // std version returns a `\\?\`-prefixed verbatim path, and that path
        // becomes this project's (and its agents') spawn `cwd`
        // (`process::pty::PtyProcess::spawn` → `CommandBuilder::cwd`).
        // `CreateProcessW`'s `lpCurrentDirectory` does not accept the `\\?\`
        // prefix, so every agent launch failed with `ERROR_ACCESS_DENIED`
        // (os error 5). `dunce` resolves symlinks the same way but keeps the
        // legacy path form when the verbatim form isn't required.
        let path = dunce::canonicalize(&path).unwrap_or(path);
        if let Some(legacy_root) = &legacy_root {
            self.todos.migrate_legacy_root(&path, legacy_root);
            self.scratchpads.migrate_legacy_root(&path, legacy_root);
        }
        // Idempotent: opening an already-open folder returns the existing
        // project rather than minting a duplicate record. This is what keeps
        // a double startup restore (e.g. React StrictMode invoking the effect
        // twice) from producing two sidebar entries for one project. The
        // check and the reservation below happen under the same lock
        // acquisition, atomically with each other: the config load is async,
        // so if the reservation happened only after that `.await`, two
        // concurrent opens of the same folder could both pass the check
        // before either had inserted, minting two records for one folder.
        let (id, is_new) = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            if let Some(existing) = inner
                .projects
                .iter()
                .find(|(_, p)| p.root == path)
                .map(|(id, _)| *id)
            {
                (existing, false)
            } else {
                let id = ProjectId::new();
                let name = project::folder_name(&path);
                let icon_initials = project::derive_icon_initials(&name);
                inner.projects.insert(
                    id,
                    ProjectHandle {
                        name,
                        root: path.clone(),
                        icon_initials,
                        config_error: None,
                        agents: AgentsConfig::default(),
                        name_override: None,
                        sections: ProjectSections::default(),
                        color: None,
                        wsl: crate::platform::wsl_for(&path),
                    },
                );
                inner.order.push(id);
                (id, true)
            }
        };
        if !is_new {
            return Ok(id);
        }
        let root = path.clone();
        let loaded = tokio::task::spawn_blocking(move || project::load_project_config(&root))
            .await
            .map_err(|e| CoreError::Config(format!("config load task failed: {e}")))?;
        let (name, icon_initials, config_error, configured, agents) = match loaded {
            Ok(Some(cfg)) => (cfg.name, cfg.icon_initials, None, cfg.processes, cfg.agents),
            Ok(None) => {
                let name = project::folder_name(&path);
                let initials = project::derive_icon_initials(&name);
                (name, initials, None, Vec::new(), AgentsConfig::default())
            }
            Err(e) => {
                let name = project::folder_name(&path);
                let initials = project::derive_icon_initials(&name);
                (
                    name,
                    initials,
                    Some(e.to_string()),
                    Vec::new(),
                    AgentsConfig::default(),
                )
            }
        };
        let (added, auto_start) = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            if let Some(handle) = inner.projects.get_mut(&id) {
                handle.name = name;
                handle.icon_initials = icon_initials;
                handle.config_error = config_error;
                handle.agents = agents;
            }
            insert_configured(&mut inner, id, configured)
        };
        self.events
            .publish(PodiumEvent::ProjectOpened { project_id: id });
        for pid in added {
            self.events.publish(PodiumEvent::ProcessAdded {
                project_id: id,
                process_id: pid,
            });
        }
        for pid in auto_start {
            if let Err(e) = do_start(&self.inner, &self.events, self.supervisor, pid, false, None) {
                tracing::warn!(process_id = %pid, "auto-start failed: {e}");
            }
        }
        // Taken even with the toggle off, so old sessions never resurface
        // later — and before the restore, so a quit during it cannot lose
        // what that quit captures.
        let remembered = self.session_memory.take(&path).unwrap_or_else(|e| {
            tracing::warn!("taking remembered agent sessions failed: {e}");
            Vec::new()
        });
        if self.agent_settings.get().remember_sessions {
            self.restore_remembered_sessions(id, &path, remembered)
                .await;
        }
        Ok(id)
    }

    /// Respawn every agent remembered for `root` on the session it was on,
    /// through the same resume path as a manual restart ([`restart_launch`]),
    /// on the args it was started with and owning its to-dos/scratchpads
    /// again. Best-effort per entry.
    async fn restore_remembered_sessions(
        &self,
        project_id: ProjectId,
        root: &Path,
        remembered: Vec<RememberedSession>,
    ) {
        let wsl = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            project_backend(&inner, project_id)
        };
        let now = Utc::now();
        let mut remembered = remembered.into_iter();
        while let Some(entry) = remembered.next() {
            if entry.is_expired(now) {
                continue;
            }
            // The file is user-writable and the id lands unquoted on a shell
            // line: never hand `--resume` anything but a UUID.
            if !session_memory::is_valid_session_id(&entry.session_id) {
                tracing::warn!("skipping a remembered agent session with an invalid id");
                continue;
            }
            // No transcript (pruned by Claude Code's retention, or an adapter
            // that cannot resume): a fresh agent without its context and
            // without a prompt would be of no use, so it is not respawned.
            if !self
                .session_stored(&entry.adapter_id, &entry.session_id, wsl)
                .await
            {
                continue;
            }
            // The app is quitting: spawn nothing more, and keep this entry
            // and the rest for the next run, next to what the quit captured.
            // ponytail: checked before each spawn, not under the spawn's own
            // lock, so a spawn already past this point when the quit begins
            // can still start after the stop sweep; closing that needs the
            // flag checked in `do_start`.
            if self.shutting_down.load(Ordering::SeqCst) {
                let rest = std::iter::once(entry)
                    .chain(remembered)
                    .map(|e| (root.to_path_buf(), e))
                    .collect();
                if let Err(e) = self.session_memory.remember_all(rest) {
                    tracing::warn!("keeping remembered agent sessions failed: {e}");
                }
                return;
            }
            // The to-dos/scratchpads are re-assigned, not handed over: a
            // prompt composed from them would land in the resumed
            // conversation as a fresh user message.
            let spawned = self
                .spawn_agent_inner(AgentSpawn {
                    project_id,
                    adapter_id: Some(entry.adapter_id),
                    name: Some(entry.name),
                    args_override: entry.args,
                    resume_session_id: Some(entry.session_id),
                    prompt: None,
                    todo_ids: Vec::new(),
                    scratchpad_ids: Vec::new(),
                    worktree: entry.worktree,
                    worktree_name: entry.worktree_name,
                })
                .await;
            match spawned {
                Ok(process_id) => self.reassign_restored(
                    project_id,
                    root,
                    process_id,
                    &entry.todo_ids,
                    &entry.scratchpad_ids,
                ),
                Err(e) => tracing::warn!("restoring a remembered agent session failed: {e}"),
            }
        }
    }

    /// Whether adapter `adapter_id`'s CLI has a stored conversation for
    /// `session_id` on backend `wsl` ([`AgentAdapter::session_exists`]). The
    /// check reads the disk — or, on WSL, runs a login shell in the distro —
    /// so it runs off the async runtime. Callers must not hold the lock.
    async fn session_stored(
        &self,
        adapter_id: &str,
        session_id: &str,
        wsl: Option<&'static WslBackend>,
    ) -> bool {
        let Some(adapter) = self.adapters.by_id(adapter_id) else {
            return false;
        };
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || adapter.session_exists(&session_id, wsl))
            .await
            .unwrap_or(false)
    }

    /// Give a restored agent back the to-dos and scratchpads its previous
    /// session owned — those that still exist, are not archived (or done),
    /// and no live agent has claimed in the meantime.
    fn reassign_restored(
        &self,
        project_id: ProjectId,
        root: &Path,
        process_id: ProcessId,
        todo_ids: &[TodoId],
        scratchpad_ids: &[ScratchpadId],
    ) {
        let todos: Vec<TodoId> = todo_ids
            .iter()
            .copied()
            .filter(|id| {
                self.todos
                    .get(project_id, root, *id)
                    .is_some_and(|t| !t.done && !t.archived)
            })
            .collect();
        let scratchpads: Vec<ScratchpadId> = scratchpad_ids
            .iter()
            .copied()
            .filter(|id| {
                self.scratchpads
                    .get(project_id, root, *id)
                    .is_some_and(|s| !s.archived)
            })
            .collect();
        let (todos, scratchpads) = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            (
                todos
                    .into_iter()
                    .filter(|id| !assigned_agent_of(&inner, *id).is_some_and(|a| a.active))
                    .collect::<Vec<_>>(),
                scratchpads
                    .into_iter()
                    .filter(|id| {
                        !scratchpad_assigned_agent_of(&inner, *id).is_some_and(|a| a.active)
                    })
                    .collect::<Vec<_>>(),
            )
        };
        self.assign_todos(project_id, process_id, &todos);
        self.assign_scratchpads(project_id, process_id, &scratchpads);
    }

    /// Close a project, stopping and removing all of its processes.
    pub async fn close_project(&self, id: ProjectId) -> CoreResult<()> {
        let removed: Vec<ProcessId> = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            if inner.projects.remove(&id).is_none() {
                return Err(CoreError::ProjectNotFound);
            }
            inner.order.retain(|pid| *pid != id);
            let ids: Vec<ProcessId> = inner
                .processes
                .iter()
                .filter(|(_, p)| p.project_id == id)
                .map(|(pid, _)| *pid)
                .collect();
            for pid in &ids {
                if let Some(mut p) = inner.processes.remove(pid) {
                    p.restart.cancel();
                    if let Some(pty) = &p.pty {
                        pty.stop();
                    }
                }
            }
            // The project (and its to-dos/scratchpads) is gone; drop any
            // dangling assignments so the runtime maps can't leak stale
            // process ids.
            inner.todo_assignments.retain(|_, pid| !ids.contains(pid));
            inner
                .scratchpad_assignments
                .retain(|_, pid| !ids.contains(pid));
            ids
        };
        for pid in removed {
            self.events.publish(PodiumEvent::ProcessRemoved {
                project_id: id,
                process_id: pid,
            });
        }
        // A proposal against a project that is gone could only ever fail on
        // confirm, so drop it now and say why.
        self.assistant.invalidate_proposals_for_project(id).await;
        self.events
            .publish(PodiumEvent::ProjectClosed { project_id: id });
        Ok(())
    }

    pub fn list_projects(&self) -> Vec<ProjectInfo> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .order
            .iter()
            .filter_map(|id| inner.projects.get(id).map(|p| p.info(*id)))
            .collect()
    }

    /// Set or clear a project's display-name override. A `None` or blank name
    /// clears it, reverting to the `podium.yml`/folder name. The override
    /// persists across config reloads. Returns the updated snapshot.
    pub fn rename_project(&self, id: ProjectId, name: Option<String>) -> CoreResult<ProjectInfo> {
        let info = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let handle = inner
                .projects
                .get_mut(&id)
                .ok_or(CoreError::ProjectNotFound)?;
            handle.name_override = normalize_project_name(name);
            handle.info(id)
        };
        self.events
            .publish(PodiumEvent::ProjectUpdated { project_id: id });
        Ok(info)
    }

    /// Set or clear a project's badge colour. `None` or blank clears it; anything
    /// but a `#rrggbb` hex is refused, since the value lands in a CSS property.
    pub fn set_project_color(
        &self,
        id: ProjectId,
        color: Option<String>,
    ) -> CoreResult<ProjectInfo> {
        let color = normalize_project_color(color)?;
        let info = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let handle = inner
                .projects
                .get_mut(&id)
                .ok_or(CoreError::ProjectNotFound)?;
            handle.color = color;
            handle.info(id)
        };
        self.events
            .publish(PodiumEvent::ProjectUpdated { project_id: id });
        Ok(info)
    }

    /// Re-apply a project's persisted overrides (display name, badge colour,
    /// section toggles) after `open_project`, publishing one `ProjectUpdated`
    /// for all rather than one per override. `None` leaves that override
    /// unset (sections: the default). A stored
    /// colour that is not `#rrggbb` (a hand-edited `workspace.json`) is
    /// dropped (and logged) so the name still applies.
    pub fn restore_project_overrides(
        &self,
        id: ProjectId,
        name: Option<String>,
        color: Option<String>,
        sections: Option<ProjectSections>,
    ) -> CoreResult<ProjectInfo> {
        let color = color.and_then(|c| {
            normalize_project_color(Some(c)).unwrap_or_else(|e| {
                tracing::warn!(project_id = %id, "ignoring stored project colour: {e}");
                None
            })
        });
        let info = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let handle = inner
                .projects
                .get_mut(&id)
                .ok_or(CoreError::ProjectNotFound)?;
            // Only what was stored: a project with just stored sections must
            // not lose its name or colour.
            if name.is_some() {
                handle.name_override = normalize_project_name(name);
            }
            if color.is_some() {
                handle.color = color;
            }
            if let Some(sections) = sections {
                handle.sections = sections;
            }
            handle.info(id)
        };
        self.events
            .publish(PodiumEvent::ProjectUpdated { project_id: id });
        Ok(info)
    }

    /// Set which optional sidebar sections a project shows. Presentation only:
    /// nothing is stopped or deleted. Returns the updated snapshot.
    pub fn set_project_sections(
        &self,
        id: ProjectId,
        sections: ProjectSections,
    ) -> CoreResult<ProjectInfo> {
        let info = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let handle = inner
                .projects
                .get_mut(&id)
                .ok_or(CoreError::ProjectNotFound)?;
            handle.sections = sections;
            handle.info(id)
        };
        self.events
            .publish(PodiumEvent::ProjectUpdated { project_id: id });
        Ok(info)
    }

    /// Reorder the sidebar project list. `ordered` lists project ids in the
    /// desired order; unknown/duplicate ids are ignored and any open project
    /// missing from the list keeps its relative position at the end. Returns
    /// the projects in the new order.
    pub fn reorder_projects(&self, ordered: Vec<ProjectId>) -> Vec<ProjectInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let mut new_order: Vec<ProjectId> = Vec::with_capacity(inner.order.len());
        for id in ordered {
            if inner.projects.contains_key(&id) && !new_order.contains(&id) {
                new_order.push(id);
            }
        }
        // Append any open projects the caller left out, keeping their order.
        for id in &inner.order {
            if inner.projects.contains_key(id) && !new_order.contains(id) {
                new_order.push(*id);
            }
        }
        inner.order = new_order;
        inner
            .order
            .iter()
            .filter_map(|id| inner.projects.get(id).map(|p| p.info(*id)))
            .collect()
    }

    /// Root path of an open project plus the backend it runs on.
    fn project_root_and_backend(
        &self,
        id: ProjectId,
    ) -> CoreResult<(PathBuf, Option<&'static WslBackend>)> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        let handle = inner.projects.get(&id).ok_or(CoreError::ProjectNotFound)?;
        Ok((handle.root.clone(), handle.wsl))
    }

    /// Root path of an open project (to-dos are keyed by it).
    fn project_root(&self, id: ProjectId) -> CoreResult<PathBuf> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        Ok(inner
            .projects
            .get(&id)
            .ok_or(CoreError::ProjectNotFound)?
            .root
            .clone())
    }

    /// List a project's active (non-archived) to-dos in creation order, each
    /// enriched with the agent (if any) currently assigned to it. Listing also
    /// auto-archives done to-dos left over from an earlier day.
    pub fn list_todos(&self, project_id: ProjectId) -> CoreResult<Vec<TodoInfo>> {
        let root = self.project_root(project_id)?;
        let mut todos = self.todos.list(project_id, &root);
        let inner = self.inner.lock().expect(LOCK_POISONED);
        for todo in &mut todos {
            todo.assigned_agent = assigned_agent_of(&inner, todo.id);
        }
        Ok(todos)
    }

    /// List a project's archived to-dos, most recently archived first.
    pub fn list_archived_todos(&self, project_id: ProjectId) -> CoreResult<Vec<TodoInfo>> {
        let root = self.project_root(project_id)?;
        let mut todos = self.todos.list_archived(project_id, &root);
        let inner = self.inner.lock().expect(LOCK_POISONED);
        for todo in &mut todos {
            todo.assigned_agent = assigned_agent_of(&inner, todo.id);
        }
        Ok(todos)
    }

    /// Archive or unarchive a to-do (regardless of its done state).
    pub fn set_todo_archived(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        archived: bool,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .set_archived(project_id, &root, todo_id, archived)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Assign one or more to-dos to an agent process, replacing any prior
    /// assignment. Silently ignores unknown to-do ids (the caller has already
    /// validated them at spawn time). Emits `TodosChanged` for the project.
    fn assign_todos(&self, project_id: ProjectId, process_id: ProcessId, todo_ids: &[TodoId]) {
        if todo_ids.is_empty() {
            return;
        }
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            for id in todo_ids {
                inner.todo_assignments.insert(*id, process_id);
            }
        }
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
    }

    /// Self-assign a single to-do to a running agent (used by the MCP
    /// `assign_todo` tool). The to-do must exist and the process must be an
    /// active agent in the same project. Returns the enriched to-do.
    pub fn assign_todo(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        process_id: ProcessId,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self
            .todos
            .get(project_id, &root, todo_id)
            .ok_or(CoreError::TodoNotFound)?;
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner
                .processes
                .get(&process_id)
                .ok_or(CoreError::ProcessNotFound)?;
            if proc.project_id != project_id || !matches!(proc.spec.kind, ProcessKind::Agent { .. })
            {
                return Err(CoreError::InvalidInput(
                    "process is not an agent in this project".to_string(),
                ));
            }
            // Block claiming a to-do already owned by a *different* live agent
            // (one that exited or is gone can be reclaimed; re-claiming your
            // own is idempotent).
            if assigned_agent_of(&inner, todo_id)
                .is_some_and(|a| a.active && a.process_id != process_id)
            {
                return Err(CoreError::TodoAlreadyAssigned);
            }
            inner.todo_assignments.insert(todo_id, process_id);
            info.assigned_agent = assigned_agent_of(&inner, todo_id);
        }
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Clear a to-do's agent assignment (the (x) action in the UI). A best-
    /// effort cancel/rollback request is left to the command layer, which can
    /// still reach the (soon-to-be-unassigned) agent's stdin. No-op — but not
    /// an error — when the to-do had no assignment. Returns the enriched to-do.
    pub fn unassign_todo(&self, project_id: ProjectId, todo_id: TodoId) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self
            .todos
            .get(project_id, &root, todo_id)
            .ok_or(CoreError::TodoNotFound)?;
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            inner.todo_assignments.remove(&todo_id);
            info.assigned_agent = assigned_agent_of(&inner, todo_id);
        }
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// The agent process currently assigned to a to-do, if any (so the
    /// command layer can reach its stdin for a best-effort cancel request).
    pub fn agent_for_todo(&self, todo_id: TodoId) -> Option<ProcessId> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .todo_assignments
            .get(&todo_id)
            .copied()
    }

    /// The agent process currently assigned to a scratchpad, if any (so the
    /// command layer can reach its stdin for a best-effort cancel request).
    pub fn agent_for_scratchpad(&self, scratchpad_id: ScratchpadId) -> Option<ProcessId> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .scratchpad_assignments
            .get(&scratchpad_id)
            .copied()
    }

    /// Add a to-do to a project. Blank text is rejected; a blank or missing
    /// `description` leaves it unset.
    pub fn add_todo(
        &self,
        project_id: ProjectId,
        text: &str,
        description: Option<&str>,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self.todos.add(project_id, &root, text, description)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Mark a to-do as done / not done.
    pub fn set_todo_done(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        done: bool,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self.todos.set_done(project_id, &root, todo_id, done)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Remove a to-do from a project.
    pub fn remove_todo(&self, project_id: ProjectId, todo_id: TodoId) -> CoreResult<()> {
        let root = self.project_root(project_id)?;
        self.todos.remove(&root, todo_id)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(())
    }

    /// Revise a to-do's text and/or description (agents keep it current as
    /// scope evolves). At least one of `text`/`description` must be set.
    pub fn update_todo(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        text: Option<&str>,
        description: Option<&str>,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .update(project_id, &root, todo_id, text, description)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Append a progress note to a to-do (agents post these as they work).
    pub fn comment_todo(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        author: &str,
        text: &str,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .add_comment(project_id, &root, todo_id, author, text)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Revise an existing comment's text.
    pub fn edit_todo_comment(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        comment_id: CommentId,
        text: &str,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .edit_comment(project_id, &root, todo_id, comment_id, text)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Remove a comment from a to-do.
    pub fn remove_todo_comment(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        comment_id: CommentId,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .remove_comment(project_id, &root, todo_id, comment_id)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Pin an issue/PR link to the top of a to-do (agents call this when they
    /// open a GitLab issue or MR/PR while working). The url must be `http(s)`.
    pub fn add_todo_link(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        label: &str,
        url: &str,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .add_link(project_id, &root, todo_id, label, url)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// Remove a pinned link from a to-do.
    pub fn remove_todo_link(
        &self,
        project_id: ProjectId,
        todo_id: TodoId,
        link_id: LinkId,
    ) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self
            .todos
            .remove_link(project_id, &root, todo_id, link_id)?;
        self.events
            .publish(PodiumEvent::TodosChanged { project_id });
        Ok(info)
    }

    /// List a project's active (non-archived) scratchpads, each enriched with
    /// the agent (if any) currently assigned to it.
    pub fn list_scratchpads(&self, project_id: ProjectId) -> CoreResult<Vec<ScratchpadInfo>> {
        let root = self.project_root(project_id)?;
        let mut scratchpads = self.scratchpads.list(project_id, &root);
        let inner = self.inner.lock().expect(LOCK_POISONED);
        for scratchpad in &mut scratchpads {
            scratchpad.assigned_agent = scratchpad_assigned_agent_of(&inner, scratchpad.id);
        }
        Ok(scratchpads)
    }

    /// List a project's archived scratchpads, most recently archived first.
    pub fn list_archived_scratchpads(
        &self,
        project_id: ProjectId,
    ) -> CoreResult<Vec<ScratchpadInfo>> {
        let root = self.project_root(project_id)?;
        let mut scratchpads = self.scratchpads.list_archived(project_id, &root);
        let inner = self.inner.lock().expect(LOCK_POISONED);
        for scratchpad in &mut scratchpads {
            scratchpad.assigned_agent = scratchpad_assigned_agent_of(&inner, scratchpad.id);
        }
        Ok(scratchpads)
    }

    /// Create a new scratchpad in a project (empty content, named after its
    /// content once there is any). Freshly created, so it can never already
    /// be assigned.
    pub fn add_scratchpad(
        &self,
        project_id: ProjectId,
        updated_by: &str,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let info = self.scratchpads.add(project_id, &root, updated_by)?;
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// One scratchpad by id, if it exists, enriched with its assignment.
    pub fn get_scratchpad(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
    ) -> CoreResult<Option<ScratchpadInfo>> {
        let root = self.project_root(project_id)?;
        let mut info = self.scratchpads.get(project_id, &root, id);
        if let Some(info) = &mut info {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        Ok(info)
    }

    /// Replace a scratchpad's content (bumps its version, and renames an
    /// auto-titled scratchpad after the new content). `expected_updated_at`
    /// must match the scratchpad's current `updated_at` or the call fails
    /// with [`CoreError::ScratchpadConflict`] (a concurrent edit landed
    /// first).
    pub fn update_scratchpad_content(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
        content: &str,
        expected_updated_at: DateTime<Utc>,
        updated_by: &str,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self.scratchpads.update_content(
            project_id,
            &root,
            id,
            content,
            expected_updated_at,
            updated_by,
        )?;
        {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Revise a scratchpad's title, pinning it (blank hands the name back to
    /// the content).
    /// `expected_updated_at` is checked the same way as in
    /// [`Self::update_scratchpad_content`].
    pub fn update_scratchpad_title(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
        title: &str,
        expected_updated_at: DateTime<Utc>,
        updated_by: &str,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self.scratchpads.update_title(
            project_id,
            &root,
            id,
            title,
            expected_updated_at,
            updated_by,
        )?;
        {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Add a free-text tag to a scratchpad (blank rejected, dedup by value).
    pub fn add_scratchpad_tag(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
        tag: &str,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self.scratchpads.add_tag(project_id, &root, id, tag)?;
        {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Remove a tag from a scratchpad by exact value (idempotent).
    pub fn remove_scratchpad_tag(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
        tag: &str,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self.scratchpads.remove_tag(project_id, &root, id, tag)?;
        {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Assign one or more scratchpads to an agent process, replacing any
    /// prior assignment. Silently ignores unknown scratchpad ids (the caller
    /// has already validated them at spawn time). Emits `ScratchpadsChanged`
    /// for the project.
    fn assign_scratchpads(
        &self,
        project_id: ProjectId,
        process_id: ProcessId,
        scratchpad_ids: &[ScratchpadId],
    ) {
        if scratchpad_ids.is_empty() {
            return;
        }
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            for id in scratchpad_ids {
                inner.scratchpad_assignments.insert(*id, process_id);
            }
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
    }

    /// Clear a scratchpad's agent assignment (the (x) action in the UI). A
    /// best-effort cancel/rollback request is left to the command layer,
    /// which can still reach the (soon-to-be-unassigned) agent's stdin. No-op
    /// — but not an error — when the scratchpad had no assignment. Returns
    /// the enriched scratchpad. There is no MCP-facing self-assign
    /// counterpart to this (unlike `assign_todo`) — a scratchpad's assignment
    /// is only ever set at spawn time.
    pub fn unassign_scratchpad(
        &self,
        project_id: ProjectId,
        scratchpad_id: ScratchpadId,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self
            .scratchpads
            .get(project_id, &root, scratchpad_id)
            .ok_or(CoreError::ScratchpadNotFound)?;
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            inner.scratchpad_assignments.remove(&scratchpad_id);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, scratchpad_id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Archive or unarchive a scratchpad.
    pub fn set_scratchpad_archived(
        &self,
        project_id: ProjectId,
        id: ScratchpadId,
        archived: bool,
    ) -> CoreResult<ScratchpadInfo> {
        let root = self.project_root(project_id)?;
        let mut info = self
            .scratchpads
            .set_archived(project_id, &root, id, archived)?;
        {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            info.assigned_agent = scratchpad_assigned_agent_of(&inner, id);
        }
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(info)
    }

    /// Permanently remove a scratchpad (deletion lives behind the Archive
    /// view, mirroring to-dos). Emits `ScratchpadsChanged`.
    pub fn remove_scratchpad(&self, project_id: ProjectId, id: ScratchpadId) -> CoreResult<()> {
        let root = self.project_root(project_id)?;
        self.scratchpads.remove(&root, id)?;
        self.events
            .publish(PodiumEvent::ScratchpadsChanged { project_id });
        Ok(())
    }

    /// Re-read `podium.yml`: update name/initials, replace config-defined
    /// processes (stopping any running ones), keep manually added processes.
    /// A parse/validation failure keeps current processes and only updates
    /// [`ProjectInfo::config_error`].
    pub async fn reload_project_config(&self, id: ProjectId) -> CoreResult<()> {
        let root = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            inner
                .projects
                .get(&id)
                .ok_or(CoreError::ProjectNotFound)?
                .root
                .clone()
        };
        let loaded = {
            let root = root.clone();
            tokio::task::spawn_blocking(move || project::load_project_config(&root))
                .await
                .map_err(|e| CoreError::Config(format!("config load task failed: {e}")))?
        };
        let (name, icon_initials, configured, agents) = match loaded {
            Ok(Some(cfg)) => (cfg.name, cfg.icon_initials, cfg.processes, cfg.agents),
            Ok(None) => {
                let name = project::folder_name(&root);
                let initials = project::derive_icon_initials(&name);
                (name, initials, Vec::new(), AgentsConfig::default())
            }
            Err(e) => {
                {
                    let mut inner = self.inner.lock().expect(LOCK_POISONED);
                    let handle = inner
                        .projects
                        .get_mut(&id)
                        .ok_or(CoreError::ProjectNotFound)?;
                    handle.config_error = Some(e.to_string());
                }
                self.events
                    .publish(PodiumEvent::ProjectUpdated { project_id: id });
                return Ok(());
            }
        };
        let (removed, added, auto_start) = {
            let mut guard = self.inner.lock().expect(LOCK_POISONED);
            let inner = &mut *guard;
            let handle = inner
                .projects
                .get_mut(&id)
                .ok_or(CoreError::ProjectNotFound)?;
            handle.name = name;
            handle.icon_initials = icon_initials;
            handle.config_error = None;
            handle.agents = agents;
            let old: Vec<ProcessId> = inner
                .processes
                .iter()
                .filter(|(_, p)| p.project_id == id && p.from_config)
                .map(|(pid, _)| *pid)
                .collect();
            for pid in &old {
                if let Some(mut p) = inner.processes.remove(pid) {
                    p.restart.cancel();
                    if let Some(pty) = &p.pty {
                        pty.stop();
                    }
                }
            }
            let (added, auto_start) = insert_configured(inner, id, configured);
            (old, added, auto_start)
        };
        self.events
            .publish(PodiumEvent::ProjectUpdated { project_id: id });
        for pid in removed {
            self.events.publish(PodiumEvent::ProcessRemoved {
                project_id: id,
                process_id: pid,
            });
        }
        for pid in added {
            self.events.publish(PodiumEvent::ProcessAdded {
                project_id: id,
                process_id: pid,
            });
        }
        for pid in auto_start {
            if let Err(e) = do_start(&self.inner, &self.events, self.supervisor, pid, false, None) {
                tracing::warn!(process_id = %pid, "auto-start failed: {e}");
            }
        }
        Ok(())
    }

    pub async fn add_process(
        &self,
        project_id: ProjectId,
        spec: ProcessSpec,
    ) -> CoreResult<ProcessId> {
        let id = ProcessId::new();
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            if !inner.projects.contains_key(&project_id) {
                return Err(CoreError::ProjectNotFound);
            }
            insert_process(&mut inner, id, project_id, spec, false);
        }
        self.events.publish(PodiumEvent::ProcessAdded {
            project_id,
            process_id: id,
        });
        Ok(id)
    }

    /// List the agent adapters this orchestrator can spawn. Probes each
    /// adapter's binary on the login-shell `PATH` of the backend the agent
    /// would run on — `project_id`'s, or the configured one when `None`
    /// (Settings) — which shells out: call from a blocking-friendly context.
    pub fn list_adapters(&self, project_id: Option<ProjectId>) -> CoreResult<Vec<AdapterInfo>> {
        let wsl = match project_id {
            Some(id) => self.project_root_and_backend(id)?.1,
            None => crate::platform::wsl(),
        };
        Ok(self.adapters.infos(wsl))
    }

    /// Add and immediately start an agent process in `project_id`.
    ///
    /// `adapter_id` defaults to the project's `agents.default_adapter`, then
    /// the global Settings → Agents default, then the built-in fallback;
    /// `name` defaults to the first free `<binary>`, `<binary>-2`, … in the
    /// project. When `todo_ids` is non-empty, the agent's prompt is seeded
    /// with those to-dos' text/description plus standing instructions to keep
    /// them current over MCP (comment progress, update on scope change,
    /// complete when done); multiple to-dos are handed over as one combined
    /// task. `scratchpad_ids` works the same way for scratchpads, but only
    /// when `todo_ids` is empty — to-dos win if both are somehow populated.
    /// When `worktree` is true the agent is *told* to move its work into a
    /// Podium-managed git worktree it creates itself (`create_worktree` over
    /// MCP), optionally named `worktree_name`; Podium does not create one, so
    /// the agent still starts in the project root. `args_override`, when
    /// `Some`, replaces the global Settings → Agents default args for this
    /// spawn only (still merged with the project's `agents.extra_args` per the
    /// merge mode). On a start
    /// failure the process stays listed as `NotStarted` so the user can retry
    /// from the UI.
    #[allow(clippy::too_many_arguments)] // deliberate: one flat spawn API
    pub async fn spawn_agent(
        &self,
        project_id: ProjectId,
        adapter_id: Option<String>,
        name: Option<String>,
        prompt: Option<String>,
        todo_ids: Vec<TodoId>,
        scratchpad_ids: Vec<ScratchpadId>,
        worktree: bool,
        worktree_name: Option<String>,
        args_override: Option<Vec<String>>,
    ) -> CoreResult<ProcessId> {
        self.spawn_agent_inner(AgentSpawn {
            project_id,
            adapter_id,
            name,
            prompt,
            todo_ids,
            scratchpad_ids,
            worktree,
            worktree_name,
            args_override,
            resume_session_id: None,
        })
        .await
    }

    async fn spawn_agent_inner(&self, req: AgentSpawn) -> CoreResult<ProcessId> {
        let AgentSpawn {
            project_id,
            adapter_id,
            name,
            prompt,
            todo_ids,
            scratchpad_ids,
            worktree,
            worktree_name,
            args_override,
            resume_session_id,
        } = req;
        let (root, wsl, agents, existing_names, session_color) = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let handle = inner
                .projects
                .get(&project_id)
                .ok_or(CoreError::ProjectNotFound)?;
            if active_agent_count(&inner, project_id) >= MAX_AGENTS_PER_PROJECT {
                return Err(CoreError::AgentLimitReached);
            }
            let names: HashSet<String> = inner
                .processes
                .values()
                .filter(|p| p.project_id == project_id)
                .map(|p| p.spec.name.clone())
                .collect();
            let color = pick_session_color(&inner, project_id, None);
            (
                handle.root.clone(),
                handle.wsl,
                handle.agents.clone(),
                names,
                color,
            )
        };
        // Adapter precedence: an explicit choice, then the project's pinned
        // `agents.default_adapter`, then the global Settings → Agents default,
        // then the built-in fallback.
        let global = self.agent_settings.get();
        let adapter_id = adapter_id
            .or_else(|| agents.default_adapter.clone())
            .or_else(|| global.default_adapter.clone())
            .unwrap_or_else(|| DEFAULT_ADAPTER_ID.to_string());
        let adapter = self.adapters.by_id(&adapter_id).ok_or_else(|| {
            CoreError::InvalidInput(format!("unknown agent adapter: {adapter_id}"))
        })?;
        // Fetch the to-dos (if any): they seed both the window name and the
        // prompt. All must exist before we spawn anything.
        let todos: Vec<TodoInfo> = todo_ids
            .iter()
            .map(|id| {
                self.todos
                    .get(project_id, &root, *id)
                    .ok_or(CoreError::TodoNotFound)
            })
            .collect::<CoreResult<Vec<_>>>()?;
        // Same for scratchpads — but only when no to-dos were given: to-dos
        // win, and scratchpad_ids is documented as ignored in that case, so
        // an invalid/deleted scratchpad id must not fail (or a valid one
        // spuriously get assigned) a to-do spawn. All must exist before we
        // spawn anything.
        let scratchpads: Vec<ScratchpadInfo> = if todos.is_empty() {
            scratchpad_ids
                .iter()
                .map(|id| {
                    self.scratchpads
                        .get(project_id, &root, *id)
                        .ok_or(CoreError::ScratchpadNotFound)
                })
                .collect::<CoreResult<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let base_prompt = prompt.as_deref().map(str::trim).filter(|p| !p.is_empty());
        // Window name, in precedence: an explicit name, the first to-do's
        // text (an agent spawned on to-dos), the first scratchpad's title (an
        // agent spawned on scratchpads), a short label derived from the
        // prompt, else the adapter binary. A generically named agent (the
        // binary fallback) is told via its launch plan to rename itself over
        // MCP after the first user message.
        let explicit = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        // A single to-do/scratchpad names the session after it; with several
        // grouped, picking one is arbitrary ("a random to-do"), so leave the
        // name underived — `named` stays false and the agent renames itself
        // over MCP once it has read all of them.
        let derived = todos
            .first()
            .filter(|_| todos.len() == 1)
            .map(|t| t.text.trim())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| {
                scratchpads
                    .first()
                    .filter(|_| scratchpads.len() == 1)
                    .map(|s| s.title.trim())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| base_prompt.and_then(name_from_prompt));
        let named = explicit.is_some() || derived.is_some();
        let name = match explicit {
            Some(n) => n.to_string(),
            None => next_free_name(
                &derived.unwrap_or_else(|| adapter.binary().to_string()),
                &existing_names,
            ),
        };
        let mcp = self.mcp.lock().expect(LOCK_POISONED).clone();
        let process_id = ProcessId::new();
        // To-dos seed a context-rich prompt; failing that, scratchpads;
        // otherwise use the raw one.
        let final_prompt: Option<String> = if !todos.is_empty() {
            Some(compose_todos_prompt(&todos, base_prompt))
        } else if !scratchpads.is_empty() {
            Some(compose_scratchpads_prompt(&scratchpads, base_prompt))
        } else {
            base_prompt.map(str::to_string)
        };
        // A requested worktree is the agent's job, not Podium's: it starts in
        // the project root (where the user's tooling and caches live) and its
        // identity prompt tells it to create the worktree over MCP before the
        // first code change. An empty `worktree_name` means "you pick".
        let wt_name = worktree_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        // Combine the global default args (Settings → Agents) with the
        // project's `agents.extra_args` per the user's merge mode, and apply
        // any global command override for this adapter. A per-session
        // `args_override` (from the New agent dialog) replaces the global
        // default args just for this spawn.
        let ov = global.override_for(&adapter_id);
        let global_args: &[String] = args_override
            .as_deref()
            .or_else(|| ov.map(|o| o.default_args.as_slice()))
            .unwrap_or(&[]);
        let merged_args = settings::merge_args(global.merge_mode, global_args, &agents.extra_args);
        let command_override = ov.and_then(|o| o.command.as_deref());
        let plan = adapter.build_launch(&AgentLaunchCtx {
            project_id,
            process_id,
            project_root: &root,
            wsl,
            prompt: final_prompt.as_deref(),
            extra_args: &merged_args,
            command_override,
            mcp: mcp.as_ref(),
            named,
            worktree,
            worktree_name: wt_name,
            suggest_worktree: global.suggest_worktree && !worktree,
        })?;
        let mut session = plan.restart.map(|commands| AgentSession {
            commands,
            id: process_id.to_string(),
            resumed: false,
            resume_failed: false,
        });
        // A restore starts on the remembered session, spelled exactly as a
        // restart that found it stored. Without session flags (the user's own
        // args pick the session now) there is nothing to resume it with.
        let relaunch = match (resume_session_id, session.as_mut()) {
            (Some(id), Some(session)) => {
                session.id = id;
                Some(restart_launch(session, true))
            }
            (Some(_), None) => {
                return Err(CoreError::InvalidInput(
                    "this agent's launch cannot resume a session".to_string(),
                ))
            }
            (None, _) => None,
        };
        let spec = ProcessSpec {
            name,
            command: plan.command,
            cwd: root.clone(),
            env: plan.env,
            kind: ProcessKind::Agent {
                adapter: adapter.id().to_string(),
            },
            restart_policy: RestartPolicy::Never,
            color: Some(session_color),
        };
        {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            // Re-check under the same lock as the insert: a concurrent spawn
            // may have raced past the early check above.
            let guard = if !inner.projects.contains_key(&project_id) {
                Some(CoreError::ProjectNotFound)
            } else if active_agent_count(&inner, project_id) >= MAX_AGENTS_PER_PROJECT {
                Some(CoreError::AgentLimitReached)
            } else {
                None
            };
            if let Some(err) = guard {
                return Err(err);
            }
            insert_process(&mut inner, process_id, project_id, spec, false);
            if let Some(p) = inner.processes.get_mut(&process_id) {
                p.session = session;
                p.spawn_args = args_override;
                p.spawn_worktree = worktree;
                p.spawn_worktree_name = worktree_name.clone();
            }
        }
        self.events.publish(PodiumEvent::ProcessAdded {
            project_id,
            process_id,
        });
        do_start(
            &self.inner,
            &self.events,
            self.supervisor,
            process_id,
            true,
            relaunch,
        )?;
        // Link the agent to the to-dos/scratchpads it was spawned for so the
        // UI can show who is working on what (also emits TodosChanged /
        // ScratchpadsChanged for the project).
        let assign_todo_ids: Vec<TodoId> = todos.iter().map(|t| t.id).collect();
        self.assign_todos(project_id, process_id, &assign_todo_ids);
        let assign_scratchpad_ids: Vec<ScratchpadId> = scratchpads.iter().map(|s| s.id).collect();
        self.assign_scratchpads(project_id, process_id, &assign_scratchpad_ids);
        Ok(process_id)
    }

    /// Remove a process, stopping it first if it is still running. Also
    /// cancels any pending supervised restart.
    pub async fn remove_process(&self, id: ProcessId) -> CoreResult<()> {
        let (project_id, cleared) = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            // Drop the process's to-do/scratchpad assignments while it is
            // still in the map (clearing needs its project id), then remove
            // the process.
            let cleared = clear_agent_assignments(&mut inner, id);
            let mut p = inner
                .processes
                .remove(&id)
                .ok_or(CoreError::ProcessNotFound)?;
            p.restart.cancel();
            if let Some(pty) = &p.pty {
                pty.stop();
            }
            (p.project_id, cleared)
        };
        self.events.publish(PodiumEvent::ProcessRemoved {
            project_id,
            process_id: id,
        });
        for pid in cleared.todos {
            self.events
                .publish(PodiumEvent::TodosChanged { project_id: pid });
        }
        for pid in cleared.scratchpads {
            self.events
                .publish(PodiumEvent::ScratchpadsChanged { project_id: pid });
        }
        Ok(())
    }

    pub fn list_processes(&self, project_id: Option<ProjectId>) -> Vec<ProcessInfo> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .processes
            .iter()
            .filter(|(_, p)| project_id.is_none_or(|pid| p.project_id == pid))
            .map(|(id, p)| p.info(*id))
            .collect()
    }

    /// The Git board (issues + PRs/MRs, all-time PR/MR counts) of the repo
    /// behind the project's `origin` remote, via the project's chosen setup
    /// (or the one matching the remote's host); `mine` narrows it to the
    /// user's own. Returns it with the time it was fetched. Shells out — call
    /// from a blocking-friendly context.
    pub fn issue_board(&self, project_id: ProjectId, mine: bool) -> CoreResult<CachedBoard> {
        let root = self.project_root(project_id)?;
        let issues = self.agent_settings().issues;
        let selected = issues
            .projects
            .get(&root)
            .and_then(|id| issues.setups.iter().find(|s| &s.id == id));
        // Taken before the fetch: a move landing meanwhile wins over its result.
        let edits = self.boards.edits(&root);
        // Either scope's last board tells GitLab where the columns were.
        let cached = self
            .boards
            .get(&root, mine)
            .or_else(|| self.boards.get(&root, !mine));
        let board = crate::issues::board(
            &root,
            &issues.setups,
            selected,
            mine,
            cached.as_ref().map(|c| &c.board),
        )?;
        if let Some(stored) = self.boards.put(&root, mine, board.clone(), edits) {
            return Ok(stored);
        }
        // A move/unassign of this project landed mid-fetch, so this board
        // predates it: serve the cache (which has the edit), with its own
        // fetch time; the next refresh catches up.
        Ok(self.boards.get(&root, mine).unwrap_or_else(|| CachedBoard {
            board,
            fetched_at: Utc::now(),
        }))
    }

    /// The last successfully fetched board of this scope, if any. No network.
    pub fn cached_issue_board(
        &self,
        project_id: ProjectId,
        mine: bool,
    ) -> CoreResult<Option<CachedBoard>> {
        Ok(self.boards.get(&self.project_root(project_id)?, mine))
    }

    /// Move a card to another column of the project's Git board, on the
    /// platform itself (GitLab labels/state, GitHub close/reopen/Project
    /// Status). Validated against the cached board, so it needs one: of the
    /// scopes holding the card, the most recently fetched — the best word on
    /// whether it is open or closed, which decides close/reopen. A card
    /// already in `column` there is a no-op success (the other scope was
    /// stale), not a refusal. On success both cached scopes are updated and
    /// `BoardChanged` published. Shells out / blocks — call from a
    /// blocking-friendly context.
    pub fn move_board_item(
        &self,
        project_id: ProjectId,
        kind: crate::ItemKind,
        number: u64,
        column: &str,
    ) -> CoreResult<()> {
        let root = self.project_root(project_id)?;
        let cached = self
            .newest_board_with(&root, kind, number)
            .or_else(|| self.boards.get(&root, true))
            .or_else(|| self.boards.get(&root, false))
            .ok_or_else(|| CoreError::IssuesMove("Refresh the board first.".to_string()))?;
        let board = &cached.board;
        let item = crate::issues::validate_move(board, kind, number, column)?;
        if item.column == column {
            self.boards.apply_move(&root, kind, number, column, None);
            self.events
                .publish(PodiumEvent::BoardChanged { project_id });
            return Ok(());
        }
        let issues = self.agent_settings().issues;
        let selected = issues
            .projects
            .get(&root)
            .and_then(|id| issues.setups.iter().find(|s| &s.id == id));
        let labels =
            crate::issues::move_item(&root, &issues.setups, selected, board, item, column)?;
        self.boards
            .apply_move(&root, kind, number, column, labels.as_deref());
        self.events
            .publish(PodiumEvent::BoardChanged { project_id });
        Ok(())
    }

    /// Of the cached scopes holding the card, the most recently fetched: the
    /// best word on its state when the two disagree.
    fn newest_board_with(
        &self,
        root: &Path,
        kind: crate::ItemKind,
        number: u64,
    ) -> Option<CachedBoard> {
        [true, false]
            .into_iter()
            .filter_map(|mine| self.boards.get(root, mine))
            .filter(|c| {
                c.board
                    .items
                    .iter()
                    .any(|i| i.kind == kind && i.number == number)
            })
            .max_by_key(|c| c.fetched_at)
    }

    /// Unassign the signed-in user from a card of the project's Git board, on
    /// the platform itself. Validated against the cached board (which knows
    /// who you are), so it needs one. On success both cached scopes are
    /// updated and `BoardChanged` published. Blocks — call from a
    /// blocking-friendly context.
    pub fn unassign_board_item(
        &self,
        project_id: ProjectId,
        kind: crate::ItemKind,
        number: u64,
    ) -> CoreResult<()> {
        let root = self.project_root(project_id)?;
        let cached = self.newest_board_with(&root, kind, number).ok_or_else(|| {
            CoreError::IssuesMove(
                "That card is not on the board. Refresh the board first.".to_string(),
            )
        })?;
        let (item, me) = crate::issues::validate_unassign(&cached.board, kind, number)?;
        let issues = self.agent_settings().issues;
        let selected = issues
            .projects
            .get(&root)
            .and_then(|id| issues.setups.iter().find(|s| &s.id == id));
        crate::issues::unassign_item(&root, &issues.setups, selected, item, me)?;
        self.boards.apply_unassign(&root, kind, number, me);
        self.events
            .publish(PodiumEvent::BoardChanged { project_id });
        Ok(())
    }

    /// Settings → Issues: every setup (with whether a GitLab token is stored —
    /// never the token) plus each project's pick. Touches the keychain.
    pub fn issues_overview(&self) -> crate::issues::IssuesOverview {
        let issues = self.agent_settings().issues;
        crate::issues::IssuesOverview {
            setups: issues
                .setups
                .into_iter()
                .map(|setup| crate::issues::IssueSetupInfo {
                    token_set: setup.provider == crate::issues::IssueProvider::Gitlab
                        && crate::issues::gitlab_token_present(&setup.id),
                    setup,
                })
                .collect(),
            projects: issues.projects,
        }
    }

    /// Add a setup (`id: None`) or replace the one with that id. The URL is
    /// normalized (scheme kept, `https` by default); a blank name falls back to
    /// the host. `token` (GitLab only): `None` keeps the stored one, blank
    /// deletes it, anything else replaces it.
    pub fn save_issue_setup(
        &self,
        id: Option<String>,
        name: String,
        provider: crate::issues::IssueProvider,
        url: String,
        account: Option<String>,
        token: Option<String>,
    ) -> CoreResult<crate::issues::IssuesOverview> {
        let url = crate::issues::normalize_url(&url).ok_or_else(|| {
            CoreError::InvalidInput(
                "enter a valid http(s) URL, without a user:token@ part".to_string(),
            )
        })?;
        let name = match name.trim() {
            "" => crate::issues::url_host(&url).to_string(),
            n => n.to_string(),
        };
        let github = provider == crate::issues::IssueProvider::Github;
        let setup = crate::issues::IssueSetup {
            id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            provider,
            url,
            account: account
                .map(|a| a.trim().to_string())
                .filter(|a| github && !a.is_empty()),
        };
        if let Some(token) = token.filter(|_| !github) {
            crate::issues::gitlab_token_set(&setup.id, &token)?;
        }
        let mut issues = self.agent_settings().issues;
        // Turned from GitLab into GitHub: its token has no use any more.
        let was_gitlab = issues
            .setups
            .iter()
            .any(|s| s.id == setup.id && s.provider == crate::IssueProvider::Gitlab);
        if github && was_gitlab {
            if let Err(e) = crate::issues::gitlab_token_set(&setup.id, "") {
                tracing::warn!("could not delete a GitLab token no longer in use: {e}");
            }
        }
        match issues.setups.iter_mut().find(|s| s.id == setup.id) {
            Some(existing) => *existing = setup,
            None => issues.setups.push(setup),
        }
        self.agent_settings.set_issues(issues)?;
        Ok(self.issues_overview())
    }

    /// Remove a setup, its keychain token, and any project's pick of it.
    pub fn remove_issue_setup(&self, id: &str) -> CoreResult<crate::issues::IssuesOverview> {
        let mut issues = self.agent_settings().issues;
        // Only a GitLab setup has a token. A keychain that will not delete it
        // must not keep the setup alive: log it and remove the setup anyway.
        let gitlab = issues
            .setups
            .iter()
            .any(|s| s.id == id && s.provider == crate::IssueProvider::Gitlab);
        if gitlab {
            if let Err(e) = crate::issues::gitlab_token_set(id, "") {
                tracing::warn!("could not delete a removed setup's GitLab token: {e}");
            }
        }
        issues.setups.retain(|s| s.id != id);
        issues.projects.retain(|_, pick| pick != id);
        self.agent_settings.set_issues(issues)?;
        Ok(self.issues_overview())
    }

    /// Pin a project to one setup; `None` goes back to matching by host.
    pub fn set_project_issue_setup(
        &self,
        project_id: ProjectId,
        setup_id: Option<String>,
    ) -> CoreResult<crate::issues::IssuesOverview> {
        let root = self.project_root(project_id)?;
        let mut issues = self.agent_settings().issues;
        match setup_id {
            Some(id) => {
                if !issues.setups.iter().any(|s| s.id == id) {
                    return Err(CoreError::InvalidInput("unknown issue setup".to_string()));
                }
                issues.projects.insert(root, id);
            }
            None => {
                issues.projects.remove(&root);
            }
        }
        self.agent_settings.set_issues(issues)?;
        Ok(self.issues_overview())
    }

    /// The project's Podium-managed git worktrees, with `in_use` marking the
    /// ones an active process runs in. Shells out to git — call from a
    /// blocking-friendly context.
    pub fn list_worktrees(&self, project_id: ProjectId) -> CoreResult<Vec<WorktreeInfo>> {
        let (root, wsl) = self.project_root_and_backend(project_id)?;
        let mut infos = crate::worktree::list(wsl, &root)?;
        let inner = self.inner.lock().expect(LOCK_POISONED);
        for info in &mut infos {
            info.in_use = inner.processes.values().any(|p| {
                p.project_id == project_id && p.is_active() && p.spec.cwd.starts_with(&info.path)
            });
        }
        Ok(infos)
    }

    /// Public recipe names from the project root's justfile (empty when
    /// there is none). Read fresh on every call — cheap, and always current
    /// after the user edits the file.
    pub fn project_just_recipes(&self, project_id: ProjectId) -> CoreResult<Vec<String>> {
        let root = self.project_root(project_id)?;
        Ok(crate::justfile::recipes(&root))
    }

    /// Open the project root in a new Warp tab via Warp's URI scheme.
    /// Launches an OS process — call from a blocking-friendly context.
    pub fn open_project_in_warp(&self, project_id: ProjectId) -> CoreResult<()> {
        let root = self.project_root(project_id)?;
        crate::platform::open_in_warp(&root)
    }

    /// The git branch currently checked out in a process's working
    /// directory, or `None` when its cwd is not a git repo / detached. Feeds
    /// the focused-process header. Shells out to git — call from a
    /// blocking-friendly context.
    pub fn process_git_branch(&self, id: ProcessId) -> CoreResult<Option<String>> {
        let (cwd, wsl) = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
            (
                proc.spec.cwd.clone(),
                project_backend(&inner, proc.project_id),
            )
        };
        Ok(crate::worktree::current_branch(wsl, &cwd))
    }

    /// Create a Podium-managed git worktree in the project (name slugified
    /// and de-duplicated). Shells out to git — call from a blocking-friendly
    /// context. Emits `WorktreesChanged` for the project.
    pub fn create_worktree(&self, project_id: ProjectId, name: &str) -> CoreResult<WorktreeInfo> {
        let (root, wsl) = self.project_root_and_backend(project_id)?;
        let info = crate::worktree::create(wsl, &root, name)?;
        self.events
            .publish(PodiumEvent::WorktreesChanged { project_id });
        Ok(info)
    }

    /// Remove a Podium-managed git worktree. Refused while an active process
    /// runs in it, or while it has uncommitted changes unless `force`; its
    /// `podium/<name>` branch is kept. Shells out to git — call from a
    /// blocking-friendly context. Emits `WorktreesChanged` for the project.
    pub fn remove_worktree(
        &self,
        project_id: ProjectId,
        name: &str,
        force: bool,
    ) -> CoreResult<()> {
        let (root, wsl) = self.project_root_and_backend(project_id)?;
        let in_use = self
            .list_worktrees(project_id)?
            .iter()
            .any(|w| w.name == name && w.in_use);
        if in_use {
            return Err(CoreError::WorktreeInUse);
        }
        crate::worktree::remove(wsl, &root, name, force)?;
        self.events
            .publish(PodiumEvent::WorktreesChanged { project_id });
        Ok(())
    }

    /// Open a worktree's checkout in a new Warp tab via Warp's URI scheme.
    /// Launches an OS process — call from a blocking-friendly context.
    pub fn open_worktree_in_warp(&self, project_id: ProjectId, name: &str) -> CoreResult<()> {
        let entry = self
            .list_worktrees(project_id)?
            .into_iter()
            .find(|w| w.name == name)
            .ok_or(CoreError::WorktreeNotFound)?;
        crate::platform::open_in_warp(&entry.path)
    }

    /// Whether any agent or terminal process is currently running (or
    /// stopping). The shell uses this to warn before the app closes, since
    /// exiting SIGTERMs/SIGKILLs every managed process group — losing an
    /// agent's in-flight work or a live terminal session. Services are
    /// intentionally excluded: they come from `podium.yml` and are meant to
    /// come and go with the app.
    pub fn has_active_agents_or_terminals(&self) -> bool {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .processes
            .values()
            .any(|p| {
                p.is_active()
                    && matches!(
                        p.spec.kind,
                        ProcessKind::Agent { .. } | ProcessKind::Terminal
                    )
            })
    }

    /// Rename a process's display name. Blank names are rejected. The rename
    /// only affects the sidebar/window label — it does not restart the
    /// process or touch its command. Returns the updated snapshot.
    pub fn rename_process(&self, id: ProcessId, name: &str) -> CoreResult<ProcessInfo> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidInput(
                "process name must not be empty".to_string(),
            ));
        }
        let (project_id, info) = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner
                .processes
                .get_mut(&id)
                .ok_or(CoreError::ProcessNotFound)?;
            proc.spec.name = name.to_string();
            (proc.project_id, proc.info(id))
        };
        self.events.publish(PodiumEvent::ProcessUpdated {
            project_id,
            process_id: id,
        });
        Ok(info)
    }

    /// Manually start a process. Cancels any pending supervised restart and
    /// resets the backoff/breaker state: an explicit user start is a fresh
    /// beginning.
    ///
    /// An agent that ran before picks its CLI session back up when the CLI
    /// has it stored, and otherwise starts a fresh session (see
    /// [`restart_launch`]). Its first start always runs `spec.command`.
    pub async fn start_process(&self, id: ProcessId) -> CoreResult<()> {
        let restart = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
            match (&proc.spec.kind, &proc.session) {
                (ProcessKind::Agent { adapter }, Some(session))
                    if matches!(proc.status, ProcessStatus::Exited { .. }) =>
                {
                    Some((
                        adapter.clone(),
                        session.clone(),
                        project_backend(&inner, proc.project_id),
                    ))
                }
                _ => None,
            }
        };
        let relaunch = match restart {
            Some((adapter, session, wsl)) => {
                let stored = self.session_stored(&adapter, &session.id, wsl).await;
                Some(restart_launch(&session, stored))
            }
            None => None,
        };
        do_start(
            &self.inner,
            &self.events,
            self.supervisor,
            id,
            true,
            relaunch,
        )
    }

    /// Stop a stuck agent and put a fresh one on the same work, with the
    /// answer it was waiting for already in its prompt.
    ///
    /// Nothing can type into a running agent's terminal, so an agent blocked
    /// on a question it will never get an answer to is dead weight — the only
    /// way through is a new session that never has to ask. Doing it here
    /// rather than leaving the caller to stop and spawn keeps the one thing
    /// it cannot reconstruct: the to-dos the old session owned, which is what
    /// carries the task over. `task` supplies that context for an agent
    /// spawned without a to-do; with neither, the replacement would start
    /// knowing only an answer to a question nobody asked it, so that is
    /// refused rather than spawned.
    ///
    /// The replacement is deliberately not given the old name: both sessions
    /// stay in the list, and a distinct name is what tells them apart.
    pub async fn respawn_agent(
        &self,
        process_id: ProcessId,
        answer: &str,
        task: Option<&str>,
    ) -> CoreResult<ProcessId> {
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(CoreError::InvalidInput(
                "an answer for the fresh agent is required".to_string(),
            ));
        }
        let (project_id, adapter_id) = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner
                .processes
                .get(&process_id)
                .ok_or(CoreError::ProcessNotFound)?;
            match &proc.spec.kind {
                ProcessKind::Agent { adapter } => (proc.project_id, adapter.clone()),
                _ => {
                    return Err(CoreError::InvalidInput(
                        "only an agent can be respawned".to_string(),
                    ))
                }
            }
        };
        let todo_ids: Vec<TodoId> = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            inner
                .todo_assignments
                .iter()
                .filter(|(_, pid)| **pid == process_id)
                .map(|(id, _)| *id)
                .collect()
        };
        let task = task.map(str::trim).filter(|t| !t.is_empty());
        if todo_ids.is_empty() && task.is_none() {
            return Err(CoreError::InvalidInput(
                "the agent owns no to-do, so the fresh one needs a task as well as an answer"
                    .to_string(),
            ));
        }
        // Stop first so the old session cannot keep working (or answer its
        // own question) while its replacement starts. A stop that fails
        // because it already exited is fine — the point is that it is not
        // running when the new one does.
        // ponytail: the stop is a SIGTERM, and a process still `Stopping`
        // counts towards the 8-agents cap, so a respawn in a project already
        // at the limit fails; waiting for the exit would need a watch on the
        // event bus, which is not worth it for that corner.
        match self.stop_process(process_id).await {
            Ok(()) | Err(CoreError::ProcessNotRunning) => {}
            Err(e) => return Err(e),
        }
        let mut prompt = String::new();
        if let Some(task) = task {
            prompt.push_str(task);
            prompt.push_str("\n\n");
        }
        prompt.push_str(
            "The previous session on this work was stopped while it sat waiting on a \
             question. Here is the answer, so you can carry straight on without asking \
             it again:\n\n",
        );
        prompt.push_str(answer);
        self.spawn_agent(
            project_id,
            Some(adapter_id),
            None,
            Some(prompt),
            todo_ids,
            Vec::new(),
            false,
            None,
            None,
        )
        .await
    }

    /// Request a graceful stop (SIGTERM, then SIGKILL after a grace period).
    /// Also cancels a pending supervised restart; if only a restart was
    /// pending (the process already exited), that cancellation is success.
    pub async fn stop_process(&self, id: ProcessId) -> CoreResult<()> {
        let (project_id, pty) = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner
                .processes
                .get_mut(&id)
                .ok_or(CoreError::ProcessNotFound)?;
            let cancelled = proc.restart.cancel();
            let running = matches!(proc.status, ProcessStatus::Running { .. });
            match proc.pty.clone() {
                Some(pty) if running => {
                    proc.user_stopped = true;
                    proc.status = ProcessStatus::Stopping;
                    (proc.project_id, pty)
                }
                _ if cancelled => return Ok(()),
                _ => return Err(CoreError::ProcessNotRunning),
            }
        };
        self.events.publish(PodiumEvent::ProcessStatusChanged {
            project_id,
            process_id: id,
            status: ProcessStatus::Stopping,
        });
        pty.stop();
        Ok(())
    }

    /// Stop the process if running, wait for it to exit, then start it again.
    pub async fn restart_process(&self, id: ProcessId) -> CoreResult<()> {
        match self.stop_process(id).await {
            Ok(()) | Err(CoreError::ProcessNotRunning) => {}
            Err(e) => return Err(e),
        }
        self.wait_until_stopped(id).await?;
        self.start_process(id).await
    }

    async fn wait_until_stopped(&self, id: ProcessId) -> CoreResult<()> {
        let deadline = tokio::time::Instant::now() + RESTART_STOP_TIMEOUT;
        loop {
            {
                let inner = self.inner.lock().expect(LOCK_POISONED);
                let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
                if !proc.is_active() {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(CoreError::Pty(
                    "timed out waiting for process to stop".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub async fn write_stdin(&self, id: ProcessId, bytes: &[u8]) -> CoreResult<()> {
        self.running_pty(id)?.write(bytes)
    }

    pub async fn resize(&self, id: ProcessId, cols: u16, rows: u16) -> CoreResult<()> {
        self.running_pty(id)?.resize(cols, rows)
    }

    fn running_pty(&self, id: ProcessId) -> CoreResult<Arc<PtyProcess>> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
        proc.pty.clone().ok_or(CoreError::ProcessNotRunning)
    }

    /// Atomically snapshot the scrollback and subscribe to subsequent
    /// chunks: the returned `next_seq` is exactly the seq of the first chunk
    /// the receiver can observe, with no gap and no duplication.
    pub async fn attach(
        &self,
        id: ProcessId,
    ) -> CoreResult<(Vec<u8>, u64, broadcast::Receiver<TermChunk>)> {
        let (scrollback, chunk_tx) = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
            (Arc::clone(&proc.scrollback), proc.chunk_tx.clone())
        };
        // Hold the scrollback lock across snapshot + subscribe: the reader
        // thread appends and broadcasts under this same lock, so nothing can
        // slip in between.
        let sb = scrollback.lock().expect(LOCK_POISONED);
        let (snapshot, next_seq) = sb.snapshot();
        let rx = chunk_tx.subscribe();
        drop(sb);
        Ok((snapshot, next_seq, rx))
    }

    /// Lossy-UTF-8 text of the last `max_bytes` of a process's output.
    pub async fn tail_text(&self, id: ProcessId, max_bytes: usize) -> CoreResult<String> {
        let scrollback = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            let proc = inner.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
            Arc::clone(&proc.scrollback)
        };
        let bytes = scrollback
            .lock()
            .expect(LOCK_POISONED)
            .tail_bytes(max_bytes);
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<PodiumEvent> {
        self.events.subscribe()
    }

    /// Stop every running process (SIGTERM its group now, SIGKILL after the
    /// grace period) and cancel all pending supervised restarts. With
    /// `remember_sessions` on, running agents' session ids are captured first
    /// (see [`Self::capture_agent_sessions`]) so the next open can resume them.
    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.assistant.shutdown().await;
        if self.agent_settings.get().remember_sessions {
            self.capture_agent_sessions();
        }
        let stopped: Vec<(ProjectId, ProcessId)> = {
            let mut inner = self.inner.lock().expect(LOCK_POISONED);
            let mut stopped = Vec::new();
            for (id, proc) in inner.processes.iter_mut() {
                proc.restart.cancel();
                if let Some(pty) = &proc.pty {
                    proc.user_stopped = true;
                    proc.status = ProcessStatus::Stopping;
                    pty.stop();
                    stopped.push((proc.project_id, *id));
                }
            }
            stopped
        };
        for (project_id, process_id) in stopped {
            self.events.publish(PodiumEvent::ProcessStatusChanged {
                project_id,
                process_id,
                status: ProcessStatus::Stopping,
            });
        }
    }

    /// Remember each running agent's current CLI session ([`AgentSession`]:
    /// the id it was launched or last restarted on) for the next open of its
    /// project, with the to-dos/scratchpads it owns and the args it was
    /// started with. An agent without a Podium-assigned session — an adapter
    /// that cannot resume (Auggie), or one whose own args pick the session
    /// (`--resume`, `--continue`, …) — is not remembered.
    ///
    /// Runs on the app's exit path, so it reads nothing from disk and starts
    /// no process; every entry goes out in one write.
    fn capture_agent_sessions(&self) {
        let mut entries: Vec<(Option<Instant>, PathBuf, RememberedSession)> = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            inner
                .processes
                .iter()
                .filter_map(|(id, p)| {
                    let ProcessKind::Agent { adapter } = &p.spec.kind else {
                        return None;
                    };
                    if !matches!(p.status, ProcessStatus::Running { .. }) {
                        return None;
                    }
                    let session = p.session.as_ref()?;
                    Some((
                        p.started_at,
                        inner.projects.get(&p.project_id)?.root.clone(),
                        RememberedSession {
                            adapter_id: adapter.clone(),
                            name: p.spec.name.clone(),
                            session_id: session.id.clone(),
                            updated_at: Utc::now(),
                            todo_ids: owned_by(&inner.todo_assignments, *id),
                            scratchpad_ids: owned_by(&inner.scratchpad_assignments, *id),
                            args: p.spawn_args.clone(),
                            worktree: p.spawn_worktree,
                            worktree_name: p.spawn_worktree_name.clone(),
                        },
                    ))
                })
                .collect()
        };
        // Oldest first, so the restore respawns them — and the sidebar lists
        // them — in the order they were started.
        entries.sort_by_key(|(started, ..)| *started);
        let entries = entries
            .into_iter()
            .map(|(_, root, entry)| (root, entry))
            .collect();
        if let Err(e) = self.session_memory.remember_all(entries) {
            tracing::warn!("remembering agent sessions failed: {e}");
        }
    }
}

/// The keys of `assignments` owned by `process_id`.
fn owned_by<K: Copy>(assignments: &HashMap<K, ProcessId>, process_id: ProcessId) -> Vec<K> {
    assignments
        .iter()
        .filter(|(_, pid)| **pid == process_id)
        .map(|(k, _)| *k)
        .collect()
}

/// Everything one agent spawn takes. [`Orchestrator::spawn_agent`] keeps its
/// flat public signature; internally a restore also needs the session to
/// resume, and a struct keeps that from growing yet another positional
/// argument.
struct AgentSpawn {
    project_id: ProjectId,
    adapter_id: Option<String>,
    name: Option<String>,
    prompt: Option<String>,
    todo_ids: Vec<TodoId>,
    scratchpad_ids: Vec<ScratchpadId>,
    worktree: bool,
    worktree_name: Option<String>,
    args_override: Option<Vec<String>>,
    resume_session_id: Option<String>,
}

/// Trim a project display name; blank means "none".
pub fn normalize_project_name(name: Option<String>) -> Option<String> {
    name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

/// Trim and lowercase a project badge colour; blank means "none". Anything
/// but `#rrggbb` is refused, since the value lands in a CSS property.
fn normalize_project_color(color: Option<String>) -> CoreResult<Option<String>> {
    let color = color
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty());
    if let Some(c) = &color {
        let hex = c.strip_prefix('#').unwrap_or("");
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidInput(format!(
                "project colour must be #rrggbb, got {c:?}"
            )));
        }
    }
    Ok(color)
}

/// Insert config-defined processes; returns (added, auto-start subset).
fn insert_configured(
    inner: &mut Inner,
    project_id: ProjectId,
    configured: Vec<ConfiguredProcess>,
) -> (Vec<ProcessId>, Vec<ProcessId>) {
    let mut added = Vec::with_capacity(configured.len());
    let mut auto_start = Vec::new();
    for cp in configured {
        let pid = ProcessId::new();
        insert_process(inner, pid, project_id, cp.spec, true);
        added.push(pid);
        if cp.auto_start {
            auto_start.push(pid);
        }
    }
    (added, auto_start)
}

/// Subtle session colours (Radix-ish hues) assigned round-robin to agent
/// sessions so each session — and every to-do it owns — can be tinted the
/// same. Solid base hues; the frontend applies the transparency.
const SESSION_COLORS: [&str; 8] = [
    "#3e63dd", // blue
    "#30a46c", // green
    "#e5484d", // red
    "#8e4ec6", // purple
    "#f76b15", // orange
    "#12a594", // teal
    "#e93d82", // pink
    "#ffb224", // amber
];

/// Pick a session colour not currently used by another live agent in the
/// project — `preferred` when it is free; once the palette is exhausted,
/// cycle by the used count.
// ponytail: two concurrent spawns could momentarily pick the same colour —
// cosmetic only, and the max is 8 agents = palette size, so it's rare.
fn pick_session_color(inner: &Inner, project_id: ProjectId, preferred: Option<&str>) -> String {
    let used: HashSet<&str> = inner
        .processes
        .values()
        .filter(|p| {
            p.project_id == project_id
                && p.is_active()
                && matches!(p.spec.kind, ProcessKind::Agent { .. })
        })
        .filter_map(|p| p.spec.color.as_deref())
        .collect();
    preferred
        .into_iter()
        .chain(SESSION_COLORS.iter().copied())
        .find(|c| !used.contains(c))
        .unwrap_or(SESSION_COLORS[used.len() % SESSION_COLORS.len()])
        .to_string()
}

/// Resolve a to-do's assignment to an [`AssignedAgent`] snapshot, dropping a
/// stale entry whose process has since vanished from the map.
fn assigned_agent_of(inner: &Inner, todo_id: TodoId) -> Option<AssignedAgent> {
    let process_id = *inner.todo_assignments.get(&todo_id)?;
    inner.processes.get(&process_id).map(|p| AssignedAgent {
        process_id,
        name: p.spec.name.clone(),
        color: p.spec.color.clone(),
        active: p.is_active(),
    })
}

/// Resolve a scratchpad's assignment to an [`AssignedAgent`] snapshot,
/// dropping a stale entry whose process has since vanished from the map.
fn scratchpad_assigned_agent_of(
    inner: &Inner,
    scratchpad_id: ScratchpadId,
) -> Option<AssignedAgent> {
    let process_id = *inner.scratchpad_assignments.get(&scratchpad_id)?;
    inner.processes.get(&process_id).map(|p| AssignedAgent {
        process_id,
        name: p.spec.name.clone(),
        color: p.spec.color.clone(),
        active: p.is_active(),
    })
}

/// The projects whose to-do and/or scratchpad assignments changed, returned
/// by [`clear_agent_assignments`] so the caller can publish exactly the
/// events that apply (never both, unless the removed process really did hold
/// both kinds of assignment).
#[derive(Default)]
struct ClearedAssignments {
    todos: HashSet<ProjectId>,
    scratchpads: HashSet<ProjectId>,
}

/// Drop every to-do/scratchpad assignment pointing at `process_id` (called
/// when an agent is removed — an exited agent keeps its assignments). Returns the
/// affected projects, split by kind, so the caller can emit `TodosChanged`
/// and/or `ScratchpadsChanged` once per project — without firing an event for
/// a kind that had nothing to clear.
fn clear_agent_assignments(inner: &mut Inner, process_id: ProcessId) -> ClearedAssignments {
    let removed_todos: Vec<TodoId> = inner
        .todo_assignments
        .iter()
        .filter(|(_, pid)| **pid == process_id)
        .map(|(tid, _)| *tid)
        .collect();
    let removed_scratchpads: Vec<ScratchpadId> = inner
        .scratchpad_assignments
        .iter()
        .filter(|(_, pid)| **pid == process_id)
        .map(|(sid, _)| *sid)
        .collect();
    let project = inner.processes.get(&process_id).map(|p| p.project_id);
    for tid in &removed_todos {
        inner.todo_assignments.remove(tid);
    }
    for sid in &removed_scratchpads {
        inner.scratchpad_assignments.remove(sid);
    }
    ClearedAssignments {
        todos: match project {
            Some(pid) if !removed_todos.is_empty() => HashSet::from([pid]),
            _ => HashSet::new(),
        },
        scratchpads: match project {
            Some(pid) if !removed_scratchpads.is_empty() => HashSet::from([pid]),
            _ => HashSet::new(),
        },
    }
}

/// Agents in `project_id` that are still active (running or stopping).
fn active_agent_count(inner: &Inner, project_id: ProjectId) -> usize {
    inner
        .processes
        .values()
        .filter(|p| {
            p.project_id == project_id
                && matches!(p.spec.kind, ProcessKind::Agent { .. })
                && p.is_active()
        })
        .count()
}

/// A short window name derived from an agent's launch prompt: the first
/// non-empty line, truncated to ~40 chars on a word boundary. Lets a
/// standalone agent be recognisable in the sidebar immediately, without
/// depending on the model to rename itself over MCP. `None` if the prompt has
/// no usable text.
fn name_from_prompt(prompt: &str) -> Option<String> {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty())?;
    const MAX: usize = 40;
    if line.chars().count() <= MAX {
        return Some(line.to_string());
    }
    // Truncate on a char boundary, then back off to the last word break so we
    // don't cut mid-word; append an ellipsis.
    let head: String = line.chars().take(MAX).collect();
    let trimmed = head.rsplit_once(' ').map(|(h, _)| h).unwrap_or(&head);
    Some(format!("{}…", trimmed.trim_end()))
}

/// Build an agent launch prompt from a to-do: its text/description, the
/// standing instructions to keep it current over MCP, then any user prompt.
/// Referencing the ids by name (not value) keeps this Podium-owned text.
fn compose_todo_prompt(todo: &TodoInfo, user_prompt: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("You are working on a Podium to-do.\n\n");
    out.push_str(&format!("To-do id: {}\n", todo.id));
    out.push_str(&format!("Task: {}\n", todo.text));
    if let Some(description) = &todo.description {
        out.push_str(&format!("Description:\n{description}\n"));
    }
    out.push_str(
        "\nKeep this to-do up to date as you work, using the Podium MCP tools \
         (pass the to-do id above):\n\
         - comment_todo: leave small comments — a short overview of what you \
         did, not a full write-up. No code, no diffs; a sentence or two per \
         meaningful step, and a brief summary when you finish.\n\
         - update_todo: revise the task text or description if the scope or \
         details change.\n\
         - complete_todo: mark the to-do done once the work is finished.\n",
    );
    if let Some(user_prompt) = user_prompt {
        out.push_str("\nAdditional instructions:\n");
        out.push_str(user_prompt);
        out.push('\n');
    }
    out
}

/// Build an agent launch prompt from one or more to-dos. A single to-do keeps
/// the original single-to-do phrasing; several to-dos are handed over as one
/// combined task, each listed with its id/text/description, followed by the
/// standing MCP instructions (once) and any user prompt.
fn compose_todos_prompt(todos: &[TodoInfo], user_prompt: Option<&str>) -> String {
    if let [only] = todos {
        return compose_todo_prompt(only, user_prompt);
    }
    let mut out = String::new();
    out.push_str(&format!(
        "You are working on {} Podium to-dos as a single task.\n\n",
        todos.len()
    ));
    for (i, todo) in todos.iter().enumerate() {
        out.push_str(&format!("To-do {} of {}\n", i + 1, todos.len()));
        out.push_str(&format!("To-do id: {}\n", todo.id));
        out.push_str(&format!("Task: {}\n", todo.text));
        if let Some(description) = &todo.description {
            out.push_str(&format!("Description:\n{description}\n"));
        }
        out.push('\n');
    }
    out.push_str(
        "Work through all of the to-dos above. Keep each one up to date as you \
         work, using the Podium MCP tools (pass the relevant to-do id):\n\
         - comment_todo: leave small comments — a short overview of what you \
         did, not a full write-up. No code, no diffs; a sentence or two per \
         meaningful step, and a brief summary when you finish.\n\
         - update_todo: revise a to-do's text or description if the scope or \
         details change.\n\
         - complete_todo: mark each to-do done once its part is finished.\n",
    );
    if let Some(user_prompt) = user_prompt {
        out.push_str("\nAdditional instructions:\n");
        out.push_str(user_prompt);
        out.push('\n');
    }
    out
}

/// Build an agent launch prompt from a scratchpad: its title/content, the
/// standing instructions to keep it current over MCP, then any user prompt.
fn compose_scratchpad_prompt(scratchpad: &ScratchpadInfo, user_prompt: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("You are working on a Podium scratchpad.\n\n");
    out.push_str(&format!("Scratchpad id: {}\n", scratchpad.id));
    out.push_str(&format!("Title: {}\n", scratchpad.title));
    out.push_str(
        "\nThis scratchpad is shared with the user and other agents. Fetch \
         its current content and use the Podium MCP tools to keep it up to \
         date as you work (pass the scratchpad id above):\n\
         - list_scratchpads: get the current content and updatedAt — do \
         this before your first update_scratchpad call, so it doesn't get \
         rejected as a conflict.\n\
         - update_scratchpad: revise the content — pass the updatedAt you \
         just fetched as expected_updated_at.\n\
         - add_scratchpad_tag / remove_scratchpad_tag: tag it for easy \
         discovery.\n",
    );
    if let Some(user_prompt) = user_prompt {
        out.push_str("\nAdditional instructions:\n");
        out.push_str(user_prompt);
        out.push('\n');
    }
    out
}

/// Build an agent launch prompt from one or more scratchpads. A single
/// scratchpad keeps the original single-scratchpad phrasing; several
/// scratchpads are handed over as one combined task, each listed with its
/// id/title, followed by the standing MCP instructions (once) and any user
/// prompt. Content isn't embedded here — the agent fetches it (and the
/// current `updatedAt`) via `list_scratchpads` itself.
fn compose_scratchpads_prompt(scratchpads: &[ScratchpadInfo], user_prompt: Option<&str>) -> String {
    if let [only] = scratchpads {
        return compose_scratchpad_prompt(only, user_prompt);
    }
    let mut out = String::new();
    out.push_str(&format!(
        "You are working on {} Podium scratchpads as a single task.\n\n",
        scratchpads.len()
    ));
    for (i, scratchpad) in scratchpads.iter().enumerate() {
        out.push_str(&format!("Scratchpad {} of {}\n", i + 1, scratchpads.len()));
        out.push_str(&format!("Scratchpad id: {}\n", scratchpad.id));
        out.push_str(&format!("Title: {}\n", scratchpad.title));
        out.push('\n');
    }
    out.push_str(
        "Work through all of the scratchpads above. Fetch each one's current \
         content and use the Podium MCP tools to keep it up to date as you \
         work (pass the relevant scratchpad id):\n\
         - list_scratchpads: get the current content and updatedAt — do \
         this before your first update_scratchpad call for each scratchpad, \
         so it doesn't get rejected as a conflict.\n\
         - update_scratchpad: revise the content — pass the updatedAt you \
         just fetched as expected_updated_at.\n\
         - add_scratchpad_tag / remove_scratchpad_tag: tag it for easy \
         discovery.\n",
    );
    if let Some(user_prompt) = user_prompt {
        out.push_str("\nAdditional instructions:\n");
        out.push_str(user_prompt);
        out.push('\n');
    }
    out
}

/// First of `base`, `base-2`, `base-3`, … not already used in the project.
fn next_free_name(base: &str, existing: &HashSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// The backend `project_id` runs on (see [`ProjectHandle::wsl`]); `None`
/// natively, or once the project is gone.
fn project_backend(inner: &Inner, project_id: ProjectId) -> Option<&'static WslBackend> {
    inner.projects.get(&project_id).and_then(|p| p.wsl)
}

fn insert_process(
    inner: &mut Inner,
    id: ProcessId,
    project_id: ProjectId,
    spec: ProcessSpec,
    from_config: bool,
) {
    let (chunk_tx, _) = broadcast::channel(CHUNK_CHANNEL_CAPACITY);
    inner.processes.insert(
        id,
        ManagedProcess {
            spec,
            project_id,
            scrollback: Arc::new(Mutex::new(ScrollbackBuffer::new())),
            pty: None,
            status: ProcessStatus::NotStarted,
            user_stopped: false,
            chunk_tx,
            from_config,
            restart: RestartState::default(),
            started_at: None,
            waiting_since: None,
            spawn_args: None,
            spawn_worktree: false,
            spawn_worktree_name: None,
            session: None,
            last_color: None,
        },
    );
}

/// Spawn a process's PTY. A free function (not `&self`) so supervised
/// restart tasks can call it too. `manual` marks an explicit user start: it
/// cancels any pending supervised restart and resets the backoff/breaker.
fn do_start(
    inner: &Arc<Mutex<Inner>>,
    events: &EventBus,
    supervisor: SupervisorConfig,
    id: ProcessId,
    manual: bool,
    relaunch: Option<Relaunch>,
) -> CoreResult<()> {
    // The project's backend, read under a short lock of its own: the gate
    // below depends on it, and must be taken before `inner`.
    let wsl = {
        let guard = inner.lock().expect(LOCK_POISONED);
        let proc = guard.processes.get(&id).ok_or(CoreError::ProcessNotFound)?;
        project_backend(&guard, proc.project_id)
    };
    // On the WSL backend, one `wsl.exe` start at a time — a native launch
    // (a Windows-drive project with `nativeDriveProjects`) skips the gate,
    // so it never waits behind a slow distro start. Taken *before* `inner`
    // and declared first so it drops last: waiting for the gate — and its
    // settle sleep on drop — must never happen under `inner`, which every
    // command (sync ones on the main thread included) needs.
    let launch_gate = crate::platform::wsl_launch_guard_for(wsl);
    let mut guard = inner.lock().expect(LOCK_POISONED);
    // An agent that lost its colour on exit takes it back if no live agent
    // holds it now, else gets a new one picked exactly as at spawn.
    let new_color = match guard.processes.get(&id) {
        Some(p)
            if p.spec.color.is_none()
                && !p.is_active()
                && matches!(p.spec.kind, ProcessKind::Agent { .. }) =>
        {
            Some(pick_session_color(
                &guard,
                p.project_id,
                p.last_color.as_deref(),
            ))
        }
        _ => None,
    };
    let proc = guard
        .processes
        .get_mut(&id)
        .ok_or(CoreError::ProcessNotFound)?;
    if proc.is_active() {
        return Err(CoreError::ProcessAlreadyRunning);
    }
    if manual {
        proc.restart.cancel();
        proc.restart.reset();
    }
    proc.user_stopped = false;
    let project_id = proc.project_id;
    // A restart's command runs this once; `spec.command` stays as spawned.
    let restart_spec = relaunch.as_ref().map(|r| ProcessSpec {
        command: r.command.clone(),
        ..proc.spec.clone()
    });
    let pty = PtyProcess::spawn(
        restart_spec.as_ref().unwrap_or(&proc.spec),
        wsl,
        launch_gate.as_ref(),
        None,
        Arc::clone(&proc.scrollback),
        proc.chunk_tx.clone(),
        make_exit_handler(
            Arc::clone(inner),
            events.clone(),
            supervisor,
            id,
            project_id,
        ),
    )?;
    let status = ProcessStatus::Running {
        pid: pty.pid(),
        since: Utc::now(),
    };
    proc.status = status.clone();
    proc.pty = Some(Arc::new(pty));
    proc.started_at = Some(Instant::now());
    // Only now that the launch is running does its session become current.
    if let (Some(r), Some(session)) = (relaunch, proc.session.as_mut()) {
        session.resume_failed &= r.session_id == session.id;
        session.id = r.session_id;
        session.resumed = r.resumed;
    }
    let recolored = new_color.is_some();
    if recolored {
        proc.spec.color = new_color;
    }
    // Publish while still holding the lock: the exit callback also takes
    // the lock, so an instantly-exiting child's Exited event can never be
    // observed before this Running event.
    events.publish(PodiumEvent::ProcessStatusChanged {
        project_id,
        process_id: id,
        status,
    });
    if recolored {
        publish_color_change(&guard, events, id, project_id);
    }
    Ok(())
}

/// The command a manual start of an exited agent runs, and the session id it
/// is on afterwards: the current session resumed when the CLI has its
/// transcript, else a fresh session under a **new** id. Never the current id
/// again: the CLI refuses an id it already used, so a transcript the check
/// failed to find must not make the agent unstartable — and the new id keeps
/// the agent resumable from then on.
/// A resume that already failed right away is not tried again
/// ([`AgentSession::resume_failed`]), so the agent never crash-loops on it.
fn restart_launch(session: &AgentSession, stored: bool) -> Relaunch {
    if stored && !session.resume_failed {
        Relaunch {
            command: session.commands.resume(&session.id),
            session_id: session.id.clone(),
            resumed: true,
        }
    } else {
        let session_id = uuid::Uuid::new_v4().to_string();
        Relaunch {
            command: session.commands.fresh(&session_id),
            session_id,
            resumed: false,
        }
    }
}

/// Whether an agent run that just exited was a resume that failed: it
/// crashed within [`RESUME_FAILURE_WINDOW`] of starting.
fn resume_failed(session: &AgentSession, crashed: bool, ran_for: Option<Duration>) -> bool {
    session.resumed && crashed && ran_for.is_some_and(|d| d < RESUME_FAILURE_WINDOW)
}

/// Tell the UI an agent's colour changed: its own row (`ProcessUpdated`
/// re-lists processes) and the to-dos/scratchpads it owns, which carry the
/// colour on their assignment.
fn publish_color_change(inner: &Inner, events: &EventBus, id: ProcessId, project_id: ProjectId) {
    events.publish(PodiumEvent::ProcessUpdated {
        project_id,
        process_id: id,
    });
    if inner.todo_assignments.values().any(|p| *p == id) {
        events.publish(PodiumEvent::TodosChanged { project_id });
    }
    if inner.scratchpad_assignments.values().any(|p| *p == id) {
        events.publish(PodiumEvent::ScratchpadsChanged { project_id });
    }
}

/// Handler run on the PTY wait-thread when the child exits. A non-zero exit
/// only counts as a crash when the user did not stop the process. Depending
/// on the restart policy, schedules a supervised restart with exponential
/// backoff — unless the circuit breaker trips.
fn make_exit_handler(
    inner: Arc<Mutex<Inner>>,
    events: EventBus,
    supervisor: SupervisorConfig,
    id: ProcessId,
    project_id: ProjectId,
) -> ExitCallback {
    // The wait-thread is not a tokio context; capture the runtime handle now
    // (all `do_start` callers run inside the runtime).
    let rt = tokio::runtime::Handle::current();
    Box::new(move |code| {
        let restart = {
            let mut guard = inner.lock().expect(LOCK_POISONED);
            let Some(p) = guard.processes.get_mut(&id) else {
                return;
            };
            let crashed = code != Some(0) && !p.user_stopped;
            let status = ProcessStatus::Exited {
                code,
                crashed,
                stopped: p.user_stopped,
                at: Utc::now(),
            };
            p.status = status.clone();
            p.pty = None;
            let ran_for = p.started_at.take().map(|s| s.elapsed());
            if let Some(session) = p.session.as_mut() {
                if resume_failed(session, crashed, ran_for) {
                    session.resume_failed = true;
                }
            }
            let ran_long = ran_for.is_some_and(|d| d >= supervisor.backoff_reset_after);
            if ran_long {
                p.restart.reset_backoff();
            }
            let wants_restart = !p.user_stopped
                && match p.spec.restart_policy {
                    RestartPolicy::Never => false,
                    RestartPolicy::OnCrash => crashed,
                    RestartPolicy::Always => true,
                };
            let restart = if wants_restart {
                let scheduled = p.restart.try_schedule(Instant::now(), &supervisor);
                if scheduled.is_none() {
                    tracing::warn!(
                        process = %p.spec.name,
                        "restart circuit breaker tripped; giving up"
                    );
                }
                scheduled
            } else {
                None
            };
            // An exited agent that is not being restarted keeps its
            // to-dos/scratchpads (a restart picks the work back up) but loses
            // its colour, so the UI shows it as no longer live. The next
            // start takes it back if it is still free.
            let decolored = restart.is_none()
                && matches!(p.spec.kind, ProcessKind::Agent { .. })
                && p.spec.color.is_some();
            if decolored {
                p.last_color = p.spec.color.take();
            }
            events.publish(PodiumEvent::ProcessStatusChanged {
                project_id,
                process_id: id,
                status,
            });
            if decolored {
                publish_color_change(&guard, &events, id, project_id);
            }
            restart
        };
        if let Some((delay, generation)) = restart {
            schedule_restart(&rt, inner, events, supervisor, id, delay, generation);
        }
    })
}

/// Spawn the delayed-restart task and record its abort handle. The captured
/// generation makes cancellation race-free: stop/remove/close/shutdown bump
/// it, and a stale task gives up instead of starting a ghost process.
fn schedule_restart(
    rt: &tokio::runtime::Handle,
    inner: Arc<Mutex<Inner>>,
    events: EventBus,
    supervisor: SupervisorConfig,
    id: ProcessId,
    delay: Duration,
    generation: u64,
) {
    let task_inner = Arc::clone(&inner);
    let handle = rt
        .spawn(async move {
            tokio::time::sleep(delay).await;
            {
                let mut guard = task_inner.lock().expect(LOCK_POISONED);
                let Some(p) = guard.processes.get_mut(&id) else {
                    return;
                };
                if p.restart.generation() != generation {
                    return;
                }
                p.restart.clear_pending();
            }
            if let Err(e) = do_start(&task_inner, &events, supervisor, id, false, None) {
                tracing::warn!(process_id = %id, "supervised restart failed: {e}");
            }
        })
        .abort_handle();
    let mut guard = inner.lock().expect(LOCK_POISONED);
    match guard.processes.get_mut(&id) {
        Some(p) if p.restart.generation() == generation => p.restart.set_pending(handle),
        // Cancelled/removed in the meantime: the task would see the stale
        // generation anyway; abort it eagerly.
        _ => handle.abort(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_resumes_a_stored_session_and_otherwise_starts_fresh() {
        let session = AgentSession {
            commands: RestartCommands {
                bin: "claude".to_string(),
                prompt: Some("'fix the login bug'".to_string()),
                tail: String::new(),
                new_flag: "--session-id",
                resume_flag: "--resume",
            },
            id: "0f0e0d0c".to_string(),
            resumed: false,
            resume_failed: false,
        };
        let resume = restart_launch(&session, true);
        assert_eq!(resume.command, "claude --resume 0f0e0d0c");
        assert_eq!(resume.session_id, "0f0e0d0c");
        assert!(resume.resumed);

        // Not found: a new id, never the current one the CLI may still know.
        let fresh = restart_launch(&session, false);
        assert_ne!(fresh.session_id, session.id);
        assert!(!fresh.resumed);
        assert_eq!(
            fresh.command,
            format!(
                "claude 'fix the login bug' --session-id {}",
                fresh.session_id
            )
        );

        // A resume that already failed is not tried again, transcript or not.
        let failed = AgentSession {
            resume_failed: true,
            ..session
        };
        let next = restart_launch(&failed, true);
        assert!(!next.resumed);
        assert_ne!(next.session_id, failed.id);
    }

    #[test]
    fn only_a_resume_that_crashes_right_away_counts_as_failed() {
        let session = |resumed| AgentSession {
            commands: RestartCommands {
                bin: "claude".to_string(),
                prompt: None,
                tail: String::new(),
                new_flag: "--session-id",
                resume_flag: "--resume",
            },
            id: "0f0e0d0c".to_string(),
            resumed,
            resume_failed: false,
        };
        let quick = Some(Duration::from_secs(1));
        let long = Some(RESUME_FAILURE_WINDOW + Duration::from_secs(1));
        assert!(resume_failed(&session(true), true, quick));
        assert!(
            !resume_failed(&session(true), false, quick),
            "clean exit or user stop"
        );
        assert!(
            !resume_failed(&session(true), true, long),
            "ran a while first"
        );
        assert!(
            !resume_failed(&session(false), true, quick),
            "a fresh start"
        );
    }

    fn todo(text: &str, description: Option<&str>) -> TodoInfo {
        TodoInfo {
            id: TodoId::new(),
            project_id: ProjectId::new(),
            text: text.to_string(),
            description: description.map(str::to_string),
            done: false,
            created_at: Utc::now(),
            done_at: None,
            archived: false,
            archived_at: None,
            links: Vec::new(),
            comments: Vec::new(),
            assigned_agent: None,
        }
    }

    #[test]
    fn compose_todo_prompt_includes_task_and_mcp_instructions() {
        let todo = todo("wire up auth", None);
        let prompt = compose_todo_prompt(&todo, None);

        assert!(prompt.contains(&format!("To-do id: {}", todo.id)));
        assert!(prompt.contains("Task: wire up auth"));
        // The standing instructions name each tool the agent should use.
        assert!(prompt.contains("comment_todo"));
        assert!(prompt.contains("update_todo"));
        assert!(prompt.contains("complete_todo"));
        // No description was set, so no Description block appears.
        assert!(!prompt.contains("Description:"));
        // No user prompt, so no additional-instructions block.
        assert!(!prompt.contains("Additional instructions:"));
    }

    #[test]
    fn compose_todo_prompt_appends_description_and_user_prompt() {
        let todo = todo("wire up auth", Some("use the shared provider"));
        let prompt = compose_todo_prompt(&todo, Some("focus on the happy path"));

        assert!(prompt.contains("Description:\nuse the shared provider"));
        assert!(prompt.contains("Additional instructions:\nfocus on the happy path"));
    }

    #[test]
    fn compose_todos_prompt_single_matches_single_helper() {
        // One to-do routes through the original single-to-do phrasing.
        let todo = todo("wire up auth", Some("use the shared provider"));
        let one = compose_todos_prompt(std::slice::from_ref(&todo), Some("go"));
        assert_eq!(one, compose_todo_prompt(&todo, Some("go")));
    }

    #[test]
    fn compose_todos_prompt_combines_multiple_todos() {
        let a = todo("wire up auth", Some("use the shared provider"));
        let b = todo("write tests", None);
        let prompt = compose_todos_prompt(&[a.clone(), b.clone()], Some("start with auth"));

        // Presented as one combined task listing every to-do id and text.
        assert!(prompt.contains("2 Podium to-dos as a single task"));
        assert!(prompt.contains("To-do 1 of 2"));
        assert!(prompt.contains("To-do 2 of 2"));
        assert!(prompt.contains(&format!("To-do id: {}", a.id)));
        assert!(prompt.contains(&format!("To-do id: {}", b.id)));
        assert!(prompt.contains("Task: wire up auth"));
        assert!(prompt.contains("Task: write tests"));
        assert!(prompt.contains("Description:\nuse the shared provider"));
        // The standing MCP instructions and user prompt appear once.
        assert!(prompt.contains("complete_todo"));
        assert!(prompt.contains("Additional instructions:\nstart with auth"));
    }

    fn scratchpad(title: &str, content: &str) -> ScratchpadInfo {
        ScratchpadInfo {
            id: ScratchpadId::new(),
            project_id: ProjectId::new(),
            title: title.to_string(),
            content: content.to_string(),
            archived: false,
            archived_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "User".to_string(),
            version: 1,
            tags: Vec::new(),
            assigned_agent: None,
        }
    }

    #[test]
    fn compose_scratchpad_prompt_includes_title_and_mcp_instructions() {
        let pad = scratchpad("Launch notes", "draft the release checklist");
        let prompt = compose_scratchpad_prompt(&pad, None);

        assert!(prompt.contains(&format!("Scratchpad id: {}", pad.id)));
        assert!(prompt.contains("Title: Launch notes"));
        // Content isn't embedded — the agent fetches it via list_scratchpads.
        assert!(!prompt.contains("draft the release checklist"));
        // The standing instructions name each tool the agent should use.
        assert!(prompt.contains("list_scratchpads"));
        assert!(prompt.contains("update_scratchpad"));
        assert!(prompt.contains("add_scratchpad_tag"));
        assert!(prompt.contains("remove_scratchpad_tag"));
        // No user prompt, so no additional-instructions block.
        assert!(!prompt.contains("Additional instructions:"));
    }

    #[test]
    fn compose_scratchpad_prompt_appends_user_prompt() {
        let pad = scratchpad("Launch notes", "draft the release checklist");
        let prompt = compose_scratchpad_prompt(&pad, Some("focus on the changelog"));

        assert!(prompt.contains("Additional instructions:\nfocus on the changelog"));
    }

    #[test]
    fn compose_scratchpads_prompt_single_matches_single_helper() {
        // One scratchpad routes through the original single-scratchpad phrasing.
        let pad = scratchpad("Launch notes", "draft the release checklist");
        let one = compose_scratchpads_prompt(std::slice::from_ref(&pad), Some("go"));
        assert_eq!(one, compose_scratchpad_prompt(&pad, Some("go")));
    }

    #[test]
    fn compose_scratchpads_prompt_combines_multiple_scratchpads() {
        let a = scratchpad("Launch notes", "draft the release checklist");
        let b = scratchpad("Bug list", "triage open reports");
        let prompt =
            compose_scratchpads_prompt(&[a.clone(), b.clone()], Some("start with launch notes"));

        // Presented as one combined task listing every scratchpad id and title.
        assert!(prompt.contains("2 Podium scratchpads as a single task"));
        assert!(prompt.contains("Scratchpad 1 of 2"));
        assert!(prompt.contains("Scratchpad 2 of 2"));
        assert!(prompt.contains(&format!("Scratchpad id: {}", a.id)));
        assert!(prompt.contains(&format!("Scratchpad id: {}", b.id)));
        assert!(prompt.contains("Title: Launch notes"));
        assert!(prompt.contains("Title: Bug list"));
        // Content isn't embedded for either scratchpad.
        assert!(!prompt.contains("draft the release checklist"));
        assert!(!prompt.contains("triage open reports"));
        // The standing MCP instructions and user prompt appear once.
        assert!(prompt.contains("update_scratchpad"));
        assert!(prompt.contains("Additional instructions:\nstart with launch notes"));
    }

    #[test]
    fn name_from_prompt_takes_first_line_and_truncates_on_word_boundary() {
        assert_eq!(
            name_from_prompt("fix the bug").as_deref(),
            Some("fix the bug")
        );
        // Leading blank lines skipped; first real line used.
        assert_eq!(
            name_from_prompt("\n  \nrename sessions").as_deref(),
            Some("rename sessions")
        );
        // Long single line truncates on a word break with an ellipsis, and
        // never exceeds the char budget before the ellipsis.
        let n = name_from_prompt(
            "investigate the terminal height clipping problem in agent windows please",
        )
        .unwrap();
        assert!(n.ends_with('…'));
        assert!(n.trim_end_matches('…').chars().count() <= 40);
        assert!(!n.trim_end_matches('…').ends_with(' '));
        // Empty / whitespace-only prompt yields nothing.
        assert_eq!(name_from_prompt("   \n  "), None);
    }

    #[tokio::test]
    async fn projects_list_in_open_order_then_reorder() {
        let (a, b, c) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let orch = Orchestrator::new();
        let ida = orch.open_project(a.path().to_path_buf()).await.unwrap();
        let idb = orch.open_project(b.path().to_path_buf()).await.unwrap();
        let idc = orch.open_project(c.path().to_path_buf()).await.unwrap();

        let ids: Vec<_> = orch.list_projects().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![ida, idb, idc], "listed in open order");

        let reordered: Vec<_> = orch
            .reorder_projects(vec![idc, ida, idb])
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(reordered, vec![idc, ida, idb]);
        let ids: Vec<_> = orch.list_projects().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![idc, ida, idb], "list reflects the new order");
    }

    #[tokio::test]
    async fn issue_setups_save_normalize_and_pin_per_project() {
        use crate::issues::IssueProvider;
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();

        // GitHub needs no keychain: account kept, URL normalized, name defaults to host.
        let o = orch
            .save_issue_setup(
                None,
                " ".into(),
                IssueProvider::Github,
                "GitHub.com/".into(),
                Some(" octo-fictional ".into()),
                Some("ignored for GitHub".into()),
            )
            .unwrap();
        let setup = &o.setups[0];
        assert_eq!(setup.setup.url, "https://github.com");
        assert_eq!(setup.setup.name, "github.com");
        assert_eq!(setup.setup.account.as_deref(), Some("octo-fictional"));
        assert!(!setup.token_set);
        assert!(matches!(
            orch.save_issue_setup(
                None,
                "x".into(),
                IssueProvider::Github,
                "ftp://nope".into(),
                None,
                None
            ),
            Err(CoreError::InvalidInput(_))
        ));

        // Editing by id replaces in place.
        let sid = setup.setup.id.clone();
        let o = orch
            .save_issue_setup(
                Some(sid.clone()),
                "Personal".into(),
                IssueProvider::Github,
                "https://github.com".into(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(o.setups.len(), 1);
        assert_eq!(o.setups[0].setup.name, "Personal");

        let root = orch.project_root(id).unwrap();
        let o = orch.set_project_issue_setup(id, Some(sid.clone())).unwrap();
        assert_eq!(o.projects.get(&root), Some(&sid));
        assert!(orch
            .set_project_issue_setup(id, Some("missing".into()))
            .is_err());
        let o = orch.set_project_issue_setup(id, None).unwrap();
        assert!(o.projects.is_empty());
    }

    // A `\\?\`-prefixed root breaks agent spawn on Windows: it becomes the
    // agent's `cwd` (`process::pty::PtyProcess::spawn` → portable-pty's
    // `CommandBuilder::cwd`), and `CreateProcessW`'s `lpCurrentDirectory`
    // rejects that prefix with `ERROR_ACCESS_DENIED` (os error 5).
    #[tokio::test]
    #[cfg(windows)]
    async fn opened_project_root_has_no_verbatim_prefix_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let info = orch
            .list_projects()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert!(
            !info.root.to_string_lossy().starts_with(r"\\?\"),
            "project root must not be a verbatim path: {:?}",
            info.root
        );
    }

    // The `dunce` switch above changes an existing Windows project's
    // canonical identity, which would otherwise orphan its to-dos and
    // scratchpads (stored keyed by that identity's string) under the old
    // `\\?\`-prefixed key. `open_project` must migrate them onto the new key.
    #[tokio::test]
    #[cfg(windows)]
    async fn open_project_migrates_legacy_keyed_todos_and_scratchpads() {
        let dir = tempfile::tempdir().unwrap();
        // Seed data under the pre-fix (verbatim) key, as an older build would
        // have stored it.
        let legacy_root = std::fs::canonicalize(dir.path()).unwrap();
        let orch = Orchestrator::new();
        orch.todos
            .add(ProjectId::new(), &legacy_root, "pre-existing todo", None)
            .unwrap();
        orch.scratchpads
            .add(ProjectId::new(), &legacy_root, "tester")
            .unwrap();

        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();

        let todos = orch.list_todos(id).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "pre-existing todo");
        assert_eq!(orch.list_scratchpads(id).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reorder_ignores_unknown_and_appends_missing() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let orch = Orchestrator::new();
        let ida = orch.open_project(a.path().to_path_buf()).await.unwrap();
        let idb = orch.open_project(b.path().to_path_buf()).await.unwrap();
        // Mention only b plus a bogus id; a is appended at the end.
        let ids: Vec<_> = orch
            .reorder_projects(vec![idb, ProjectId::new()])
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec![idb, ida]);
    }

    #[tokio::test]
    async fn rename_overrides_name_and_initials_then_clears() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let original = orch
            .list_projects()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();

        let info = orch
            .rename_project(id, Some("  My Cool Project  ".to_string()))
            .unwrap();
        assert_eq!(info.name, "My Cool Project", "trimmed override wins");
        assert_eq!(info.icon_initials, "MC", "initials derive from override");

        // A blank name clears the override, reverting to the folder name.
        let cleared = orch.rename_project(id, Some("   ".to_string())).unwrap();
        assert_eq!(cleared.name, original.name);
        assert_eq!(cleared.icon_initials, original.icon_initials);
    }

    #[tokio::test]
    async fn set_project_sections_updates_info_and_emits() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let opened = orch
            .list_projects()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(opened.sections, ProjectSections::default());

        let mut rx = orch.subscribe_events();
        let sections = ProjectSections {
            scratchpads: false,
            ..ProjectSections::default()
        };
        let info = orch.set_project_sections(id, sections).unwrap();
        assert!(!info.sections.scratchpads);
        assert!(info.sections.todos);
        let listed = orch
            .list_projects()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(listed.sections, sections);
        assert!(matches!(
            rx.try_recv(),
            Ok(PodiumEvent::ProjectUpdated { project_id }) if project_id == id
        ));
        assert!(matches!(
            orch.set_project_sections(ProjectId::new(), sections),
            Err(CoreError::ProjectNotFound)
        ));
    }

    #[tokio::test]
    async fn project_color_sets_validates_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();

        let info = orch
            .set_project_color(id, Some(" #3E63DD ".to_string()))
            .unwrap();
        assert_eq!(info.color.as_deref(), Some("#3e63dd"));

        for bad in ["red", "#12345", "#1234567", "#gggggg", "3e63dd"] {
            assert!(
                orch.set_project_color(id, Some(bad.to_string())).is_err(),
                "{bad}"
            );
        }
        let still = orch
            .list_projects()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(
            still.color.as_deref(),
            Some("#3e63dd"),
            "a refused value keeps the old one"
        );

        let cleared = orch.set_project_color(id, Some("  ".to_string())).unwrap();
        assert_eq!(cleared.color, None);
    }

    #[tokio::test]
    async fn restore_project_overrides_applies_both_in_one_event() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let mut rx = orch.subscribe_events();

        let info = orch
            .restore_project_overrides(
                id,
                Some("Webshop".into()),
                Some("#E5484D".into()),
                Some(ProjectSections {
                    scratchpads: false,
                    ..ProjectSections::default()
                }),
            )
            .unwrap();
        assert_eq!(info.name, "Webshop");
        assert_eq!(info.color.as_deref(), Some("#e5484d"));
        assert!(!info.sections.scratchpads, "sections ride along");

        let mut updates = 0;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, PodiumEvent::ProjectUpdated { project_id } if project_id == id) {
                updates += 1;
            }
        }
        assert_eq!(
            updates, 1,
            "one ProjectUpdated for name + colour + sections"
        );

        let info = orch
            .restore_project_overrides(id, Some("Shop".into()), Some("red".into()), None)
            .unwrap();
        assert_eq!(info.name, "Shop", "a bad stored colour keeps the name");
        assert_eq!(
            info.color.as_deref(),
            Some("#e5484d"),
            "a bad stored colour is ignored, not applied"
        );

        // Only sections stored: the name and colour stay as they are.
        let info = orch
            .restore_project_overrides(
                id,
                None,
                None,
                Some(ProjectSections {
                    todos: false,
                    ..ProjectSections::default()
                }),
            )
            .unwrap();
        assert_eq!(info.name, "Shop");
        assert_eq!(info.color.as_deref(), Some("#e5484d"));
        assert!(!info.sections.todos);
    }

    #[tokio::test]
    async fn rename_process_updates_name_and_rejects_blank() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let spec = ProcessSpec {
            name: "term".to_string(),
            command: "true".to_string(),
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
            kind: ProcessKind::Terminal,
            restart_policy: RestartPolicy::Never,
            color: None,
        };
        let id = orch.add_process(project_id, spec).await.unwrap();

        let info = orch.rename_process(id, "  My Terminal  ").unwrap();
        assert_eq!(info.name, "My Terminal", "trimmed name wins");
        let listed = orch
            .list_processes(Some(project_id))
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(listed.name, "My Terminal", "rename persists in the list");

        assert!(matches!(
            orch.rename_process(id, "   "),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            orch.rename_process(ProcessId::new(), "x"),
            Err(CoreError::ProcessNotFound)
        ));
    }

    #[tokio::test]
    async fn close_removes_from_order() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let orch = Orchestrator::new();
        let ida = orch.open_project(a.path().to_path_buf()).await.unwrap();
        let idb = orch.open_project(b.path().to_path_buf()).await.unwrap();
        orch.close_project(ida).await.unwrap();
        let ids: Vec<_> = orch.list_projects().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![idb]);
    }

    #[tokio::test]
    async fn add_scratchpad_publishes_scratchpads_changed_event() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let mut rx = orch.subscribe_events();

        let added = orch.add_scratchpad(project_id, "User").unwrap();
        assert_eq!(added.title, "Untitled scratchpad");

        let event = rx.try_recv().expect("event published");
        assert!(matches!(
            event,
            PodiumEvent::ScratchpadsChanged { project_id: pid } if pid == project_id
        ));
    }

    #[tokio::test]
    async fn update_scratchpad_content_publishes_scratchpads_changed_event() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let added = orch.add_scratchpad(project_id, "User").unwrap();
        let mut rx = orch.subscribe_events();

        let updated = orch
            .update_scratchpad_content(project_id, added.id, "hello", added.updated_at, "claude")
            .unwrap();
        assert_eq!(updated.content, "hello");
        assert_eq!(updated.version, 2);

        let event = rx.try_recv().expect("event published");
        assert!(matches!(
            event,
            PodiumEvent::ScratchpadsChanged { project_id: pid } if pid == project_id
        ));
    }

    #[tokio::test]
    async fn list_scratchpads_does_not_publish_event() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        orch.add_scratchpad(project_id, "User").unwrap();
        let mut rx = orch.subscribe_events();

        let listed = orch.list_scratchpads(project_id).unwrap();
        assert_eq!(listed.len(), 1);

        assert!(rx.try_recv().is_err(), "list must not publish an event");
    }

    /// Simulates a concurrent user + agent edit: the user's stale
    /// `expected_updated_at` is rejected once the agent's edit has landed,
    /// instead of silently clobbering it.
    #[tokio::test]
    async fn concurrent_update_with_stale_timestamp_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let added = orch.add_scratchpad(project_id, "User").unwrap();

        // The agent's edit lands first.
        let agent_edit = orch
            .update_scratchpad_content(
                project_id,
                added.id,
                "agent wrote this",
                added.updated_at,
                "claude",
            )
            .unwrap();

        // The user's UI still holds the pre-agent-edit timestamp.
        let result = orch.update_scratchpad_content(
            project_id,
            added.id,
            "user's stale edit",
            added.updated_at,
            "User",
        );
        assert!(matches!(result, Err(CoreError::ScratchpadConflict)));

        // Retrying with the fresh timestamp succeeds.
        let resolved = orch
            .update_scratchpad_content(
                project_id,
                added.id,
                "user's edit after reload",
                agent_edit.updated_at,
                "User",
            )
            .unwrap();
        assert_eq!(resolved.content, "user's edit after reload");
    }

    #[tokio::test]
    async fn scratchpad_tags_add_remove_and_archive_publish_events() {
        let dir = tempfile::tempdir().unwrap();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let added = orch.add_scratchpad(project_id, "User").unwrap();
        let mut rx = orch.subscribe_events();

        let tagged = orch
            .add_scratchpad_tag(project_id, added.id, "urgent")
            .unwrap();
        assert_eq!(tagged.tags, vec!["urgent".to_string()]);
        assert!(matches!(
            rx.try_recv().unwrap(),
            PodiumEvent::ScratchpadsChanged { project_id: pid } if pid == project_id
        ));

        let untagged = orch
            .remove_scratchpad_tag(project_id, added.id, "urgent")
            .unwrap();
        assert!(untagged.tags.is_empty());
        assert!(matches!(
            rx.try_recv().unwrap(),
            PodiumEvent::ScratchpadsChanged { project_id: pid } if pid == project_id
        ));

        let archived = orch
            .set_scratchpad_archived(project_id, added.id, true)
            .unwrap();
        assert!(archived.archived);
        assert!(orch.list_scratchpads(project_id).unwrap().is_empty());
        assert_eq!(orch.list_archived_scratchpads(project_id).unwrap().len(), 1);
        assert!(matches!(
            rx.try_recv().unwrap(),
            PodiumEvent::ScratchpadsChanged { project_id: pid } if pid == project_id
        ));
    }

    /// A throwaway git repo with one commit, identity pinned so commits work
    /// on machines without global git config (mirrors the same helper in
    /// `worktree.rs`'s own tests and the MCP integration tests).
    fn init_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.name", "Test"]);
        run(&["config", "user.email", "test@example.com"]);
        std::fs::write(dir.path().join("README.md"), "sample\n").expect("write file");
        run(&["add", "."]);
        run(&["commit", "-m", "initial"]);
        dir
    }

    #[tokio::test]
    async fn create_and_remove_worktree_publish_worktrees_changed_events() {
        let dir = init_git_repo();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        let mut rx = orch.subscribe_events();

        let created = orch.create_worktree(project_id, "Fix Login").unwrap();
        assert_eq!(created.name, "fix-login");
        assert!(matches!(
            rx.try_recv().unwrap(),
            PodiumEvent::WorktreesChanged { project_id: pid } if pid == project_id
        ));

        orch.remove_worktree(project_id, "fix-login", false)
            .unwrap();
        assert!(matches!(
            rx.try_recv().unwrap(),
            PodiumEvent::WorktreesChanged { project_id: pid } if pid == project_id
        ));
    }

    #[tokio::test]
    async fn open_worktree_in_warp_rejects_an_unknown_name() {
        let dir = init_git_repo();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        // Only the error path is testable — a valid name would launch Warp.
        assert!(matches!(
            orch.open_worktree_in_warp(project_id, "nope"),
            Err(CoreError::WorktreeNotFound)
        ));
    }

    #[tokio::test]
    async fn list_worktrees_does_not_publish_an_event() {
        let dir = init_git_repo();
        let orch = Orchestrator::new();
        let project_id = orch.open_project(dir.path().to_path_buf()).await.unwrap();
        orch.create_worktree(project_id, "mine").unwrap();
        let mut rx = orch.subscribe_events();

        let listed = orch.list_worktrees(project_id).unwrap();
        assert_eq!(listed.len(), 1);

        assert!(rx.try_recv().is_err(), "list must not publish an event");
    }

    /// Agent adapter for the session-memory tests: every launch prints its
    /// session flag and id as `<flag><id>` and then stays up, and the CLI
    /// "has a transcript" only for the ids in `stored`. With `fail_resume`,
    /// a `--resume` launch exits non-zero at once, like a CLI refusing it.
    /// `worktrees` records each launch's worktree request.
    struct MemoryAdapter {
        stored: Mutex<Vec<String>>,
        fail_resume: bool,
        worktrees: Mutex<Vec<(bool, Option<String>)>>,
    }

    impl MemoryAdapter {
        fn new(fail_resume: bool) -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(Vec::new()),
                fail_resume,
                worktrees: Mutex::new(Vec::new()),
            })
        }

        fn store(&self, session_id: &str) {
            self.stored.lock().unwrap().push(session_id.to_string());
        }
    }

    impl crate::agent::AgentAdapter for MemoryAdapter {
        fn id(&self) -> &'static str {
            "memory"
        }

        fn display_name(&self) -> &'static str {
            "Memory"
        }

        fn binary(&self) -> &'static str {
            "memory-agent"
        }

        fn build_launch(&self, ctx: &AgentLaunchCtx) -> CoreResult<crate::agent::LaunchPlan> {
            self.worktrees
                .lock()
                .unwrap()
                .push((ctx.worktree, ctx.worktree_name.map(str::to_string)));
            let (bin, tail) = if self.fail_resume {
                (
                    r#"f() { printf '<%s>' "$@"; [ "$1" != --resume ] || exit 3; sleep 30; }; f"#,
                    "",
                )
            } else {
                (r#"printf '<%s>'"#, "; sleep 30")
            };
            let restart = RestartCommands {
                bin: bin.to_string(),
                prompt: None,
                tail: tail.to_string(),
                new_flag: "--session-id",
                resume_flag: "--resume",
            };
            Ok(crate::agent::LaunchPlan {
                command: restart.fresh(&ctx.process_id.to_string()),
                env: Vec::new(),
                restart: Some(restart),
            })
        }

        fn session_exists(&self, session_id: &str, _wsl: Option<&WslBackend>) -> bool {
            self.stored.lock().unwrap().iter().any(|s| s == session_id)
        }

        fn is_available(&self, _wsl: Option<&WslBackend>) -> bool {
            true
        }
    }

    /// An orchestrator on `adapter`, persisting under `data`.
    fn memory_orch(data: &Path, adapter: Arc<MemoryAdapter>) -> Orchestrator {
        let orch = Orchestrator::new().with_adapters(AdapterRegistry::new(vec![adapter]));
        orch.set_agent_settings_path(data.join("agents.json"));
        orch.set_session_memory_path(data.join("session_memory.json"));
        orch.set_todos_path(data.join("todos.json"));
        orch.set_scratchpads_path(data.join("scratchpads.json"));
        orch
    }

    async fn spawn_memory_agent(
        orch: &Orchestrator,
        project: ProjectId,
        name: &str,
        todo_ids: Vec<TodoId>,
        args: Option<Vec<String>>,
    ) -> ProcessId {
        orch.spawn_agent(
            project,
            Some("memory".to_string()),
            Some(name.to_string()),
            None,
            todo_ids,
            Vec::new(),
            false,
            None,
            args,
        )
        .await
        .unwrap()
    }

    /// Wait until agent `id` printed a launch line `<flag><session id>` that
    /// satisfies `done`, and return it.
    async fn wait_for_launch(
        orch: &Orchestrator,
        id: ProcessId,
        done: impl Fn(&str, &str) -> bool,
    ) -> (String, String) {
        let mut last = (String::new(), String::new());
        for _ in 0..200 {
            let out = orch.tail_text(id, 4096).await.unwrap();
            if let Some(at) = out.rfind("<--") {
                let mut parts = out[at + 1..].split('>');
                let flag = parts.next().unwrap_or("").to_string();
                let session = parts.next().unwrap_or("").trim_start_matches('<');
                last = (flag, session.to_string());
                // A whole UUID, so a half-flushed line is never taken.
                if last.1.len() == 36 && done(&last.0, &last.1) {
                    return last;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("launch never matched, last saw {last:?}");
    }

    fn remembered(session_id: &str, updated_at: DateTime<Utc>) -> RememberedSession {
        RememberedSession {
            adapter_id: "memory".to_string(),
            name: "agent-1".to_string(),
            session_id: session_id.to_string(),
            updated_at,
            todo_ids: Vec::new(),
            scratchpad_ids: Vec::new(),
            args: None,
            worktree: false,
            worktree_name: None,
        }
    }

    const SESS_1: &str = "00000000-0000-4000-8000-000000000001";

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_remembered_session_resumes_on_the_next_open() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        let orch = memory_orch(data.path(), adapter.clone());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = spawn_memory_agent(&orch, id, "agent-1", Vec::new(), None).await;
        let session = agent.to_string();
        wait_for_launch(&orch, agent, |flag, s| {
            flag == "--session-id" && s == session
        })
        .await;
        orch.shutdown().await;
        let listed = orch.session_memory.list(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, session, "the Podium-assigned id");

        // The next run: the CLI has the transcript, so the agent comes back
        // on that session through the restart path's `--resume`.
        adapter.store(&session);
        let restarted = memory_orch(data.path(), adapter);
        let id = restarted.open_project(root.clone()).await.unwrap();
        let procs = restarted.list_processes(Some(id));
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "agent-1");
        wait_for_launch(&restarted, procs[0].id, |flag, s| {
            flag == "--resume" && s == session
        })
        .await;
        {
            let inner = restarted.inner.lock().unwrap();
            let s = inner.processes[&procs[0].id].session.as_ref().unwrap();
            assert_eq!(s.id, session);
            assert!(s.resumed);
        }
        assert!(
            restarted.session_memory.list(&root).is_empty(),
            "restored entries are cleared"
        );

        // Quit again: still the same conversation, so it is remembered again.
        restarted.shutdown().await;
        let listed = restarted.session_memory.list(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, session);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_session_the_cli_no_longer_has_is_not_restored() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = memory_orch(data.path(), MemoryAdapter::new(false));
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        spawn_memory_agent(&orch, id, "agent-1", Vec::new(), None).await;
        orch.shutdown().await;
        assert_eq!(orch.session_memory.list(&root).len(), 1, "captured");

        // Pruned by the CLI's retention in between: nothing to resume.
        let restarted = memory_orch(data.path(), MemoryAdapter::new(false));
        let id = restarted.open_project(root.clone()).await.unwrap();
        assert!(restarted.list_processes(Some(id)).is_empty());
        assert!(restarted.session_memory.list(&root).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_restore_that_fails_to_resume_starts_fresh_next_time() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(true);
        adapter.store(SESS_1);
        let orch = memory_orch(data.path(), adapter);
        orch.set_agent_remember_sessions(true).unwrap();
        orch.session_memory
            .remember(&root, remembered(SESS_1, Utc::now()))
            .unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = orch.list_processes(Some(id))[0].id;
        // The resume exits at once, so the restart path's failure window
        // marks it, exactly as for a restarted agent.
        let resume_failed = || {
            orch.inner.lock().unwrap().processes[&agent]
                .session
                .as_ref()
                .is_some_and(|s| s.resume_failed)
        };
        for _ in 0..200 {
            if resume_failed() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(resume_failed());
        orch.start_process(agent).await.unwrap();
        wait_for_launch(&orch, agent, |flag, s| {
            flag == "--session-id" && s != SESS_1
        })
        .await;
        orch.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn toggle_off_skips_restore_even_with_remembered_sessions() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        adapter.store(SESS_1);
        let orch = memory_orch(data.path(), adapter);
        orch.session_memory
            .remember(&root, remembered(SESS_1, Utc::now()))
            .unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        assert!(
            orch.list_processes(Some(id)).is_empty(),
            "no agent auto-spawns with the toggle off"
        );
        assert!(
            orch.session_memory.list(&root).is_empty(),
            "entries are dropped on open even with the toggle off"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn turning_the_setting_off_forgets_every_captured_session() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        adapter.store(SESS_1);
        let orch = memory_orch(data.path(), adapter);
        orch.session_memory
            .remember(&root, remembered(SESS_1, Utc::now()))
            .unwrap();
        // Off, then on again before the project ever reopens.
        orch.set_agent_remember_sessions(false).unwrap();
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        assert!(orch.list_processes(Some(id)).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_expired_or_invalid_remembered_session_is_not_restored() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        // Both "stored", so only the age and id checks can reject them.
        adapter.store(SESS_1);
        adapter.store("-x");
        let orch = memory_orch(data.path(), adapter);
        orch.set_agent_remember_sessions(true).unwrap();
        let expired = Utc::now() - session_memory::MAX_AGE - chrono::TimeDelta::hours(1);
        orch.session_memory
            .remember_all(vec![
                (root.clone(), remembered(SESS_1, expired)),
                // A tampered file: an id that would read as a flag.
                (root.clone(), remembered("-x", Utc::now())),
            ])
            .unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        assert!(orch.list_processes(Some(id)).is_empty());
        assert!(orch.session_memory.list(&root).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_restored_agent_owns_its_todos_and_keeps_its_args() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        let orch = memory_orch(data.path(), adapter.clone());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let owned = orch.add_todo(id, "Fictional task", None).unwrap();
        let finished = orch.add_todo(id, "Fictional finished task", None).unwrap();
        let pad = orch.add_scratchpad(id, "you").unwrap();
        let args = vec!["--model".to_string(), "opus".to_string()];
        let agent =
            spawn_memory_agent(&orch, id, "agent-1", vec![owned.id], Some(args.clone())).await;
        orch.assign_todo(id, finished.id, agent).unwrap();
        orch.assign_scratchpads(id, agent, &[pad.id]);
        orch.shutdown().await;
        // Done before the restart: nothing left to own.
        orch.set_todo_done(id, finished.id, true).unwrap();

        adapter.store(&agent.to_string());
        let restarted = memory_orch(data.path(), adapter);
        let id = restarted.open_project(root.clone()).await.unwrap();
        let resumed = restarted.list_processes(Some(id))[0].id;
        assert_eq!(
            restarted.inner.lock().unwrap().processes[&resumed].spawn_args,
            Some(args)
        );
        let todos = restarted.list_todos(id).unwrap();
        let owner = |tid| {
            todos
                .iter()
                .find(|t| t.id == tid)
                .and_then(|t| t.assigned_agent.as_ref())
                .map(|a| a.process_id)
        };
        assert_eq!(owner(owned.id), Some(resumed));
        assert_eq!(owner(finished.id), None);
        let pads = restarted.list_scratchpads(id).unwrap();
        assert_eq!(
            pads.iter()
                .find(|s| s.id == pad.id)
                .and_then(|s| s.assigned_agent.as_ref())
                .map(|a| a.process_id),
            Some(resumed)
        );
        restarted.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restore_after_the_quit_began_spawns_nothing_and_keeps_its_entries() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        adapter.store(SESS_1);
        let orch = memory_orch(data.path(), adapter);
        orch.set_agent_remember_sessions(true).unwrap();
        orch.session_memory
            .remember(&root, remembered(SESS_1, Utc::now()))
            .unwrap();
        // The quit lands before the restore reaches its spawn.
        orch.shutdown().await;
        let id = orch.open_project(root.clone()).await.unwrap();
        assert!(orch.list_processes(Some(id)).is_empty());
        let kept = orch.session_memory.list(&root);
        assert_eq!(kept.len(), 1, "kept for the next run");
        assert_eq!(kept[0].session_id, SESS_1);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_restored_agent_keeps_its_worktree_request() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        let orch = memory_orch(data.path(), adapter.clone());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = orch
            .spawn_agent(
                id,
                Some("memory".to_string()),
                Some("agent-1".to_string()),
                None,
                Vec::new(),
                Vec::new(),
                true,
                Some("fictional-wt".to_string()),
                None,
            )
            .await
            .unwrap();
        orch.shutdown().await;
        let listed = orch.session_memory.list(&root);
        assert!(listed[0].worktree);
        assert_eq!(listed[0].worktree_name.as_deref(), Some("fictional-wt"));

        adapter.worktrees.lock().unwrap().clear();
        adapter.store(&agent.to_string());
        let restarted = memory_orch(data.path(), adapter.clone());
        let id = restarted.open_project(root.clone()).await.unwrap();
        assert_eq!(restarted.list_processes(Some(id)).len(), 1);
        // The restored launch was built with the same request, so its
        // identity prompt matches the original's.
        assert_eq!(
            *adapter.worktrees.lock().unwrap(),
            vec![(true, Some("fictional-wt".to_string()))]
        );
        restarted.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn a_todo_a_live_agent_owns_is_not_reassigned_on_restore() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let orch = memory_orch(data.path(), MemoryAdapter::new(false));
        let root = dunce::canonicalize(project.path()).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let taken = orch.add_todo(id, "Fictional taken task", None).unwrap();
        let free = orch.add_todo(id, "Fictional free task", None).unwrap();
        let other = spawn_memory_agent(&orch, id, "other", vec![taken.id], None).await;
        let restored = spawn_memory_agent(&orch, id, "restored", Vec::new(), None).await;
        orch.reassign_restored(id, &root, restored, &[taken.id, free.id], &[]);
        let todos = orch.list_todos(id).unwrap();
        let owner = |tid| {
            todos
                .iter()
                .find(|t| t.id == tid)
                .and_then(|t| t.assigned_agent.as_ref())
                .map(|a| a.process_id)
        };
        assert_eq!(owner(taken.id), Some(other));
        assert_eq!(owner(free.id), Some(restored));
        orch.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "POSIX shell in MemoryAdapter")]
    async fn two_concurrent_opens_restore_one_agent() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let adapter = MemoryAdapter::new(false);
        adapter.store(SESS_1);
        let orch = memory_orch(data.path(), adapter);
        orch.set_agent_remember_sessions(true).unwrap();
        orch.session_memory
            .remember(&root, remembered(SESS_1, Utc::now()))
            .unwrap();
        // As React StrictMode's double startup restore does.
        let (a, b) = tokio::join!(
            orch.open_project(root.clone()),
            orch.open_project(root.clone())
        );
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(orch.list_processes(None).len(), 1);
        orch.shutdown().await;
    }

    /// The real Claude Code adapter: an agent whose own args pick the
    /// session has no Podium-assigned one, so there is nothing to remember.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg(unix)]
    async fn an_agent_whose_args_pick_the_session_is_not_remembered() {
        use std::os::unix::fs::PermissionsExt;
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let script = data.path().join("fake-claude");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let orch = Orchestrator::new();
        orch.set_agent_settings_path(data.path().join("agents.json"));
        orch.set_session_memory_path(data.path().join("session_memory.json"));
        orch.set_agent_override(
            "claude-code",
            Some(script.to_string_lossy().into_owned()),
            vec!["--continue".to_string()],
        )
        .unwrap();
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        orch.spawn_agent(
            id,
            Some("claude-code".to_string()),
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
        )
        .await
        .unwrap();
        orch.shutdown().await;
        assert!(orch.session_memory.list(&root).is_empty());
    }
}
