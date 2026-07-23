//! Integration tests for the built-in MCP server (phase 6): lifecycle,
//! bearer auth, the tool surface, and the agent recursion guard.

use std::sync::Arc;
use std::time::Duration;

use podium_core::mcp::tools::{
    AddTodoLinkParams, AddTodoParams, AssignTodoParams, CommentTodoParams, CreateScratchpadParams,
    CreateWorktreeParams, GetProcessOutputParams, ListScratchpadsParams, ListTodosParams,
    ListWorktreesParams, PodiumTools, RemoveWorktreeParams, RenameSessionParams,
    ScratchpadTagParams, SetScratchpadArchivedParams, SpawnAgentParams, UpdateScratchpadParams,
    UpdateTodoParams,
};
use podium_core::mcp::{self, McpServer};
use podium_core::{
    AdapterRegistry, AgentAdapter, AgentLaunchCtx, CoreError, CoreResult, LaunchPlan, Orchestrator,
    ProcessId, ProcessKind, ProcessSpec, ProcessStatus, ProjectId, RestartPolicy, TodoId,
};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"podium-test","version":"0.0.0"}}}"#;

async fn start_server(config_dir: std::path::PathBuf) -> (Arc<Orchestrator>, McpServer) {
    let orch = Arc::new(Orchestrator::new());
    let server = mcp::start(Arc::clone(&orch), config_dir)
        .await
        .expect("mcp server starts");
    (orch, server)
}

/// One raw HTTP/1.1 POST to `<base_url>/mcp`; returns (status, full response).
async fn http_post_mcp(base_url: &str, token: Option<&str>, body: &str) -> (u16, String) {
    http_post_mcp_with_session(base_url, token, None, body).await
}

/// Like [`http_post_mcp`], but able to carry an `Mcp-Session-Id` header (the
/// streamable-HTTP transport ties `tools/list`/`tools/call` to the session
/// opened by `initialize`).
async fn http_post_mcp_with_session(
    base_url: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    body: &str,
) -> (u16, String) {
    let addr = base_url.strip_prefix("http://").expect("http url");
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let session = session_id
        .map(|s| format!("Mcp-Session-Id: {s}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\n{auth}{session}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = Vec::new();
    timeout(TEST_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("timely response")
        .expect("read response");
    let text = String::from_utf8_lossy(&response).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    (status, text)
}

/// Pull the `Mcp-Session-Id` response header out of a raw HTTP response
/// (case-insensitive header name, as HTTP requires).
fn extract_session_id(response: &str) -> String {
    response
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("mcp-session-id") {
                Some(value.trim().to_string())
            } else {
                None
            }
        })
        .expect("Mcp-Session-Id response header present")
}

/// The JSON-RPC payload of a raw HTTP response. The server answers either
/// `application/json` (a plain body) or `text/event-stream` (one `data: `
/// line per SSE event, chunked-encoded) — either way, the payload we want is
/// the first `{...}` JSON object in the response.
fn extract_json_body(response: &str) -> serde_json::Value {
    let start = response.find('{').expect("json object in body");
    let mut depth = 0usize;
    let mut end = start;
    for (i, c) in response[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    serde_json::from_str(&response[start..end]).expect("valid json body")
}

/// Text of the first content block of a tool result (shape-agnostic).
fn first_text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).expect("serialize tool result");
    value["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

async fn wait_for_exit(orch: &Orchestrator, id: ProcessId) {
    timeout(TEST_TIMEOUT, async {
        loop {
            let status = orch
                .list_processes(None)
                .into_iter()
                .find(|p| p.id == id)
                .expect("process listed")
                .status;
            if matches!(status, ProcessStatus::Exited { .. }) {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for process to exit");
}

#[tokio::test(flavor = "multi_thread")]
async fn starts_on_ephemeral_localhost_port_and_wipes_config_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_dir = dir.path().join("mcp");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(config_dir.join("agent-stale.json"), "{}").expect("write stale file");

    let (_orch, server) = start_server(config_dir.clone()).await;

    assert!(server.url().starts_with("http://127.0.0.1:"));
    assert_eq!(server.token().len(), 64, "64-char hex token");
    assert!(server.token().chars().all(|c| c.is_ascii_hexdigit()));
    assert!(config_dir.is_dir(), "config dir recreated");
    assert!(
        !config_dir.join("agent-stale.json").exists(),
        "stale configs from previous runs are wiped"
    );

    // The stdio bridge discovers the server through this file.
    let connect_file = config_dir.join("server.json");
    let connect: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&connect_file).expect("server.json written"))
            .expect("valid json");
    assert_eq!(connect["url"], server.url());
    assert_eq!(connect["token"], server.token());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&connect_file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "server.json must be private");
    }
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_requests_without_or_with_wrong_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_orch, server) = start_server(dir.path().join("mcp")).await;

    let (status, _) = http_post_mcp(server.url(), None, INITIALIZE).await;
    assert_eq!(status, 401, "missing token is rejected");

    let (status, _) = http_post_mcp(server.url(), Some("wrong-token"), INITIALIZE).await;
    assert_eq!(status, 401, "wrong token is rejected");
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_succeeds_with_the_bearer_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_orch, server) = start_server(dir.path().join("mcp")).await;

    let (status, response) = http_post_mcp(server.url(), Some(server.token()), INITIALIZE).await;
    assert_eq!(status, 200);
    assert!(
        response.contains("protocolVersion"),
        "initialize result returned: {response}"
    );
    server.stop();
}

