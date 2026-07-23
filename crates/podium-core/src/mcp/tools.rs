//! The 25 MCP tools Podium exposes to agents — all thin calls into
//! [`Orchestrator`], returning JSON (or plain text for output tails).

use std::str::FromStr;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::CoreError;
use crate::ids::{ProcessId, ProjectId, ScratchpadId, TodoId};
use crate::orchestrator::Orchestrator;

/// Default number of trailing lines returned by `get_process_output`.
const DEFAULT_OUTPUT_LINES: usize = 100;
/// Hard cap on requested lines, to bound tool-result size.
const MAX_OUTPUT_LINES: usize = 2000;
/// Raw bytes fetched per requested line (before ANSI stripping).
const BYTES_PER_LINE: usize = 512;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListProcessesParams {
    /// Project UUID to filter by; omit to list processes of all projects.
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProcessParams {
    /// Process UUID (from `list_processes`).
    pub process_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetProcessOutputParams {
    /// Process UUID (from `list_processes`).
    pub process_id: String,
    /// Trailing lines to return (default 100, max 2000).
    pub lines: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTodosParams {
    /// Project UUID whose to-dos to list.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTodoParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// The to-do text (must not be blank).
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompleteTodoParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// To-do UUID (from `list_todos`).
    pub todo_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTodoParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// To-do UUID (from `list_todos`).
    pub todo_id: String,
    /// New task text (omit to leave unchanged; must not be blank).
    pub text: Option<String>,
    /// New description (omit to leave unchanged; blank clears it).
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssignTodoParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// To-do UUID (from `list_todos`) to take ownership of.
    pub todo_id: String,
    /// Your own agent process UUID — the value of the `PODIUM_PROCESS_ID`
    /// environment variable set for you at launch.
    pub process_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameSessionParams {
    /// Your own agent process UUID — the value of the `PODIUM_PROCESS_ID`
    /// environment variable set for you at launch.
    pub process_id: String,
    /// The new session name (must not be blank). Keep it short and
    /// descriptive of what the session is about.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentTodoParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// To-do UUID (from `list_todos`).
    pub todo_id: String,
    /// The progress note to append (must not be blank). Keep it a short
    /// overview of what was done — no code or diffs.
    pub text: String,
    /// Who is leaving the note (defaults to `agent`).
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTodoLinkParams {
    /// Project UUID the to-do belongs to.
    pub project_id: String,
    /// To-do UUID (from `list_todos`).
    pub todo_id: String,
    /// The http(s) URL of the issue or MR/PR.
    pub url: String,
    /// Human-readable label (e.g. `"#42 Fix login"`); defaults to the url.
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpawnAgentParams {
    /// Project UUID the agent should run in.
    pub project_id: String,
    /// Initial prompt for the agent (optional).
    pub prompt: Option<String>,
    /// Display name; defaults to a free `<binary>`, `<binary>-2`, … name.
    pub name: Option<String>,
    /// Adapter id (e.g. `claude-code`); defaults to the project's config.
    pub adapter_id: Option<String>,
    /// To-do UUID to work on; seeds the agent's prompt with the to-do and
    /// instructions to keep it current over MCP. Prefer `todo_ids` for one or
    /// more to-dos; this single-id field is kept for compatibility and is
    /// merged with `todo_ids` (deduplicated).
    pub todo_id: Option<String>,
    /// To-do UUIDs to work on; several are handed to the one agent as a single
    /// combined task. Seeds the prompt with each to-do plus the standing
    /// instructions to keep them current over MCP.
    pub todo_ids: Option<Vec<String>>,
    /// Scratchpad UUID to work on; seeds the agent's prompt with the
    /// scratchpad and instructions to keep it current over MCP. Only used
    /// when no to-do is given. Prefer `scratchpad_ids` for one or more
    /// scratchpads; this single-id field is kept for compatibility and is
    /// merged with `scratchpad_ids` (deduplicated).
    pub scratchpad_id: Option<String>,
    /// Scratchpad UUIDs to work on; several are handed to the one agent as a
    /// single combined task. Only used when no to-do is given.
    pub scratchpad_ids: Option<Vec<String>>,
    /// Run the agent in a fresh git worktree under `.podium/worktrees/`
    /// (requires the project to be a git repository). Named after the agent
    /// unless `worktree_name` is given.
    pub worktree: Option<bool>,
    /// Explicit worktree/branch name (slugified and de-duplicated). Defaults
    /// to the agent's window name. Ignored unless `worktree` is true.
    pub worktree_name: Option<String>,
    /// Check the worktree out on a detached HEAD so the agent creates and
    /// names its own branch, instead of a fresh `podium/<name>` branch.
    /// Ignored unless `worktree` is true.
    pub worktree_on_head: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateWorktreeParams {
    /// Project UUID to create the worktree in.
    pub project_id: String,
    /// Short name for the work (slugified and de-duplicated automatically).
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorktreesParams {
    /// Project UUID whose worktrees to list.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveWorktreeParams {
    /// Project UUID the worktree belongs to.
    pub project_id: String,
    /// Worktree name (from `list_worktrees`).
    pub name: String,
    /// Discard uncommitted changes in the worktree (default false).
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListScratchpadsParams {
    /// Project UUID whose scratchpads to list.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateScratchpadParams {
    /// Project UUID the scratchpad belongs to.
    pub project_id: String,
    /// Who is creating the scratchpad (defaults to `agent`).
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateScratchpadParams {
    /// Project UUID the scratchpad belongs to.
    pub project_id: String,
    /// Scratchpad UUID (from `list_scratchpads`).
    pub id: String,
    /// The new full content of the scratchpad.
    pub content: String,
    /// The scratchpad's `updatedAt` from your last `list_scratchpads` /
    /// `create_scratchpad` / `update_scratchpad` call (RFC 3339). Must match
    /// the current value or the update is rejected as a conflict — someone
    /// else (the user or another agent) edited it first. On conflict,
    /// re-fetch with `list_scratchpads` and retry with the fresh timestamp.
    pub expected_updated_at: String,
    /// Who is making the edit (defaults to `agent`).
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScratchpadTagParams {
    /// Project UUID the scratchpad belongs to.
    pub project_id: String,
    /// Scratchpad UUID (from `list_scratchpads`).
    pub id: String,
    /// The tag text (must not be blank).
    pub tag: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetScratchpadArchivedParams {
    /// Project UUID the scratchpad belongs to.
    pub project_id: String,
    /// Scratchpad UUID (from `list_scratchpads`).
    pub id: String,
    /// `true` to archive, `false` to restore to the active list.
    pub archived: bool,
}

/// The MCP tool surface. One instance is created per HTTP session; all state
/// lives in the shared [`Orchestrator`].
#[derive(Clone)]
pub struct PodiumTools {
    orchestrator: Arc<Orchestrator>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl PodiumTools {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self {
            orchestrator,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all projects currently open in Podium.")]
    pub async fn list_projects(&self) -> Result<CallToolResult, McpError> {
        json_result(&self.orchestrator.list_projects())
    }

    #[tool(
        description = "List managed processes (dev servers, terminals, agents), optionally filtered to one project."
    )]
    pub async fn list_processes(
        &self,
        Parameters(p): Parameters<ListProcessesParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = p.project_id.as_deref().map(parse_project_id).transpose()?;
        json_result(&self.orchestrator.list_processes(project_id))
    }

    #[tool(description = "Get the current status snapshot of one process.")]
    pub async fn get_process_status(
        &self,
        Parameters(p): Parameters<ProcessParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_process_id(&p.process_id)?;
        json_result(&self.find_process(id)?)
    }

    #[tool(
        description = "Read the last N lines of a process's terminal output, with ANSI escape sequences stripped (default 100 lines)."
    )]
    pub async fn get_process_output(
        &self,
        Parameters(p): Parameters<GetProcessOutputParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_process_id(&p.process_id)?;
        let lines =
            (p.lines.unwrap_or(DEFAULT_OUTPUT_LINES as u32) as usize).clamp(1, MAX_OUTPUT_LINES);
        let raw = self
            .orchestrator
            .tail_text(id, lines * BYTES_PER_LINE)
            .await
            .map_err(core_error)?;
        Ok(text_result(tail_lines(&strip_ansi(&raw), lines)))
    }

    #[tool(
        description = "Spawn a new AI coding agent in a project (max 8 running agents per project). Pass todo_ids (or the single todo_id) to have it work on one or more to-dos and keep them updated; several to-dos are handed over as one combined task. Pass scratchpad_ids (or the single scratchpad_id) to have it work on one or more scratchpads instead (ignored if a to-do is also given). Pass worktree=true to run it in a fresh git worktree (requires a git repository), optionally with worktree_name to name it (defaults to the agent's name) and worktree_on_head=true to check it out on a detached HEAD so the agent names its own branch. Returns the new process snapshot."
    )]
    pub async fn spawn_agent(
        &self,
        Parameters(p): Parameters<SpawnAgentParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        // Merge each single-id compatibility field with its plural, preserving
        // order and dropping duplicates.
        let mut todo_ids = Vec::new();
        for raw in p.todo_id.iter().chain(p.todo_ids.iter().flatten()) {
            let id = parse_todo_id(raw)?;
            if !todo_ids.contains(&id) {
                todo_ids.push(id);
            }
        }
        let mut scratchpad_ids = Vec::new();
        for raw in p
            .scratchpad_id
            .iter()
            .chain(p.scratchpad_ids.iter().flatten())
        {
            let id = parse_scratchpad_id(raw)?;
            if !scratchpad_ids.contains(&id) {
                scratchpad_ids.push(id);
            }
        }
        let id = self
            .orchestrator
            .spawn_agent(
                project_id,
                p.adapter_id,
                p.name,
                p.prompt,
                todo_ids,
                scratchpad_ids,
                p.worktree.unwrap_or(false),
                p.worktree_name,
                p.worktree_on_head.unwrap_or(false),
                None,
            )
            .await
            .map_err(core_error)?;
        json_result(&self.find_process(id)?)
    }

    #[tool(
        description = "Create a git worktree for isolated code changes, at `<project_root>/.podium/worktrees/<name>` on a new `podium/<name>` branch. The name is slugified and de-duplicated automatically. Call this when the user has agreed to do the work in a worktree, then make all file changes under the returned path."
    )]
    pub async fn create_worktree(
        &self,
        Parameters(p): Parameters<CreateWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let orch = Arc::clone(&self.orchestrator);
        let info = tokio::task::spawn_blocking(move || orch.create_worktree(project_id, &p.name))
            .await
            .map_err(|e| McpError::internal_error(format!("worktree task failed: {e}"), None))?
            .map_err(core_error)?;
        json_result(&info)
    }

    #[tool(
        description = "List the project's Podium-managed git worktrees (name, path, branch, inUse)."
    )]
    pub async fn list_worktrees(
        &self,
        Parameters(p): Parameters<ListWorktreesParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let orch = Arc::clone(&self.orchestrator);
        let infos = tokio::task::spawn_blocking(move || orch.list_worktrees(project_id))
            .await
            .map_err(|e| McpError::internal_error(format!("worktree task failed: {e}"), None))?
            .map_err(core_error)?;
        json_result(&infos)
    }

    #[tool(
        description = "Remove a Podium-managed git worktree. Refused while it has uncommitted changes (pass force=true to discard them) or while a Podium-managed process is still running in it. Its `podium/<name>` branch is kept."
    )]
    pub async fn remove_worktree(
        &self,
        Parameters(p): Parameters<RemoveWorktreeParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let force = p.force.unwrap_or(false);
        let orch = Arc::clone(&self.orchestrator);
        tokio::task::spawn_blocking(move || orch.remove_worktree(project_id, &p.name, force))
            .await
            .map_err(|e| McpError::internal_error(format!("worktree task failed: {e}"), None))?
            .map_err(core_error)?;
        Ok(text_result("removed".to_string()))
    }

    #[tool(description = "Start a stopped or not-yet-started process.")]
    pub async fn start_process(
        &self,
        Parameters(p): Parameters<ProcessParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_process_id(&p.process_id)?;
        self.orchestrator
            .start_process(id)
            .await
            .map_err(core_error)?;
        json_result(&self.find_process(id)?)
    }

    #[tool(
        description = "Gracefully stop a running process (SIGTERM, then SIGKILL after a grace period)."
    )]
    pub async fn stop_process(
        &self,
        Parameters(p): Parameters<ProcessParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_process_id(&p.process_id)?;
        self.orchestrator
            .stop_process(id)
            .await
            .map_err(core_error)?;
        json_result(&self.find_process(id)?)
    }

    #[tool(description = "Restart a process: stop it if running, wait for exit, start again.")]
    pub async fn restart_process(
        &self,
        Parameters(p): Parameters<ProcessParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = parse_process_id(&p.process_id)?;
        self.orchestrator
            .restart_process(id)
            .await
            .map_err(core_error)?;
        json_result(&self.find_process(id)?)
    }

    #[tool(description = "List a project's to-do items (id, text, done, createdAt).")]
    pub async fn list_todos(
        &self,
        Parameters(p): Parameters<ListTodosParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        json_result(
            &self
                .orchestrator
                .list_todos(project_id)
                .map_err(core_error)?,
        )
    }

    #[tool(description = "Add a to-do item to a project's shared to-do list.")]
    pub async fn add_todo(
        &self,
        Parameters(p): Parameters<AddTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        json_result(
            &self
                .orchestrator
                .add_todo(project_id, &p.text)
                .map_err(core_error)?,
        )
    }

    #[tool(description = "Mark a to-do item as done. Returns the updated to-do.")]
    pub async fn complete_todo(
        &self,
        Parameters(p): Parameters<CompleteTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let todo_id = parse_todo_id(&p.todo_id)?;
        json_result(
            &self
                .orchestrator
                .set_todo_done(project_id, todo_id, true)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Claim a to-do as the one you (a running agent) are working on, so the user sees it under your agent in Podium. Pass your own process_id from the PODIUM_PROCESS_ID environment variable. Returns the updated to-do."
    )]
    pub async fn assign_todo(
        &self,
        Parameters(p): Parameters<AssignTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let todo_id = parse_todo_id(&p.todo_id)?;
        let process_id = parse_process_id(&p.process_id)?;
        json_result(
            &self
                .orchestrator
                .assign_todo(project_id, todo_id, process_id)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Rename your own session (agent process) to reflect what it is about, so the user can tell your sessions apart in Podium. Pick a short, descriptive name yourself. If you were started standalone (not handed a to-do at launch), do this right after the user's first prompt. Pass your own process_id from the PODIUM_PROCESS_ID environment variable. Returns the updated process snapshot."
    )]
    pub async fn rename_session(
        &self,
        Parameters(p): Parameters<RenameSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        let process_id = parse_process_id(&p.process_id)?;
        json_result(
            &self
                .orchestrator
                .rename_process(process_id, &p.name)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Revise a to-do's text and/or description (e.g. as scope changes). Returns the updated to-do."
    )]
    pub async fn update_todo(
        &self,
        Parameters(p): Parameters<UpdateTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let todo_id = parse_todo_id(&p.todo_id)?;
        json_result(
            &self
                .orchestrator
                .update_todo(
                    project_id,
                    todo_id,
                    p.text.as_deref(),
                    p.description.as_deref(),
                )
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Append a short progress note to a to-do so the user and other agents can track what was done. Keep it small: a brief overview of what you did (a sentence or two), not a full write-up — no code or diffs. Returns the updated to-do."
    )]
    pub async fn comment_todo(
        &self,
        Parameters(p): Parameters<CommentTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let todo_id = parse_todo_id(&p.todo_id)?;
        let author = p.author.as_deref().unwrap_or("agent");
        json_result(
            &self
                .orchestrator
                .comment_todo(project_id, todo_id, author, &p.text)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Pin an issue/PR link to the top of a to-do. Call this when you open a GitLab issue or MR/PR while working on the to-do, so the user can jump straight to it. The url must be http(s). Returns the updated to-do."
    )]
    pub async fn add_todo_link(
        &self,
        Parameters(p): Parameters<AddTodoLinkParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let todo_id = parse_todo_id(&p.todo_id)?;
        let label = p.label.as_deref().unwrap_or("");
        json_result(
            &self
                .orchestrator
                .add_todo_link(project_id, todo_id, label, &p.url)
                .map_err(core_error)?,
        )
    }

    #[tool(description = "List a project's active (non-archived) scratchpads.")]
    pub async fn list_scratchpads(
        &self,
        Parameters(p): Parameters<ListScratchpadsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        json_result(
            &self
                .orchestrator
                .list_scratchpads(project_id)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Create a new scratchpad in a project (auto-generated timestamp title, empty content). Returns the created scratchpad."
    )]
    pub async fn create_scratchpad(
        &self,
        Parameters(p): Parameters<CreateScratchpadParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let author = p.author.as_deref().unwrap_or("agent");
        json_result(
            &self
                .orchestrator
                .add_scratchpad(project_id, author)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Replace a scratchpad's content (bumps its version). Requires expected_updated_at (from your last read of the scratchpad) — if someone else edited it since, this fails as a conflict; re-fetch with list_scratchpads and retry. Returns the updated scratchpad."
    )]
    pub async fn update_scratchpad(
        &self,
        Parameters(p): Parameters<UpdateScratchpadParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let id = parse_scratchpad_id(&p.id)?;
        let expected_updated_at = parse_timestamp(&p.expected_updated_at)?;
        let author = p.author.as_deref().unwrap_or("agent");
        json_result(
            &self
                .orchestrator
                .update_scratchpad_content(project_id, id, &p.content, expected_updated_at, author)
                .map_err(core_error)?,
        )
    }

    #[tool(description = "Add a free-text tag to a scratchpad. Returns the updated scratchpad.")]
    pub async fn add_scratchpad_tag(
        &self,
        Parameters(p): Parameters<ScratchpadTagParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let id = parse_scratchpad_id(&p.id)?;
        json_result(
            &self
                .orchestrator
                .add_scratchpad_tag(project_id, id, &p.tag)
                .map_err(core_error)?,
        )
    }

    #[tool(
        description = "Remove a tag from a scratchpad (by exact value). Returns the updated scratchpad."
    )]
    pub async fn remove_scratchpad_tag(
        &self,
        Parameters(p): Parameters<ScratchpadTagParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let id = parse_scratchpad_id(&p.id)?;
        json_result(
            &self
                .orchestrator
                .remove_scratchpad_tag(project_id, id, &p.tag)
                .map_err(core_error)?,
        )
    }

    #[tool(description = "Archive or restore a scratchpad. Returns the updated scratchpad.")]
    pub async fn set_scratchpad_archived(
        &self,
        Parameters(p): Parameters<SetScratchpadArchivedParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        let id = parse_scratchpad_id(&p.id)?;
        json_result(
            &self
                .orchestrator
                .set_scratchpad_archived(project_id, id, p.archived)
                .map_err(core_error)?,
        )
    }

    fn find_process(&self, id: ProcessId) -> Result<crate::process::ProcessInfo, McpError> {
        self.orchestrator
            .list_processes(None)
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| core_error(CoreError::ProcessNotFound))
    }
}

