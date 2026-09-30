//! The single public API surface over projects and PTY-backed processes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::agent::session_memory::{self, RememberedSession, SessionMemoryStore};
use crate::agent::settings::{self, AgentSettings, AgentSettingsStore, MergeMode};
use crate::agent::{AdapterInfo, AdapterRegistry, AgentLaunchCtx, McpConnectInfo};
use crate::assignment::AssignedAgent;
use crate::assistant::{
    AssistantConfig, AssistantManager, AssistantPolling, Attachment, AutonomyMode, ProposalAction,
    ProposalInfo, TranscriptEntry,
};
use crate::config::AgentsConfig;
use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};
use crate::ids::{CommentId, LinkId, TodoId};
use crate::ids::{PendingId, ProcessId, ProjectId, ProposalId, ScratchpadId};
use crate::process::pty::{ExitCallback, PtyProcess, TermChunk};
use crate::process::scrollback::{OutputActivity, ScrollbackBuffer};
use crate::process::supervisor::{RestartState, SupervisorConfig};
use crate::process::{ProcessInfo, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy};
use crate::project::{self, ConfiguredProcess};
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
    /// The session id this agent was resumed with, kept so an idle restored
    /// session (no newer session file) is still remembered on shutdown.
    resumed_session_id: Option<String>,
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
    /// ids do not survive a restart), so it is never persisted; cleared when
    /// the agent exits, is removed, or its project closes.
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
    /// Global, cross-project agent settings (command override + default args
    /// per adapter, plus the merge mode).
    agent_settings: AgentSettingsStore,
    /// Agent sessions to resume on the next open of their project, keyed by
    /// root (`AgentSettings.remember_sessions`).
    session_memory: SessionMemoryStore,
    /// Where Claude Code keeps its session files (`~/.claude/projects`).
    claude_projects_root: Mutex<Option<PathBuf>>,
    /// The workspace-wide personal assistant session (chat panel).
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
            agent_settings: AgentSettingsStore::new(),
            session_memory: SessionMemoryStore::new(),
            claude_projects_root: Mutex::new(crate::platform::claude_projects_root()),
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

    /// Point the scratchpad store at its backing file (app data dir) and
    /// load it. Until this is called, scratchpads are held in memory only.
    pub fn set_scratchpads_path(&self, path: PathBuf) {
        self.scratchpads.set_path(path);
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
    pub fn set_assistant_autonomy(&self, mode: AutonomyMode) -> CoreResult<AutonomyMode> {
        Ok(self
            .agent_settings
            .set_assistant_autonomy(mode)?
            .assistant
            .autonomy)
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

    /// Point the global agent settings at their backing file (app data dir)
    /// and load them. Until this is called, settings are held in memory only.
    pub fn set_agent_settings_path(&self, path: PathBuf) {
        self.agent_settings.set_path(path);
    }

    /// Point the remembered-sessions store at its backing file (app data
    /// dir) and load it. Until this is called it is in-memory only.
    pub fn set_session_memory_path(&self, path: PathBuf) {
        self.session_memory.set_path(path);
    }

    /// Override where Claude Code session files are looked up (tests).
    pub fn set_claude_projects_root(&self, path: PathBuf) {
        *self.claude_projects_root.lock().expect(LOCK_POISONED) = Some(path);
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
    pub fn set_agent_remember_sessions(&self, enabled: bool) -> CoreResult<AgentSettings> {
        self.agent_settings.set_remember_sessions(enabled)
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
            if let Err(e) = do_start(&self.inner, &self.events, self.supervisor, pid, false) {
                tracing::warn!(process_id = %pid, "auto-start failed: {e}");
            }
        }
        if self.agent_settings.get().remember_sessions {
            self.restore_remembered_sessions(id, &path).await;
        }
        // Dropped even with the toggle off, so old sessions never resurface later.
        if let Err(e) = self.session_memory.clear(&path) {
            tracing::warn!("clearing remembered agent sessions failed: {e}");
        }
        Ok(id)
    }

    /// Respawn every agent remembered for `root`, each via `--resume`.
    /// Best-effort per entry; the caller clears the list afterwards.
    async fn restore_remembered_sessions(&self, project_id: ProjectId, root: &Path) {
        let projects_root = self
            .claude_projects_root
            .lock()
            .expect(LOCK_POISONED)
            .clone();
        for entry in self.session_memory.list(root) {
            // Without resume support this would be a fresh agent with no context.
            if !self.adapter_supports_resume(&entry.adapter_id) {
                continue;
            }
            // The file is user-writable; never hand `--resume` a non-UUID.
            if !session_memory::is_valid_session_id(&entry.session_id) {
                tracing::warn!("skipping a remembered agent session with an invalid id");
                continue;
            }
            // Pruned by Claude Code's retention: resuming would exit at once.
            if projects_root.as_deref().is_some_and(|pr| {
                !session_memory::session_file_exists(pr, &entry.cwd, &entry.session_id)
            }) {
                continue;
            }
            if let Err(e) = self
                .spawn_agent_inner(
                    project_id,
                    Some(entry.adapter_id),
                    Some(entry.name),
                    None,
                    Vec::new(),
                    Vec::new(),
                    false,
                    None,
                    None,
                    Some(entry.session_id),
                )
                .await
            {
                tracing::warn!("restoring a remembered agent session failed: {e}");
            }
        }
    }

    fn adapter_supports_resume(&self, adapter_id: &str) -> bool {
        self.adapters
            .by_id(adapter_id)
            .is_some_and(|a| a.supports_resume())
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
            handle.name_override = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
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
            // (a stale assignment whose process is gone resolves to `None`, so
            // it can be reclaimed; re-claiming your own is idempotent).
            if let Some(existing) = assigned_agent_of(&inner, todo_id) {
                if existing.process_id != process_id {
                    return Err(CoreError::TodoAlreadyAssigned);
                }
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
            if let Err(e) = do_start(&self.inner, &self.events, self.supervisor, pid, false) {
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
    /// adapter's binary on the login-shell `PATH`, which shells out — call
    /// from a blocking-friendly context.
    pub fn list_adapters(&self) -> Vec<AdapterInfo> {
        self.adapters.infos()
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
        self.spawn_agent_inner(
            project_id,
            adapter_id,
            name,
            prompt,
            todo_ids,
            scratchpad_ids,
            worktree,
            worktree_name,
            args_override,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)] // `spawn_agent` plus a resume id
    async fn spawn_agent_inner(
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
        resume_session_id: Option<String>,
    ) -> CoreResult<ProcessId> {
        let (root, agents, existing_names, session_color) = {
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
            let color = pick_session_color(&inner, project_id);
            (handle.root.clone(), handle.agents.clone(), names, color)
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
            prompt: final_prompt.as_deref(),
            extra_args: &merged_args,
            command_override,
            mcp: mcp.as_ref(),
            named,
            worktree,
            worktree_name: wt_name,
            suggest_worktree: global.suggest_worktree && !worktree,
            resume_session_id: resume_session_id.as_deref(),
        })?;
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
                p.resumed_session_id = resume_session_id;
            }
        }
        self.events.publish(PodiumEvent::ProcessAdded {
            project_id,
            process_id,
        });
        do_start(&self.inner, &self.events, self.supervisor, process_id, true)?;
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

    /// The project's Podium-managed git worktrees, with `in_use` marking the
    /// ones an active process runs in. Shells out to git — call from a
    /// blocking-friendly context.
    pub fn list_worktrees(&self, project_id: ProjectId) -> CoreResult<Vec<WorktreeInfo>> {
        let root = self.project_root(project_id)?;
        let mut infos = crate::worktree::list(&root)?;
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
        let cwd = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            inner
                .processes
                .get(&id)
                .ok_or(CoreError::ProcessNotFound)?
                .spec
                .cwd
                .clone()
        };
        Ok(crate::worktree::current_branch(&cwd))
    }

    /// Create a Podium-managed git worktree in the project (name slugified
    /// and de-duplicated). Shells out to git — call from a blocking-friendly
    /// context. Emits `WorktreesChanged` for the project.
    pub fn create_worktree(&self, project_id: ProjectId, name: &str) -> CoreResult<WorktreeInfo> {
        let root = self.project_root(project_id)?;
        let info = crate::worktree::create(&root, name)?;
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
        let root = self.project_root(project_id)?;
        let in_use = self
            .list_worktrees(project_id)?
            .iter()
            .any(|w| w.name == name && w.in_use);
        if in_use {
            return Err(CoreError::WorktreeInUse);
        }
        crate::worktree::remove(&root, name, force)?;
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
    pub async fn start_process(&self, id: ProcessId) -> CoreResult<()> {
        do_start(&self.inner, &self.events, self.supervisor, id, true)
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

    /// Remember each running resumable agent's Claude Code session id for the
    /// next open of its project. The session file is found by the
    /// `podium-<tag>` title Claude Code records for the agent's `-n` name, so
    /// several agents sharing a cwd each get their own. Without a title match
    /// it falls back to the id the agent was resumed with, then — only when it
    /// is the sole running agent in that cwd — the newest file since its spawn.
    fn capture_agent_sessions(&self) {
        struct Candidate {
            root: PathBuf,
            cwd: PathBuf,
            adapter_id: String,
            name: String,
            since: DateTime<Utc>,
            resumed: Option<String>,
            title: String,
        }
        let candidates: Vec<Candidate> = {
            let inner = self.inner.lock().expect(LOCK_POISONED);
            inner
                .processes
                .iter()
                .filter_map(|(id, p)| {
                    let ProcessKind::Agent { adapter } = &p.spec.kind else {
                        return None;
                    };
                    let ProcessStatus::Running { since, .. } = p.status else {
                        return None;
                    };
                    if !self.adapter_supports_resume(adapter) {
                        return None;
                    }
                    Some(Candidate {
                        root: inner.projects.get(&p.project_id)?.root.clone(),
                        cwd: p.spec.cwd.clone(),
                        adapter_id: adapter.clone(),
                        name: p.spec.name.clone(),
                        since,
                        resumed: p.resumed_session_id.clone(),
                        title: crate::agent::peer_session_name(&crate::agent::peer_tag(
                            &id.to_string(),
                        )),
                    })
                })
                .collect()
        };
        let projects_root = self
            .claude_projects_root
            .lock()
            .expect(LOCK_POISONED)
            .clone();
        // Only the agent's own title or the id it was resumed with count. A
        // bare "newest file in this cwd" guess would also pick up a `claude`
        // session the user runs themselves in the same repo.
        for c in &candidates {
            let Some(session_id) = projects_root
                .as_deref()
                .and_then(|pr| {
                    session_memory::find_titled_session_id_in(pr, &c.cwd, &c.title, c.since.into())
                })
                .or_else(|| {
                    c.resumed
                        .clone()
                        .filter(|id| session_memory::is_valid_session_id(id))
                })
            else {
                continue;
            };
            let entry = RememberedSession {
                adapter_id: c.adapter_id.clone(),
                name: c.name.clone(),
                cwd: c.cwd.clone(),
                session_id,
                updated_at: Utc::now(),
            };
            if let Err(e) = self.session_memory.remember(&c.root, entry) {
                tracing::warn!("remembering an agent session failed: {e}");
            }
        }
    }
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
/// project; once the palette is exhausted, cycle by the used count.
// ponytail: two concurrent spawns could momentarily pick the same colour —
// cosmetic only, and the max is 8 agents = palette size, so it's rare.
fn pick_session_color(inner: &Inner, project_id: ProjectId) -> String {
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
    SESSION_COLORS
        .iter()
        .copied()
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
/// when an agent exits, is removed, or its project closes). Returns the
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
            resumed_session_id: None,
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
) -> CoreResult<()> {
    let mut guard = inner.lock().expect(LOCK_POISONED);
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
    let pty = PtyProcess::spawn(
        &proc.spec,
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
    // Publish while still holding the lock: the exit callback also takes
    // the lock, so an instantly-exiting child's Exited event can never be
    // observed before this Running event.
    events.publish(PodiumEvent::ProcessStatusChanged {
        project_id,
        process_id: id,
        status,
    });
    Ok(())
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
        let (status, restart, cleared) = {
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
            let ran_long = p
                .started_at
                .take()
                .is_some_and(|s| s.elapsed() >= supervisor.backoff_reset_after);
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
            // An exited agent that is not being restarted no longer works on
            // its to-dos/scratchpads; drop the assignments so the UI stops
            // showing it.
            let cleared = if restart.is_none() {
                clear_agent_assignments(&mut guard, id)
            } else {
                ClearedAssignments::default()
            };
            (status, restart, cleared)
        };
        events.publish(PodiumEvent::ProcessStatusChanged {
            project_id,
            process_id: id,
            status,
        });
        for pid in cleared.todos {
            events.publish(PodiumEvent::TodosChanged { project_id: pid });
        }
        for pid in cleared.scratchpads {
            events.publish(PodiumEvent::ScratchpadsChanged { project_id: pid });
        }
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
            if let Err(e) = do_start(&task_inner, &events, supervisor, id, false) {
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

    /// An orchestrator persisting under `data`, with Claude Code resolved to
    /// a script that just sleeps and the session lookup rooted in `data`.
    #[cfg(unix)]
    fn session_memory_orch(data: &Path) -> Orchestrator {
        use std::os::unix::fs::PermissionsExt;
        let script = data.join("fake-claude");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let orch = Orchestrator::new();
        orch.set_agent_settings_path(data.join("agents.json"));
        orch.set_session_memory_path(data.join("sessions.json"));
        orch.set_claude_projects_root(data.join("claude-projects"));
        orch.set_agent_override(
            "claude-code",
            Some(script.to_string_lossy().into_owned()),
            Vec::new(),
        )
        .unwrap();
        orch
    }

    /// Stand in for Claude Code writing its session file for `cwd`; `body`
    /// is the file content (e.g. a custom-title line).
    #[cfg(unix)]
    fn write_session_file_with(data: &Path, cwd: &Path, id: &str, body: &str) {
        // Written after the agent started, like a live session.
        std::thread::sleep(Duration::from_millis(20));
        let dir = data
            .join("claude-projects")
            .join(crate::agent::session_memory::sanitize_cwd(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{id}.jsonl")), body).unwrap();
    }

    #[cfg(unix)]
    fn write_session_file(data: &Path, cwd: &Path, id: &str) {
        write_session_file_with(data, cwd, id, "{}");
    }

    /// A session file carrying the custom-title line Claude Code writes for
    /// the agent `process`'s `-n podium-<tag>` name.
    #[cfg(unix)]
    fn write_titled_session_file(data: &Path, cwd: &Path, id: &str, process: ProcessId) {
        let title = crate::agent::peer_session_name(&crate::agent::peer_tag(&process.to_string()));
        let body = format!(
            "{{\"type\":\"custom-title\",\"customTitle\":\"{title}\",\"sessionId\":\"{id}\"}}\n"
        );
        write_session_file_with(data, cwd, id, &body);
    }

    #[cfg(unix)]
    async fn spawn_named(orch: &Orchestrator, project: ProjectId, name: &str) -> ProcessId {
        orch.spawn_agent(
            project,
            Some("claude-code".to_string()),
            Some(name.to_string()),
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
        )
        .await
        .unwrap()
    }

    // Only the #[cfg(unix)] session-memory tests below use these.
    #[cfg(unix)]
    const SESS_1: &str = "00000000-0000-4000-8000-000000000001";
    #[cfg(unix)]
    const SESS_2: &str = "00000000-0000-4000-8000-000000000002";
    #[cfg(unix)]
    const SESS_3: &str = "00000000-0000-4000-8000-000000000003";
    #[cfg(unix)]
    const SESS_4: &str = "00000000-0000-4000-8000-000000000004";

    #[tokio::test]
    #[cfg(unix)]
    async fn remembered_session_resumes_with_resume_flag_on_next_open() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = spawn_named(&orch, id, "agent-1").await;
        write_titled_session_file(data.path(), &root, SESS_1, agent);
        orch.shutdown().await;

        let restarted = session_memory_orch(data.path());
        let id = restarted.open_project(root.clone()).await.unwrap();
        let procs = restarted.list_processes(Some(id));
        let resumed = procs
            .iter()
            .find(|p| p.name == "agent-1")
            .expect("remembered agent respawned");
        assert!(
            resumed.command.contains(&format!("--resume {SESS_1}")),
            "command: {}",
            resumed.command
        );
        assert!(
            restarted.session_memory.list(&root).is_empty(),
            "restored entries are cleared"
        );
        restarted.shutdown().await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn toggle_off_skips_restore_even_with_remembered_sessions() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = spawn_named(&orch, id, "agent-1").await;
        write_titled_session_file(data.path(), &root, SESS_2, agent);
        orch.shutdown().await;
        assert_eq!(orch.session_memory.list(&root).len(), 1, "captured");

        let restarted = session_memory_orch(data.path());
        restarted.set_agent_remember_sessions(false).unwrap();
        let id = restarted.open_project(root.clone()).await.unwrap();
        assert!(
            restarted.list_processes(Some(id)).is_empty(),
            "no agent auto-spawns with the toggle off"
        );
        assert!(
            restarted.session_memory.list(&root).is_empty(),
            "entries are dropped on open even with the toggle off"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn idle_restored_session_is_remembered_again_on_shutdown() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = spawn_named(&orch, id, "agent-1").await;
        write_titled_session_file(data.path(), &root, SESS_4, agent);
        orch.shutdown().await;

        // Restored, then quit without the session file being touched again.
        let restarted = session_memory_orch(data.path());
        restarted.open_project(root.clone()).await.unwrap();
        restarted.shutdown().await;
        let listed = restarted.session_memory.list(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, SESS_4);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn agents_sharing_a_cwd_each_get_their_own_session_by_title() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let first = spawn_named(&orch, id, "agent-1").await;
        let second = spawn_named(&orch, id, "agent-2").await;
        write_titled_session_file(data.path(), &root, SESS_1, first);
        write_titled_session_file(data.path(), &root, SESS_2, second);
        orch.shutdown().await;

        let listed = orch.session_memory.list(&root);
        let id_of = |name: &str| {
            listed
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.session_id.clone())
        };
        assert_eq!(id_of("agent-1").as_deref(), Some(SESS_1));
        assert_eq!(id_of("agent-2").as_deref(), Some(SESS_2));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_session_without_the_agents_title_is_not_remembered() {
        // The user's own `claude` in the same repo, newer than the agent's
        // spawn: a lone agent there must not adopt it.
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        spawn_named(&orch, id, "agent-1").await;
        write_session_file(data.path(), &root, SESS_3);
        orch.shutdown().await;
        assert!(orch.session_memory.list(&root).is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn pruned_session_is_not_restored() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        orch.set_agent_remember_sessions(true).unwrap();
        let id = orch.open_project(root.clone()).await.unwrap();
        let agent = spawn_named(&orch, id, "agent-1").await;
        write_titled_session_file(data.path(), &root, SESS_1, agent);
        orch.shutdown().await;
        assert_eq!(orch.session_memory.list(&root).len(), 1, "captured");
        // Claude Code's retention removes the session file in between.
        std::fs::remove_dir_all(data.path().join("claude-projects")).unwrap();

        let restarted = session_memory_orch(data.path());
        let id = restarted.open_project(root.clone()).await.unwrap();
        assert!(restarted.list_processes(Some(id)).is_empty());
        assert!(restarted.session_memory.list(&root).is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn invalid_remembered_session_id_is_not_restored() {
        let (data, project) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = dunce::canonicalize(project.path()).unwrap();
        let orch = session_memory_orch(data.path());
        // A tampered file: an id that would read as a flag, with a matching
        // session file so only the id check can reject it.
        write_session_file(data.path(), &root, "-x");
        orch.session_memory
            .remember(
                &root,
                RememberedSession {
                    adapter_id: "claude-code".to_string(),
                    name: "agent-1".to_string(),
                    cwd: root.clone(),
                    session_id: "-x".to_string(),
                    updated_at: Utc::now(),
                },
            )
            .unwrap();

        let restarted = session_memory_orch(data.path());
        restarted.set_agent_remember_sessions(true).unwrap();
        let id = restarted.open_project(root.clone()).await.unwrap();
        assert!(restarted.list_processes(Some(id)).is_empty());
    }
}