const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

fn tools_list_request(id: u64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#)
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_list_includes_scratchpad_tools() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_orch, server) = start_server(dir.path().join("mcp")).await;

    let (status, init_response) =
        http_post_mcp(server.url(), Some(server.token()), INITIALIZE).await;
    assert_eq!(status, 200);
    let session_id = extract_session_id(&init_response);

    let (status, _) = http_post_mcp_with_session(
        server.url(),
        Some(server.token()),
        Some(&session_id),
        INITIALIZED,
    )
    .await;
    assert_eq!(status, 202, "initialized notification accepted");

    let (status, list_response) = http_post_mcp_with_session(
        server.url(),
        Some(server.token()),
        Some(&session_id),
        &tools_list_request(2),
    )
    .await;
    assert_eq!(status, 200);
    let body = extract_json_body(&list_response);
    let tools = body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect::<Vec<_>>();
    for name in [
        "list_scratchpads",
        "create_scratchpad",
        "update_scratchpad",
        "add_scratchpad_tag",
        "remove_scratchpad_tag",
        "set_scratchpad_archived",
        "create_worktree",
        "list_worktrees",
        "remove_worktree",
    ] {
        assert!(
            tools.contains(&name),
            "{name} missing from tools/list: {tools:?}"
        );
    }

    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_list_projects_and_read_ansi_stripped_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let projects = tools.list_projects().await.expect("list_projects");
    assert!(
        first_text(&projects).contains(&project_id.to_string()),
        "list_projects returns the open project"
    );

    let pid = orch
        .add_process(
            project_id,
            ProcessSpec {
                name: "colors".to_string(),
                command: r"printf '\033[31mred\033[0m plain\n'".to_string(),
                cwd: dir.path().to_path_buf(),
                env: Vec::new(),
                kind: ProcessKind::Service,
                restart_policy: RestartPolicy::Never,
                color: None,
            },
        )
        .await
        .expect("add process");
    orch.start_process(pid).await.expect("start process");
    wait_for_exit(&orch, pid).await;

    let output = tools
        .get_process_output(Parameters(GetProcessOutputParams {
            process_id: pid.to_string(),
            lines: None,
        }))
        .await
        .expect("get_process_output");
    let text = first_text(&output);
    assert!(text.contains("red"), "output captured: {text:?}");
    assert!(!text.contains('\u{1b}'), "ANSI escapes stripped: {text:?}");

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn todo_tools_update_and_comment_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let added = tools
        .add_todo(Parameters(AddTodoParams {
            project_id: project_id.to_string(),
            text: "wire up auth".to_string(),
        }))
        .await
        .expect("add_todo");
    let todo_id = serde_json::from_str::<serde_json::Value>(&first_text(&added))
        .expect("todo json")["id"]
        .as_str()
        .expect("todo id")
        .to_string();

    let updated = tools
        .update_todo(Parameters(UpdateTodoParams {
            project_id: project_id.to_string(),
            todo_id: todo_id.clone(),
            text: Some("wire up OAuth".to_string()),
            description: Some("use the shared provider".to_string()),
        }))
        .await
        .expect("update_todo");
    let updated_text = first_text(&updated);
    assert!(updated_text.contains("wire up OAuth"), "{updated_text:?}");
    assert!(
        updated_text.contains("use the shared provider"),
        "{updated_text:?}"
    );

    tools
        .comment_todo(Parameters(CommentTodoParams {
            project_id: project_id.to_string(),
            todo_id: todo_id.clone(),
            text: "handler stubbed out".to_string(),
            author: Some("claude".to_string()),
        }))
        .await
        .expect("comment_todo");

    let linked = tools
        .add_todo_link(Parameters(AddTodoLinkParams {
            project_id: project_id.to_string(),
            todo_id: todo_id.clone(),
            url: "https://gitlab.example.com/acme/web/-/issues/42".to_string(),
            label: Some("#42 Fix login".to_string()),
        }))
        .await
        .expect("add_todo_link");
    let linked_text = first_text(&linked);
    assert!(linked_text.contains("#42 Fix login"), "{linked_text:?}");

    // A non-http(s) url is rejected.
    let bad = tools
        .add_todo_link(Parameters(AddTodoLinkParams {
            project_id: project_id.to_string(),
            todo_id: todo_id.clone(),
            url: "ftp://nope".to_string(),
            label: None,
        }))
        .await;
    assert!(bad.is_err(), "non-http url must be rejected");

    // list_todos must reflect the revised text/description, comment, and link.
    let listed_result = tools
        .list_todos(Parameters(ListTodosParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list_todos");
    let listed = first_text(&listed_result);
    assert!(listed.contains("wire up OAuth"), "{listed:?}");
    assert!(listed.contains("handler stubbed out"), "{listed:?}");
    assert!(listed.contains("claude"), "{listed:?}");
    assert!(listed.contains("#42 Fix login"), "{listed:?}");

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_scratchpad_round_trip_via_mcp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let created = tools
        .create_scratchpad(Parameters(CreateScratchpadParams {
            project_id: project_id.to_string(),
            author: None,
        }))
        .await
        .expect("create_scratchpad");
    let created_json: serde_json::Value =
        serde_json::from_str(&first_text(&created)).expect("scratchpad json");
    assert_eq!(created_json["projectId"], project_id.to_string());
    assert_eq!(created_json["content"], "");
    let scratchpad_id = created_json["id"]
        .as_str()
        .expect("scratchpad id")
        .to_string();
    let created_updated_at = created_json["updatedAt"]
        .as_str()
        .expect("updatedAt")
        .to_string();

    let updated = tools
        .update_scratchpad(Parameters(UpdateScratchpadParams {
            project_id: project_id.to_string(),
            id: scratchpad_id.clone(),
            content: "meeting notes".to_string(),
            expected_updated_at: created_updated_at.clone(),
            author: None,
        }))
        .await
        .expect("update_scratchpad");
    let updated_text = first_text(&updated);
    assert!(updated_text.contains("meeting notes"), "{updated_text:?}");
    assert!(
        updated_text.contains("\"updatedBy\": \"agent\""),
        "{updated_text:?}"
    );
    let updated_json: serde_json::Value = serde_json::from_str(&updated_text).unwrap();
    let updated_updated_at = updated_json["updatedAt"].as_str().unwrap().to_string();

    // A stale expected_updated_at (the original create timestamp, since
    // superseded) is rejected as a conflict instead of clobbering the edit.
    let stale = tools
        .update_scratchpad(Parameters(UpdateScratchpadParams {
            project_id: project_id.to_string(),
            id: scratchpad_id.clone(),
            content: "stale content".to_string(),
            expected_updated_at: created_updated_at,
            author: None,
        }))
        .await;
    assert!(stale.is_err(), "stale timestamp must be rejected");

    // An explicit author overrides the "agent" default; the fresh timestamp
    // from the prior update succeeds.
    let renamed_author = tools
        .update_scratchpad(Parameters(UpdateScratchpadParams {
            project_id: project_id.to_string(),
            id: scratchpad_id.clone(),
            content: "meeting notes v2".to_string(),
            expected_updated_at: updated_updated_at,
            author: Some("claude-code".to_string()),
        }))
        .await
        .expect("update_scratchpad with explicit author");
    let renamed_text = first_text(&renamed_author);
    assert!(
        renamed_text.contains("\"updatedBy\": \"claude-code\""),
        "{renamed_text:?}"
    );

    // Tags and archiving round-trip too.
    let tagged = tools
        .add_scratchpad_tag(Parameters(ScratchpadTagParams {
            project_id: project_id.to_string(),
            id: scratchpad_id.clone(),
            tag: "meeting".to_string(),
        }))
        .await
        .expect("add_scratchpad_tag");
    assert!(first_text(&tagged).contains("meeting"));

    let untagged = tools
        .remove_scratchpad_tag(Parameters(ScratchpadTagParams {
            project_id: project_id.to_string(),
            id: scratchpad_id.clone(),
            tag: "meeting".to_string(),
        }))
        .await
        .expect("remove_scratchpad_tag");
    assert!(!first_text(&untagged).contains("\"meeting\""));

    let archived = tools
        .set_scratchpad_archived(Parameters(SetScratchpadArchivedParams {
            project_id: project_id.to_string(),
            id: scratchpad_id,
            archived: true,
        }))
        .await
        .expect("set_scratchpad_archived");
    assert!(first_text(&archived).contains("\"archived\": true"));

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_scratchpads_round_trip_via_mcp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    tools
        .create_scratchpad(Parameters(CreateScratchpadParams {
            project_id: project_id.to_string(),
            author: None,
        }))
        .await
        .expect("create_scratchpad");

    let listed = tools
        .list_scratchpads(Parameters(ListScratchpadsParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list_scratchpads");
    let listed_json: serde_json::Value =
        serde_json::from_str(&first_text(&listed)).expect("scratchpad list json");
    let items = listed_json.as_array().expect("array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["projectId"], project_id.to_string());

    orch.shutdown().await;
}

/// Adapter whose "agent" is just a long sleep, for cap testing.
struct FakeAgentAdapter;

impl AgentAdapter for FakeAgentAdapter {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn display_name(&self) -> &'static str {
        "Fake"
    }

    fn binary(&self) -> &'static str {
        "fake-agent"
    }

    fn build_launch(&self, _ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
        Ok(LaunchPlan {
            command: "sleep 30".to_string(),
            env: Vec::new(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_is_capped_at_eight_active_agents_per_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    for n in 1..=8 {
        orch.spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("agent {n} under the cap should spawn: {e}"));
    }

    let err = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
        .await
        .expect_err("ninth concurrent agent must be rejected");
    assert!(
        matches!(err, CoreError::AgentLimitReached),
        "unexpected error: {err}"
    );

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_spawned_from_todo_takes_the_todo_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");

    // No explicit name: the window inherits the to-do's text.
    let first = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn agent for todo");
    // A second agent on the same to-do is deduplicated, not renamed generically.
    let second = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn second agent for todo");

    let procs = orch.list_processes(Some(project_id));
    let name_of = |id| {
        procs
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .expect("process present")
    };
    assert_eq!(name_of(first), "wire up OAuth");
    assert_eq!(name_of(second), "wire up OAuth-2");

    // An explicit name still wins over the to-do's text.
    let named = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            Some("custom".to_string()),
            None,
            vec![todo.id],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn named agent for todo");
    assert_eq!(
        name_of_in(&orch.list_processes(Some(project_id)), named),
        "custom"
    );

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawning_on_a_todo_assigns_the_agent_and_unassign_clears_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");

    // Before any agent, the to-do has no assignment.
    let listed = orch.list_todos(project_id).expect("list todos");
    assert!(listed[0].assigned_agent.is_none());

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn agent for todo");

    // Spawning on the to-do links the agent to it, enriched in list_todos.
    let listed = orch.list_todos(project_id).expect("list todos");
    let assigned = listed[0]
        .assigned_agent
        .as_ref()
        .expect("agent assigned after spawn");
    assert_eq!(assigned.process_id, agent);
    assert_eq!(assigned.name, "wire up OAuth");
    // The session gets a colour at spawn, carried on the assignment so the UI
    // can tint the to-do to match its owner.
    assert!(assigned.color.is_some(), "assigned agent carries a colour");
    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));

    // Unassigning clears the link (and returns the enriched, now-empty to-do).
    let after = orch.unassign_todo(project_id, todo.id).expect("unassign");
    assert!(after.assigned_agent.is_none());
    assert!(orch.list_todos(project_id).expect("list")[0]
        .assigned_agent
        .is_none());
    assert_eq!(orch.agent_for_todo(todo.id), None);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_agent_clears_its_todo_assignment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");
    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn agent for todo");
    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));

    orch.remove_process(agent).await.expect("remove agent");
    assert_eq!(orch.agent_for_todo(todo.id), None);
    assert!(orch.list_todos(project_id).expect("list")[0]
        .assigned_agent
        .is_none());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn assign_todo_tool_self_assigns_a_running_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    // A bare agent (no to-do handed to it at launch) and a separate to-do.
    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn bare agent");
    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");
    assert_eq!(orch.agent_for_todo(todo.id), None);

    let assigned = tools
        .assign_todo(Parameters(AssignTodoParams {
            project_id: project_id.to_string(),
            todo_id: todo.id.to_string(),
            process_id: agent.to_string(),
        }))
        .await
        .expect("assign_todo");
    let text = first_text(&assigned);
    assert!(text.contains(&agent.to_string()), "{text:?}");
    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));

    // A non-agent process (or an unknown one) is rejected.
    let bogus = tools
        .assign_todo(Parameters(AssignTodoParams {
            project_id: project_id.to_string(),
            todo_id: todo.id.to_string(),
            process_id: ProcessId::new().to_string(),
        }))
        .await;
    assert!(bogus.is_err(), "unknown process must be rejected");

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn assign_todo_is_blocked_when_already_owned_by_another_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let spawn_bare = || {
        orch.spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
    };
    let first = spawn_bare().await.expect("spawn first agent");
    let second = spawn_bare().await.expect("spawn second agent");
    let todo = orch.add_todo(project_id, "wire up OAuth").expect("todo");
    let todo2 = orch.add_todo(project_id, "write tests").expect("todo2");

    let claim = |todo_id: TodoId, process_id: ProcessId| {
        tools.assign_todo(Parameters(AssignTodoParams {
            project_id: project_id.to_string(),
            todo_id: todo_id.to_string(),
            process_id: process_id.to_string(),
        }))
    };

    // First session claims it; re-claiming your own is idempotent (allowed).
    claim(todo.id, first).await.expect("first claim");
    claim(todo.id, first)
        .await
        .expect("re-claiming your own is idempotent");
    // A different live session cannot steal it.
    assert!(
        claim(todo.id, second).await.is_err(),
        "a second session must not steal an owned to-do"
    );
    assert_eq!(orch.agent_for_todo(todo.id), Some(first));

    // Two concurrent sessions get distinct colours (the palette avoids reusing
    // a colour a live agent already holds). Colours live on the assignment.
    claim(todo2.id, second).await.expect("second claims todo2");
    let listed = orch.list_todos(project_id).expect("list");
    let color_of = |tid: TodoId| {
        listed
            .iter()
            .find(|t| t.id == tid)
            .and_then(|t| t.assigned_agent.as_ref())
            .and_then(|a| a.color.clone())
    };
    let c1 = color_of(todo.id);
    let c2 = color_of(todo2.id);
    assert!(c1.is_some() && c2.is_some(), "both carry a colour");
    assert_ne!(c1, c2, "concurrent sessions get distinct colours");

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_session_tool_renames_the_calling_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn agent");

    let renamed = tools
        .rename_session(Parameters(RenameSessionParams {
            process_id: agent.to_string(),
            name: "  Wiring up OAuth  ".to_string(),
        }))
        .await
        .expect("rename_session");
    assert!(first_text(&renamed).contains("Wiring up OAuth"));
    let listed = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == agent)
        .expect("agent in list");
    assert_eq!(listed.name, "Wiring up OAuth", "trimmed name persists");

    // Blank names are rejected.
    assert!(tools
        .rename_session(Parameters(RenameSessionParams {
            process_id: agent.to_string(),
            name: "   ".to_string(),
        }))
        .await
        .is_err());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_tool_accepts_multiple_todo_ids_as_one_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let a = orch.add_todo(project_id, "wire up auth").expect("todo a");
    let b = orch.add_todo(project_id, "write tests").expect("todo b");

    // Both to-dos (with `todo_id` overlapping `todo_ids` to prove dedup) go to
    // one agent. With several grouped, picking one to-do's text as the name is
    // arbitrary, so the window is left generically named (the adapter binary)
    // and the agent renames itself once it has read them all.
    let spawned = tools
        .spawn_agent(Parameters(SpawnAgentParams {
            project_id: project_id.to_string(),
            prompt: None,
            name: None,
            adapter_id: Some("fake".to_string()),
            todo_id: Some(a.id.to_string()),
            todo_ids: Some(vec![a.id.to_string(), b.id.to_string()]),
            scratchpad_id: None,
            scratchpad_ids: None,
            worktree: None,
        }))
        .await
        .expect("spawn_agent over MCP");
    let name = serde_json::from_str::<serde_json::Value>(&first_text(&spawned))
        .expect("process json")["name"]
        .as_str()
        .expect("name")
        .to_string();
    assert_eq!(name, "fake-agent", "grouped to-dos don't borrow one's text");

    // Exactly one agent was created for the two to-dos.
    let agents = orch
        .list_processes(Some(project_id))
        .into_iter()
        .filter(|p| matches!(p.kind, ProcessKind::Agent { .. }))
        .count();
    assert_eq!(agents, 1);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawning_on_a_scratchpad_assigns_the_agent_and_unassign_clears_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let pad = orch
        .add_scratchpad(project_id, "User")
        .expect("add scratchpad");

    // Before any agent, the scratchpad has no assignment.
    let listed = orch.list_scratchpads(project_id).expect("list scratchpads");
    assert!(listed[0].assigned_agent.is_none());

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![pad.id],
            false,
            None,
        )
        .await
        .expect("spawn agent for scratchpad");

    // Spawning on the scratchpad links the agent to it, enriched in
    // list_scratchpads.
    let listed = orch.list_scratchpads(project_id).expect("list scratchpads");
    let assigned = listed[0]
        .assigned_agent
        .as_ref()
        .expect("agent assigned after spawn");
    assert_eq!(assigned.process_id, agent);
    assert_eq!(orch.agent_for_scratchpad(pad.id), Some(agent));

    // Unassigning clears the link (and returns the enriched, now-empty pad).
    let after = orch
        .unassign_scratchpad(project_id, pad.id)
        .expect("unassign");
    assert!(after.assigned_agent.is_none());
    assert!(orch.list_scratchpads(project_id).expect("list")[0]
        .assigned_agent
        .is_none());
    assert_eq!(orch.agent_for_scratchpad(pad.id), None);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawning_on_both_todo_and_scratchpad_ignores_the_scratchpad() {
    // to-dos win: when both are passed, the scratchpad is neither assigned
    // nor resolved (its content never entered the prompt either).
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");
    let pad = orch
        .add_scratchpad(project_id, "User")
        .expect("add scratchpad");

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![pad.id],
            false,
            None,
        )
        .await
        .expect("spawn agent for todo and scratchpad");

    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));
    assert_eq!(orch.agent_for_scratchpad(pad.id), None);
    assert!(orch.list_scratchpads(project_id).expect("list")[0]
        .assigned_agent
        .is_none());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawning_on_a_todo_with_a_bogus_scratchpad_still_succeeds() {
    // Per the "ignored if a to-do is also given" contract, an
    // invalid/deleted scratchpad id must not fail a to-do spawn.
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let todo = orch
        .add_todo(project_id, "wire up OAuth")
        .expect("add todo");
    let bogus_id = podium_core::ScratchpadId::new();

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![todo.id],
            vec![bogus_id],
            false,
            None,
        )
        .await
        .expect("spawn agent for todo, ignoring the bogus scratchpad id");

    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));
    assert_eq!(orch.agent_for_scratchpad(bogus_id), None);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_an_agent_clears_its_scratchpad_assignment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let pad = orch
        .add_scratchpad(project_id, "User")
        .expect("add scratchpad");
    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![pad.id],
            false,
            None,
        )
        .await
        .expect("spawn agent for scratchpad");
    assert_eq!(orch.agent_for_scratchpad(pad.id), Some(agent));

    orch.remove_process(agent).await.expect("remove agent");
    assert_eq!(orch.agent_for_scratchpad(pad.id), None);
    assert!(orch.list_scratchpads(project_id).expect("list")[0]
        .assigned_agent
        .is_none());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_scratchpads_enriches_assigned_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let pad = orch
        .add_scratchpad(project_id, "User")
        .expect("add scratchpad");
    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![pad.id],
            false,
            None,
        )
        .await
        .expect("spawn agent for scratchpad");

    let listed = orch.list_scratchpads(project_id).expect("list scratchpads");
    let assigned = listed
        .iter()
        .find(|s| s.id == pad.id)
        .and_then(|s| s.assigned_agent.as_ref())
        .expect("assigned agent present");
    assert_eq!(assigned.process_id, agent);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_tool_accepts_scratchpad_ids() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let a = orch.add_scratchpad(project_id, "User").expect("pad a");
    let b = orch.add_scratchpad(project_id, "User").expect("pad b");

    // Both scratchpads (with `scratchpad_id` overlapping `scratchpad_ids` to
    // prove dedup) go to one agent.
    let spawned = tools
        .spawn_agent(Parameters(SpawnAgentParams {
            project_id: project_id.to_string(),
            prompt: None,
            name: None,
            adapter_id: Some("fake".to_string()),
            todo_id: None,
            todo_ids: None,
            scratchpad_id: Some(a.id.to_string()),
            scratchpad_ids: Some(vec![a.id.to_string(), b.id.to_string()]),
            worktree: None,
        }))
        .await
        .expect("spawn_agent over MCP");
    let agent_process_id = serde_json::from_str::<serde_json::Value>(&first_text(&spawned))
        .expect("process json")["id"]
        .as_str()
        .expect("id")
        .to_string();

    // Exactly one agent was created for the two scratchpads, assigned to both.
    let agents = orch
        .list_processes(Some(project_id))
        .into_iter()
        .filter(|p| matches!(p.kind, ProcessKind::Agent { .. }))
        .count();
    assert_eq!(agents, 1);
    let listed = orch.list_scratchpads(project_id).expect("list scratchpads");
    for pad in [&a, &b] {
        let assigned = listed
            .iter()
            .find(|s| s.id == pad.id)
            .and_then(|s| s.assigned_agent.as_ref())
            .expect("assigned agent present");
        assert_eq!(assigned.process_id.to_string(), agent_process_id);
    }

    orch.shutdown().await;
}