// rmcp 1.8's default router expression is `Self::tool_router()` (rebuilt per
// call); point it at the field so the router is built once per instance.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for PodiumTools {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` is `#[non_exhaustive]`, so it cannot be built with a
        // struct expression — mutate a default instance instead.
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Podium's control surface. Podium is an agent-orchestration \
             workspace: projects contain managed processes (dev servers, \
             terminals, AI agents). Use list_projects/list_processes to \
             discover ids, get_process_output to read terminal output, \
             and spawn_agent to launch sibling agents (pass todo_id to put \
             an agent on a specific to-do). Each project has a shared to-do \
             list (list_todos/add_todo/complete_todo) visible to the user \
             and every agent. When you are working on a to-do, keep it \
             current: comment_todo with a short overview of what you did as \
             you progress (small notes, not full write-ups, no code), \
             update_todo if the text/description needs revising, and \
             complete_todo when it is finished. When you open a GitLab issue \
             or MR/PR while working on a to-do, call add_todo_link so its URL \
             is pinned to the top of the to-do for the user. If you pick up a \
             to-do that was not handed to you at launch, call assign_todo with \
             your own PODIUM_PROCESS_ID so the user can see you own it. Keep your \
             session recognisable: call rename_session (with your own \
             PODIUM_PROCESS_ID) to give yourself a short name describing what the \
             session is about — if you were started standalone rather than on a \
             to-do, do this right after the user's first prompt. Each project \
             also has shared scratchpads for freeform notes \
             (list_scratchpads/create_scratchpad/update_scratchpad), visible \
             to the user and every agent. For isolated code changes, \
             create_worktree/list_worktrees/remove_worktree manage git \
             worktrees under .podium/worktrees/ so an agent's edits don't \
             touch the user's working tree."
                .to_string(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(format!("serialize result: {e}"), None))?;
    Ok(text_result(json))
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

