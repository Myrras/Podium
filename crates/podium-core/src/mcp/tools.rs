//! The 16 MCP tools Podium exposes to agents — all thin calls into
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
use crate::ids::{ProcessId, ProjectId, TodoId};
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
        description = "Spawn a new AI coding agent in a project (max 8 running agents per project). Pass todo_ids (or the single todo_id) to have it work on one or more to-dos and keep them updated; several to-dos are handed over as one combined task. Returns the new process snapshot."
    )]
    pub async fn spawn_agent(
        &self,
        Parameters(p): Parameters<SpawnAgentParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&p.project_id)?;
        // Merge the single-id compatibility field with `todo_ids`, preserving
        // order and dropping duplicates.
        let mut todo_ids = Vec::new();
        for raw in p.todo_id.iter().chain(p.todo_ids.iter().flatten()) {
            let id = parse_todo_id(raw)?;
            if !todo_ids.contains(&id) {
                todo_ids.push(id);
            }
        }
        let id = self
            .orchestrator
            .spawn_agent(project_id, p.adapter_id, p.name, p.prompt, todo_ids)
            .await
            .map_err(core_error)?;
        json_result(&self.find_process(id)?)
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
             to-do, do this right after the user's first prompt."
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

/// Map a [`CoreError`] onto MCP error codes. Every message is Podium-owned
/// text (never terminal output or secrets), so forwarding is safe.
fn core_error(e: CoreError) -> McpError {
    match e {
        CoreError::Io(_) | CoreError::Pty(_) | CoreError::Config(_) => {
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