fn name_of_in(procs: &[podium_core::ProcessInfo], id: ProcessId) -> String {
    procs
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone())
        .expect("process present")
}

/// Adapter that echoes the effective command override + args into its command
/// line, so tests can assert what the orchestrator actually planned.
struct EchoAdapter;

impl AgentAdapter for EchoAdapter {
    fn id(&self) -> &'static str {
        "echo"
    }

    fn display_name(&self) -> &'static str {
        "Echo"
    }

    fn binary(&self) -> &'static str {
        "echo-agent"
    }

    fn build_launch(&self, ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
        let bin = ctx.command_override.unwrap_or(self.binary());
        let mut parts = vec![bin.to_string()];
        parts.extend(ctx.extra_args.iter().cloned());
        Ok(LaunchPlan {
            command: parts.join(" "),
            env: Vec::new(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// A second echo-style adapter, so default-adapter selection can be observed
/// by which binary lands in the planned command line.
struct BetaAdapter;

impl AgentAdapter for BetaAdapter {
    fn id(&self) -> &'static str {
        "beta"
    }

    fn display_name(&self) -> &'static str {
        "Beta"
    }

    fn binary(&self) -> &'static str {
        "beta-agent"
    }

    fn build_launch(&self, _ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
        Ok(LaunchPlan {
            command: self.binary().to_string(),
            env: Vec::new(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Which adapter binary the orchestrator planned for a spawned agent.
fn planned_command(orch: &Orchestrator, project_id: ProjectId, id: ProcessId) -> String {
    orch.list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.command)
        .expect("process present")
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_spawn_uses_the_global_default_adapter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Orchestrator::new().with_adapters(AdapterRegistry::new(vec![
        Arc::new(EchoAdapter),
        Arc::new(BetaAdapter),
    ]));
    orch.set_agent_settings_path(dir.path().join("agents.json"));
    // No project podium.yml default; the global default should decide.
    orch.set_agent_default_adapter(Some("beta".to_string()))
        .expect("set default adapter");

    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .spawn_agent(project_id, None, None, None, vec![], vec![], false, None)
        .await
        .expect("spawn agent");

    assert_eq!(planned_command(&orch, project_id, id), "beta-agent");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn project_default_adapter_overrides_the_global_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The project pins "echo"; the global default is "beta" — the project wins.
    std::fs::write(
        dir.path().join("podium.yml"),
        "agents:\n  default_adapter: echo\n",
    )
    .expect("write podium.yml");
    let orch = Orchestrator::new().with_adapters(AdapterRegistry::new(vec![
        Arc::new(EchoAdapter),
        Arc::new(BetaAdapter),
    ]));
    orch.set_agent_settings_path(dir.path().join("agents.json"));
    orch.set_agent_default_adapter(Some("beta".to_string()))
        .expect("set default adapter");

    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .spawn_agent(project_id, None, None, None, vec![], vec![], false, None)
        .await
        .expect("spawn agent");

    assert_eq!(planned_command(&orch, project_id, id), "echo-agent");
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_applies_global_override_and_default_args() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings_file = dir.path().join("agents.json");
    let orch = Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(EchoAdapter)]));
    orch.set_agent_settings_path(settings_file);
    orch.set_agent_override(
        "echo",
        Some("/opt/echo".to_string()),
        vec!["--model".to_string(), "opus".to_string()],
    )
    .expect("set override");

    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let id = orch
        .spawn_agent(
            project_id,
            Some("echo".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            None,
        )
        .await
        .expect("spawn agent");

    let command = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.command)
        .expect("process present");
    assert_eq!(command, "/opt/echo --model opus");

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_args_override_replaces_global_default_args() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settings_file = dir.path().join("agents.json");
    let orch = Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(EchoAdapter)]));
    orch.set_agent_settings_path(settings_file);
    orch.set_agent_override(
        "echo",
        None,
        vec!["--model".to_string(), "opus".to_string()],
    )
    .expect("set override");

    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    // A per-session override wins over the global default args.
    let id = orch
        .spawn_agent(
            project_id,
            Some("echo".to_string()),
            None,
            None,
            vec![],
            vec![],
            false,
            Some(vec!["--model".to_string(), "haiku".to_string()]),
        )
        .await
        .expect("spawn agent");

    let command = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.command)
        .expect("process present");
    assert_eq!(command, "echo-agent --model haiku");

    orch.shutdown().await;
}

