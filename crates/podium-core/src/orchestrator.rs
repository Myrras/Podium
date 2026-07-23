//! The single public API surface over projects and PTY-backed processes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::agent::settings::{self, AgentSettings, AgentSettingsStore, MergeMode};
use crate::agent::{AdapterInfo, AdapterRegistry, AgentLaunchCtx, McpConnectInfo};
use crate::assignment::AssignedAgent;
use crate::config::AgentsConfig;
use crate::error::{CoreError, CoreResult};
use crate::events::{EventBus, PodiumEvent};
use crate::ids::{CommentId, LinkId, TodoId};
use crate::ids::{ProcessId, ProjectId, ScratchpadId};
use crate::process::pty::{ExitCallback, PtyProcess, TermChunk};
use crate::process::scrollback::ScrollbackBuffer;
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
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            events: EventBus::new(),
            supervisor,
            adapters: AdapterRegistry::default(),
            mcp: Mutex::new(None),
            todos: TodoStore::new(),
            scratchpads: ScratchpadStore::new(),
            agent_settings: AgentSettingsStore::new(),
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

    /// Point the global agent settings at their backing file (app data dir)
    /// and load them. Until this is called, settings are held in memory only.
    pub fn set_agent_settings_path(&self, path: PathBuf) {
        self.agent_settings.set_path(path);
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
        // Canonicalize so the same folder reached via different spellings
        // (symlinks, trailing slash, `..` segments) maps to one identity.
        let path = std::fs::canonicalize(&path).unwrap_or(path);
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
        Ok(id)
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

    /// Add a to-do to a project. Blank text is rejected.
    pub fn add_todo(&self, project_id: ProjectId, text: &str) -> CoreResult<TodoInfo> {
        let root = self.project_root(project_id)?;
        let info = self.todos.add(project_id, &root, text)?;
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

    /// Create a new scratchpad in a project (auto-generated timestamp title,
    /// empty content). Freshly created, so it can never already be assigned.
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

    /// Replace a scratchpad's content (bumps its version). `expected_updated_at`
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

    /// Revise a scratchpad's title (blank falls back to a timestamp title).
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
    /// When `worktree` is true a fresh git worktree named after the agent is
    /// created under `.podium/worktrees/` and used as the agent's cwd (the
    /// project must be a git repo). `args_override`, when `Some`, replaces the
    /// global Settings → Agents default args for this spawn only (still merged
    /// with the project's `agents.extra_args` per the merge mode). On a start
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
        worktree_on_head: bool,
        args_override: Option<Vec<String>>,
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
        // A requested worktree is created before the launch plan so the
        // identity prompt can say the agent already runs in one. Its name is
        // an explicit `worktree_name` when given (the New agent dialog forces
        // one for multi-to-do/scratchpad spawns, where the window name would
        // otherwise be one arbitrary to-do's text), else the window name.
        // `worktree_on_head` leaves the checkout on a detached HEAD so the
        // agent picks the branch name itself.
        let wt = worktree
            .then(|| {
                let wt_name = worktree_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(name.as_str());
                crate::worktree::create(&root, wt_name, worktree_on_head)
            })
            .transpose()?;
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
            in_worktree: wt.is_some(),
            // A detached worktree has no branch yet — the agent names it.
            worktree_needs_branch: wt.is_some() && worktree_on_head,
            suggest_worktree: global.suggest_worktree && wt.is_none(),
        })?;
        let spec = ProcessSpec {
            name,
            command: plan.command,
            cwd: wt.as_ref().map(|w| w.path.clone()).unwrap_or(root.clone()),
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
                drop(inner);
                // Best-effort: don't leak the just-created worktree.
                if let Some(w) = &wt {
                    let _ = crate::worktree::remove(&root, &w.name, true);
                }
                return Err(err);
            }
            insert_process(&mut inner, process_id, project_id, spec, false);
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
    /// context.
    pub fn create_worktree(&self, project_id: ProjectId, name: &str) -> CoreResult<WorktreeInfo> {
        let root = self.project_root(project_id)?;
        crate::worktree::create(&root, name, false)
    }

    /// Remove a Podium-managed git worktree. Refused while an active process
    /// runs in it, or while it has uncommitted changes unless `force`; its
    /// `podium/<name>` branch is kept. Shells out to git — call from a
    /// blocking-friendly context.
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
        crate::worktree::remove(&root, name, force)
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
    /// grace period) and cancel all pending supervised restarts.
    pub async fn shutdown(&self) {
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
        assert!(added.title.ends_with("Scratchpad"));

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
}