fn parse_project_id(s: &str) -> Result<ProjectId, McpError> {
    ProjectId::from_str(s)
        .map_err(|_| McpError::invalid_params(format!("invalid project_id: {s}"), None))
}

fn parse_process_id(s: &str) -> Result<ProcessId, McpError> {
    ProcessId::from_str(s)
        .map_err(|_| McpError::invalid_params(format!("invalid process_id: {s}"), None))
}

fn parse_todo_id(s: &str) -> Result<TodoId, McpError> {
    TodoId::from_str(s).map_err(|_| McpError::invalid_params(format!("invalid todo_id: {s}"), None))
}

fn parse_scratchpad_id(s: &str) -> Result<ScratchpadId, McpError> {
    ScratchpadId::from_str(s)
        .map_err(|_| McpError::invalid_params(format!("invalid scratchpad id: {s}"), None))
}

/// Parse an RFC 3339 timestamp (e.g. a scratchpad's `updatedAt`) from a tool
/// parameter. `schemars`/`JsonSchema` doesn't cover `DateTime<Utc>` for the
/// generated tool schemas, so callers pass it as a plain string and this
/// parses it, mirroring the id-parsing helpers above.
fn parse_timestamp(s: &str) -> Result<chrono::DateTime<chrono::Utc>, McpError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|_| McpError::invalid_params(format!("invalid RFC 3339 timestamp: {s}"), None))
}

/// Map a [`CoreError`] onto MCP error codes. Every message is Podium-owned
/// text (never terminal output or secrets), so forwarding is safe.
fn core_error(e: CoreError) -> McpError {
    match e {
        CoreError::Io(_) | CoreError::Pty(_) | CoreError::Config(_) | CoreError::Git(_) => {
            McpError::internal_error(e.to_string(), None)
        }
        _ => McpError::invalid_params(e.to_string(), None),
    }
}

fn strip_ansi(raw: &str) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(raw.as_bytes())).into_owned()
}

/// Last `n` lines of `text` (like `tail -n`).
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_removes_color_codes() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m plain"), "red plain");
    }

    #[test]
    fn tail_lines_returns_last_n() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("a\nb", 10), "a\nb");
        assert_eq!(tail_lines("", 3), "");
    }
}