/// A throwaway git repo with one commit, identity pinned so commits work on
/// machines without global git config.
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

#[tokio::test(flavor = "multi_thread")]
async fn worktree_tools_round_trip_create_list_remove() {
    let dir = init_git_repo();
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let created = tools
        .create_worktree(Parameters(CreateWorktreeParams {
            project_id: project_id.to_string(),
            name: "Fix Login".to_string(),
        }))
        .await
        .expect("create_worktree over MCP");
    let created_json =
        serde_json::from_str::<serde_json::Value>(&first_text(&created)).expect("worktree json");
    assert_eq!(created_json["name"], "fix-login");
    assert_eq!(created_json["branch"], "podium/fix-login");
    assert_eq!(created_json["inUse"], false);

    let listed = tools
        .list_worktrees(Parameters(ListWorktreesParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list_worktrees over MCP");
    let listed_json =
        serde_json::from_str::<serde_json::Value>(&first_text(&listed)).expect("list json");
    let names: Vec<&str> = listed_json
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|w| w["name"].as_str())
        .collect();
    assert_eq!(names, ["fix-login"]);

    tools
        .remove_worktree(Parameters(RemoveWorktreeParams {
            project_id: project_id.to_string(),
            name: "fix-login".to_string(),
            force: None,
        }))
        .await
        .expect("remove_worktree over MCP");
    let listed = tools
        .list_worktrees(Parameters(ListWorktreesParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list after remove");
    let listed_json =
        serde_json::from_str::<serde_json::Value>(&first_text(&listed)).expect("list json");
    assert!(listed_json.as_array().expect("array").is_empty());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_tool_with_worktree_reports_it_in_the_snapshot() {
    let dir = init_git_repo();
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let spawned = tools
        .spawn_agent(Parameters(SpawnAgentParams {
            project_id: project_id.to_string(),
            prompt: None,
            name: Some("isolated work".to_string()),
            adapter_id: Some("fake".to_string()),
            todo_id: None,
            todo_ids: None,
            scratchpad_id: None,
            scratchpad_ids: None,
            worktree: Some(true),
        }))
        .await
        .expect("spawn_agent with worktree over MCP");
    let snapshot =
        serde_json::from_str::<serde_json::Value>(&first_text(&spawned)).expect("process json");
    assert_eq!(snapshot["worktree"], "isolated-work");

    // The worktree shows up in list_worktrees as in use.
    let listed = tools
        .list_worktrees(Parameters(ListWorktreesParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list_worktrees over MCP");
    let listed_json =
        serde_json::from_str::<serde_json::Value>(&first_text(&listed)).expect("list json");
    let entry = listed_json
        .as_array()
        .expect("array")
        .iter()
        .find(|w| w["name"] == "isolated-work")
        .expect("spawned worktree listed")
        .clone();
    assert_eq!(entry["inUse"], true);

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_in_worktree_on_a_non_git_project_fails_cleanly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let err = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            None,
            None,
            vec![],
            vec![],
            true,
            None,
        )
        .await
        .expect_err("worktree spawn on a non-git project must fail");
    assert!(matches!(err, CoreError::NotAGitRepo), "got: {err}");
    // The failure happened before any process was created.
    assert!(orch.list_processes(Some(project_id)).is_empty());

    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_worktree_is_refused_while_an_agent_runs_in_it() {
    let dir = init_git_repo();
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            Some("busy".to_string()),
            None,
            vec![],
            vec![],
            true,
            None,
        )
        .await
        .expect("spawn agent in worktree");
    let info = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == agent)
        .expect("agent listed");
    assert_eq!(info.worktree.as_deref(), Some("busy"));

    let err = orch
        .remove_worktree(project_id, "busy", false)
        .expect_err("removal must be refused while the agent runs in it");
    assert!(matches!(err, CoreError::WorktreeInUse), "got: {err}");

    // Once the agent is gone, removal succeeds.
    orch.remove_process(agent).await.expect("remove agent");
    orch.remove_worktree(project_id, "busy", false)
        .expect("remove worktree after agent stopped");

    orch.shutdown().await;
}
