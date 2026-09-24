//! Integration tests for the built-in MCP server (phase 6): lifecycle,
//! bearer auth, the tool surface, and the agent recursion guard.

use std::sync::Arc;
use std::time::Duration;

use podium_core::mcp::tools::{
    AddTodoLinkParams, AddTodoParams, AssignTodoParams, CommentTodoParams, CreateScratchpadParams,
    CreateWorktreeParams, GetProcessOutputParams, ListScratchpadsParams, ListTodosParams,
    ListWorktreesParams, PodiumTools, ProcessParams, RemoveWorktreeParams, RenameSessionParams,
    RespawnAgentParams, ScratchpadTagParams, SetScratchpadArchivedParams, SpawnAgentParams,
    UpdateAssistantMemoryParams, UpdateScratchpadParams, UpdateTodoParams, VerifySenderParams,
};
use podium_core::mcp::{self, McpServer, ASSISTANT_PATH};
use podium_core::{
    AdapterRegistry, AgentAdapter, AgentLaunchCtx, AutonomyMode, CoreError, CoreResult, LaunchPlan,
    Orchestrator, ProcessId, ProcessKind, ProcessSpec, ProcessStatus, ProjectId, RestartPolicy,
    TodoId,
};
// Only the `#[cfg(unix)]` report_to_assistant tests (fake shell-script CLI) use these.
#[cfg(unix)]
use podium_core::mcp::tools::ReportToAssistantParams;
#[cfg(unix)]
use podium_core::TranscriptRole;
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

/// Like [`http_post_mcp_with_session`], but against any endpoint — the
/// assistant has its own (`ASSISTANT_PATH`).
async fn http_post_path(
    base_url: &str,
    path: &str,
    token: Option<&str>,
    session_id: Option<&str>,
    body: &str,
) -> (u16, String) {
    http_post_inner(base_url, path, token, session_id, body).await
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
    http_post_inner(base_url, "/mcp", token, session_id, body).await
}

async fn http_post_inner(
    base_url: &str,
    path: &str,
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
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\n{auth}{session}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        "verify_sender",
    ] {
        assert!(
            tools.contains(&name),
            "{name} missing from tools/list: {tools:?}"
        );
    }

    server.stop();
}

// `printf` is not a `cmd.exe` builtin, so on Windows this process fails to
// produce the expected ANSI-coded "red" output.
#[cfg_attr(windows, ignore = "printf has no cmd.exe equivalent")]
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
async fn remove_process_tool_removes_it_from_the_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let pid = orch
        .add_process(
            project_id,
            ProcessSpec {
                name: "sleeper".to_string(),
                command: "sleep 60".to_string(),
                cwd: dir.path().to_path_buf(),
                env: Vec::new(),
                kind: ProcessKind::Service,
                restart_policy: RestartPolicy::Never,
                color: None,
            },
        )
        .await
        .expect("add process");

    tools
        .remove_process(Parameters(ProcessParams {
            process_id: pid.to_string(),
        }))
        .await
        .expect("remove_process");

    assert!(
        orch.list_processes(Some(project_id)).is_empty(),
        "process removed from the list"
    );

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
            description: Some("start from the existing session handler".to_string()),
        }))
        .await
        .expect("add_todo");
    let added_text = first_text(&added);
    assert!(
        added_text.contains("start from the existing session handler"),
        "{added_text:?}"
    );
    let todo_id = serde_json::from_str::<serde_json::Value>(&added_text).expect("todo json")["id"]
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

    // list_todos must reflect the revised text/description and link — but
    // carry the comments as a count only, so a chatty to-do cannot blow up
    // the listing.
    let listed_result = tools
        .list_todos(Parameters(ListTodosParams {
            project_id: project_id.to_string(),
            todo_id: None,
        }))
        .await
        .expect("list_todos");
    let listed = first_text(&listed_result);
    assert!(listed.contains("wire up OAuth"), "{listed:?}");
    assert!(listed.contains("#42 Fix login"), "{listed:?}");
    assert!(listed.contains("\"commentCount\": 1"), "{listed:?}");
    assert!(
        !listed.contains("handler stubbed out"),
        "comment bodies must stay out of the listing: {listed:?}"
    );

    // Asking for the one to-do returns the history the listing left out.
    let detail_result = tools
        .list_todos(Parameters(ListTodosParams {
            project_id: project_id.to_string(),
            todo_id: Some(todo_id.clone()),
        }))
        .await
        .expect("list_todos detail");
    let detail = first_text(&detail_result);
    assert!(detail.contains("handler stubbed out"), "{detail:?}");
    assert!(detail.contains("claude"), "{detail:?}");

    // An unknown id is an error, not an empty listing.
    assert!(
        tools
            .list_todos(Parameters(ListTodosParams {
                project_id: project_id.to_string(),
                todo_id: Some(uuid::Uuid::new_v4().to_string()),
            }))
            .await
            .is_err(),
        "an unknown to-do id must be refused"
    );

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
async fn update_assistant_memory_round_trip_via_mcp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    orch.configure_assistant(podium_core::AssistantConfig {
        command: "claude".to_string(),
        cwd: None,
        data_dir: Some(dir.path().to_path_buf()),
    });
    let tools = PodiumTools::new(Arc::clone(&orch));

    let saved = tools
        .update_assistant_memory(Parameters(UpdateAssistantMemoryParams {
            content: "The user ships on Fridays.".to_string(),
        }))
        .await
        .expect("update_assistant_memory");
    assert!(first_text(&saved).contains("Memory saved"));
    assert_eq!(orch.assistant_memory(), "The user ships on Fridays.");

    // The same tool clears it, so the assistant can drop stale notes.
    tools
        .update_assistant_memory(Parameters(UpdateAssistantMemoryParams {
            content: String::new(),
        }))
        .await
        .expect("clear memory");
    assert_eq!(orch.assistant_memory(), "");

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
///
/// Every test below (and the other fake adapters further down: `EchoAdapter`,
/// `BetaAdapter`, `PromptEchoAdapter`) plans a command line for a binary that
/// is not installed, or not reliably present, on Windows (`sleep`, `fake-agent`,
/// `echo-agent`, `beta-agent`, `/opt/echo`, …) — `sleep` only resolves there via
/// Git for Windows' `usr\bin` on `PATH`, which GitHub's `windows-latest` runner
/// happens to have but which isn't guaranteed. On Unix that is harmless:
/// `ProcessKind::Agent` still runs through the login shell (`$SHELL -lic
/// <command>`, see `PtyProcess::build_command`), so the *shell* spawns fine and
/// only the command it tries to run inside fails — invisibly, since none of
/// these tests waits on the process's own exit. On Windows, `build_command`
/// bypasses the shell entirely for agent commands (see its doc comment) and
/// hands the binary straight to `CreateProcessW`; an unresolved binary makes
/// that spawn itself fail, so `spawn_agent`/`add_process` + `start_process`
/// return `Err` before the test's own assertions ever run — and for `sleep`
/// specifically, whether it resolves at all is environment-dependent. Every
/// test that spawns one of these fake adapters is therefore
/// `#[cfg_attr(windows, ignore)]`.
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .add_todo(project_id, "wire up OAuth", None)
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .add_todo(project_id, "wire up OAuth", None)
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .add_todo(project_id, "wire up OAuth", None)
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
            None,
        )
        .await
        .expect("spawn bare agent");
    let todo = orch
        .add_todo(project_id, "wire up OAuth", None)
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

/// A fake CLI just enough of the stream-json protocol to prove a report
/// reaches a live assistant session: greets with an init line, then answers
/// every stdin turn with a success result.
#[cfg(unix)]
const FAKE_ASSISTANT_CLAUDE: &str = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
while IFS= read -r _line; do
  echo '{"type":"result","subtype":"success","is_error":false,"result":"ok"}'
done
"#;

/// A fake CLI that only records that it was launched, standing in for "no
/// session should ever be spawned just to deliver a report".
#[cfg(unix)]
const ARGV_RECORDING_CLAUDE: &str =
    "#!/bin/sh\nprintf ok > \"$(dirname \"$0\")/argv.txt\"\nsleep 5\n";

#[cfg(unix)]
fn write_fake_cli(dir: &std::path::Path, script: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude");
    std::fs::write(&path, script).expect("write fake cli");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn report_to_assistant_names_the_agent_and_delivers_it_live() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    orch.configure_assistant(podium_core::AssistantConfig {
        command: write_fake_cli(dir.path(), FAKE_ASSISTANT_CLAUDE),
        cwd: Some(dir.path().to_path_buf()),
        data_dir: Some(dir.path().to_path_buf()),
    });
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            Some("Login bug agent".to_string()),
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .expect("spawn agent");

    // A live, *idle* conversation exists before the agent reports back: a
    // report during a still-running turn now queues (message queueing) and
    // this test is specifically about immediate, live delivery.
    let mut rx = orch.subscribe_events();
    orch.assistant_send("hi").await.expect("send");
    timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(podium_core::PodiumEvent::AssistantEvent { event }) = rx.recv().await {
                if matches!(
                    event,
                    podium_core::assistant::AssistantEvent::TurnEnd { .. }
                ) {
                    return;
                }
            }
        }
    })
    .await
    .expect("turn ends");

    tools
        .report_to_assistant(Parameters(ReportToAssistantParams {
            process_id: agent.to_string(),
            summary: "Fixed the redirect; tests green.".to_string(),
        }))
        .await
        .expect("report_to_assistant");

    let transcript = orch.assistant_transcript();
    let report = transcript
        .iter()
        .rev()
        .find(|e| e.role == TranscriptRole::Nudge)
        .expect("the report is delivered into the live session as a nudge");
    assert!(report.text.contains("Login bug agent"), "{}", report.text);
    assert!(
        report.text.contains("Fixed the redirect; tests green."),
        "{}",
        report.text
    );

    // An unknown process cannot claim to be another agent.
    assert!(tools
        .report_to_assistant(Parameters(ReportToAssistantParams {
            process_id: ProcessId::new().to_string(),
            summary: "impersonation attempt".to_string(),
        }))
        .await
        .is_err());

    orch.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn report_to_assistant_starts_the_session_when_nobody_is_chatting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)])),
    );
    orch.configure_assistant(podium_core::AssistantConfig {
        command: write_fake_cli(dir.path(), ARGV_RECORDING_CLAUDE),
        cwd: Some(dir.path().to_path_buf()),
        data_dir: Some(dir.path().to_path_buf()),
    });
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let agent = orch
        .spawn_agent(
            project_id,
            Some("fake".to_string()),
            Some("Login bug agent".to_string()),
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .expect("spawn agent");

    // No conversation is running: the report starts one rather than waiting
    // for the user to happen to type — an agent that is blocked is worth
    // hearing about now.
    tools
        .report_to_assistant(Parameters(ReportToAssistantParams {
            process_id: agent.to_string(),
            summary: "Blocked: need credentials.".to_string(),
        }))
        .await
        .expect("report_to_assistant");

    // The CLI records its argv on spawn, on its own task.
    tokio::time::timeout(Duration::from_secs(30), async {
        while !dir.path().join("argv.txt").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a report must start the assistant's CLI");
    assert!(
        !orch.assistant_transcript().is_empty(),
        "the report was delivered, so it shows in the chat"
    );

    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

    let a = orch
        .add_todo(project_id, "wire up auth", None)
        .expect("todo a");
    let b = orch
        .add_todo(project_id, "write tests", None)
        .expect("todo b");

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
            worktree_name: None,
            args: None,
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

/// Like [`FakeAgentAdapter`], but writes the prompt into the command line so
/// a test can see what the agent was actually launched with.
struct PromptEchoAdapter;

impl AgentAdapter for PromptEchoAdapter {
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
        Ok(LaunchPlan {
            command: format!("sleep 30 # {}", ctx.prompt.unwrap_or("")),
            env: Vec::new(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn respawn_agent_hands_the_to_do_and_the_answer_to_a_fresh_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(PromptEchoAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let todo = orch
        .add_todo(project_id, "wire up auth", None)
        .expect("add to-do");
    let stuck = orch
        .spawn_agent(
            project_id,
            Some("echo".to_string()),
            None,
            None,
            vec![todo.id],
            Vec::new(),
            false,
            None,
            None,
        )
        .await
        .expect("spawn the stuck agent");

    let spawned = tools
        .respawn_agent(Parameters(RespawnAgentParams {
            process_id: stuck.to_string(),
            answer: "Use the release branch, not main.".to_string(),
            task: None,
        }))
        .await
        .expect("respawn over MCP");
    let info: serde_json::Value =
        serde_json::from_str(&first_text(&spawned)).expect("process json");
    let fresh: ProcessId = info["id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("process id");
    assert_ne!(fresh, stuck, "a replacement, not the same session");

    // The answer travels in the prompt, and so does the to-do it inherited —
    // that context is exactly what the caller could not have reassembled.
    let command = info["command"].as_str().expect("command");
    assert!(
        command.contains("Use the release branch, not main."),
        "the answer is missing from the prompt: {command}"
    );
    assert!(
        command.contains("wire up auth"),
        "the inherited to-do is missing from the prompt: {command}"
    );

    // The to-do changed hands, so the sidebar shows the live session owning it.
    let owner = orch
        .list_todos(project_id)
        .expect("list to-dos")
        .into_iter()
        .find(|t| t.id == todo.id)
        .expect("the to-do")
        .assigned_agent
        .expect("assigned");
    assert_eq!(owner.process_id, fresh);

    // The stuck one is on its way out rather than left running alongside.
    let old = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == stuck)
        .expect("the stuck agent");
    assert!(
        !matches!(old.status, ProcessStatus::Running { .. }),
        "the stuck agent kept running: {:?}",
        old.status
    );

    orch.shutdown().await;
}

/// Without a to-do there is nothing to carry the task, so an answer on its own
/// would spawn a session that knows only the reply to a question nobody asked
/// it. Refusing beats burning an agent.
#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn respawning_an_agent_with_no_to_do_needs_a_task_as_well() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(PromptEchoAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let stuck = orch
        .spawn_agent(
            project_id,
            Some("echo".to_string()),
            None,
            Some("look at the failing build".to_string()),
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
        )
        .await
        .expect("spawn the stuck agent");

    let refused = tools
        .respawn_agent(Parameters(RespawnAgentParams {
            process_id: stuck.to_string(),
            answer: "Use the release branch.".to_string(),
            task: None,
        }))
        .await;
    assert!(refused.is_err(), "a contextless respawn should be refused");
    assert!(
        matches!(
            orch.list_processes(Some(project_id))
                .into_iter()
                .find(|p| p.id == stuck)
                .expect("the stuck agent")
                .status,
            ProcessStatus::Running { .. }
        ),
        "a refused respawn must not have stopped anything"
    );

    // With the task supplied it goes through, carrying both halves.
    let spawned = tools
        .respawn_agent(Parameters(RespawnAgentParams {
            process_id: stuck.to_string(),
            answer: "Use the release branch.".to_string(),
            task: Some("look at the failing build".to_string()),
        }))
        .await
        .expect("respawn with a task");
    let command = serde_json::from_str::<serde_json::Value>(&first_text(&spawned))
        .expect("process json")["command"]
        .as_str()
        .expect("command")
        .to_string();
    assert!(command.contains("look at the failing build"), "{command}");
    assert!(command.contains("Use the release branch."), "{command}");

    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .add_todo(project_id, "wire up OAuth", None)
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .add_todo(project_id, "wire up OAuth", None)
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
            None,
        )
        .await
        .expect("spawn agent for todo, ignoring the bogus scratchpad id");

    assert_eq!(orch.agent_for_todo(todo.id), Some(agent));
    assert_eq!(orch.agent_for_scratchpad(bogus_id), None);

    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
            worktree_name: None,
            args: None,
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .spawn_agent(
            project_id,
            None,
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .expect("spawn agent");

    assert_eq!(planned_command(&orch, project_id, id), "beta-agent");
    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
        .spawn_agent(
            project_id,
            None,
            None,
            None,
            vec![],
            vec![],
            false,
            None,
            None,
        )
        .await
        .expect("spawn agent");

    assert_eq!(planned_command(&orch, project_id, id), "echo-agent");
    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
            None,
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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_tool_args_become_the_launch_args_override() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(EchoAdapter)])),
    );
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    tools
        .spawn_agent(Parameters(SpawnAgentParams {
            project_id: project_id.to_string(),
            prompt: None,
            name: None,
            adapter_id: Some("echo".to_string()),
            todo_id: None,
            todo_ids: None,
            scratchpad_id: None,
            scratchpad_ids: None,
            worktree: None,
            worktree_name: None,
            args: Some(vec!["--model".to_string(), "opus".to_string()]),
        }))
        .await
        .expect("spawn_agent over MCP");

    let id = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| matches!(p.kind, ProcessKind::Agent { .. }))
        .map(|p| p.id)
        .expect("agent process");
    assert_eq!(
        planned_command(&orch, project_id, id),
        "echo-agent --model opus",
        "the args param becomes this spawn's args_override"
    );

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

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_with_worktree_does_not_create_one() {
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
            worktree_name: Some("isolated work".to_string()),
            args: None,
        }))
        .await
        .expect("spawn_agent with worktree over MCP");
    let snapshot =
        serde_json::from_str::<serde_json::Value>(&first_text(&spawned)).expect("process json");
    // The worktree is the agent's job now: it starts in the project root and
    // is only told to create one, so Podium made nothing up front.
    assert!(snapshot["worktree"].is_null());
    let listed = tools
        .list_worktrees(Parameters(ListWorktreesParams {
            project_id: project_id.to_string(),
        }))
        .await
        .expect("list_worktrees over MCP");
    let listed_json =
        serde_json::from_str::<serde_json::Value>(&first_text(&listed)).expect("list json");
    assert!(listed_json.as_array().expect("array").is_empty());

    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn spawn_agent_with_worktree_works_on_a_non_git_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch =
        Orchestrator::new().with_adapters(AdapterRegistry::new(vec![Arc::new(FakeAgentAdapter)]));
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    // Nothing git-related happens at spawn any more, so a non-git project no
    // longer fails: the agent gets the instruction and can refuse/adapt.
    orch.spawn_agent(
        project_id,
        Some("fake".to_string()),
        None,
        None,
        vec![],
        vec![],
        true,
        None,
        None,
    )
    .await
    .expect("worktree spawn on a non-git project still spawns");
    assert_eq!(orch.list_processes(Some(project_id)).len(), 1);

    orch.shutdown().await;
}

// `sleep` has no `cmd.exe` builtin equivalent, so this "busy" terminal
// process would exit almost instantly on Windows instead of holding the
// worktree open for the removal check.
#[cfg_attr(windows, ignore = "`sleep` has no cmd.exe equivalent")]
#[tokio::test(flavor = "multi_thread")]
async fn remove_worktree_is_refused_while_a_process_runs_in_it() {
    let dir = init_git_repo();
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");

    let wt = orch
        .create_worktree(project_id, "busy")
        .expect("create worktree");
    let pid = orch
        .add_process(
            project_id,
            ProcessSpec {
                name: "busy-shell".to_string(),
                command: "sleep 30".to_string(),
                cwd: wt.path.clone(),
                env: Vec::new(),
                kind: ProcessKind::Terminal,
                restart_policy: RestartPolicy::Never,
                color: None,
            },
        )
        .await
        .expect("add process");
    orch.start_process(pid).await.expect("start process");
    let info = orch
        .list_processes(Some(project_id))
        .into_iter()
        .find(|p| p.id == pid)
        .expect("process listed");
    assert_eq!(info.worktree.as_deref(), Some("busy"));

    let err = orch
        .remove_worktree(project_id, "busy", false)
        .expect_err("removal must be refused while a process runs in it");
    assert!(matches!(err, CoreError::WorktreeInUse), "got: {err}");

    // Once the process is gone, removal succeeds.
    orch.remove_process(pid).await.expect("remove process");
    orch.remove_worktree(project_id, "busy", false)
        .expect("remove worktree after the process stopped");

    orch.shutdown().await;
}

#[cfg_attr(
    windows,
    ignore = "spawns a fake agent binary; see FakeAgentAdapter's comment"
)]
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
            None,
        )
    };
    let first = spawn_bare().await.expect("spawn first agent");
    let second = spawn_bare().await.expect("spawn second agent");
    let todo = orch
        .add_todo(project_id, "wire up OAuth", None)
        .expect("todo");
    let todo2 = orch
        .add_todo(project_id, "write tests", None)
        .expect("todo2");

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

// ---------------------------------------------------------------------------
// The personal assistant's endpoint and its autonomy gate
// ---------------------------------------------------------------------------

fn tools_call_request(id: u64, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    })
    .to_string()
}

/// Open an MCP session on `path` (initialize + initialized) and return its id.
async fn open_session(server: &McpServer, path: &str) -> String {
    let (status, response) =
        http_post_path(server.url(), path, Some(server.token()), None, INITIALIZE).await;
    assert_eq!(status, 200, "initialize on {path}: {response}");
    let session_id = extract_session_id(&response);
    let (status, _) = http_post_path(
        server.url(),
        path,
        Some(server.token()),
        Some(&session_id),
        INITIALIZED,
    )
    .await;
    assert_eq!(status, 202);
    session_id
}

/// Call one tool over `path` and return the text of its first content block.
async fn call_tool_text(
    server: &McpServer,
    path: &str,
    session_id: &str,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> String {
    let (status, response) = http_post_path(
        server.url(),
        path,
        Some(server.token()),
        Some(session_id),
        &tools_call_request(id, name, arguments),
    )
    .await;
    assert_eq!(status, 200, "{name} on {path}: {response}");
    let body = extract_json_body(&response);
    body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("text content in {body}"))
        .to_string()
}

/// A project with one to-do, and the to-do's id.
async fn project_with_todo(orch: &Orchestrator, dir: &std::path::Path) -> (ProjectId, TodoId) {
    let project_id = orch
        .open_project(dir.to_path_buf())
        .await
        .expect("open project");
    let todo = orch
        .add_todo(project_id, "review the onboarding copy", None)
        .expect("add todo");
    (project_id, todo.id)
}

fn active_todo_count(orch: &Orchestrator, project_id: ProjectId) -> usize {
    orch.list_todos(project_id).expect("list todos").len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_destructive_assistant_call_waits_for_the_user_and_then_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let (project_id, todo_id) = project_with_todo(&orch, dir.path()).await;
    let server = mcp::start(Arc::clone(&orch), dir.path().join("mcp"))
        .await
        .expect("mcp server starts");
    // Default mode: destructive actions need a confirmation.
    assert_eq!(orch.assistant_autonomy(), AutonomyMode::ConfirmDestructive);
    let session = open_session(&server, ASSISTANT_PATH).await;

    let answer = call_tool_text(
        &server,
        ASSISTANT_PATH,
        &session,
        2,
        "set_todo_archived",
        serde_json::json!({
            "project_id": project_id.to_string(),
            "todo_id": todo_id.to_string(),
            "archived": true,
        }),
    )
    .await;
    assert!(
        answer.contains("waiting for the user"),
        "gated call must not report success: {answer}"
    );
    // Nothing happened yet — that is the whole point of the gate.
    assert_eq!(active_todo_count(&orch, project_id), 1);

    let pending = orch.assistant_proposals();
    assert_eq!(pending.len(), 1, "one proposal is waiting");
    assert_eq!(pending[0].tool, "set_todo_archived");
    assert!(
        pending[0].summary.contains(&todo_id.to_string()),
        "the card must show what is being approved: {}",
        pending[0].summary
    );

    // A non-destructive call is not held back in this mode.
    let added = call_tool_text(
        &server,
        ASSISTANT_PATH,
        &session,
        3,
        "add_todo",
        serde_json::json!({ "project_id": project_id.to_string(), "text": "ship the changelog" }),
    )
    .await;
    assert!(added.contains("ship the changelog"), "{added}");
    assert_eq!(active_todo_count(&orch, project_id), 2);

    orch.resolve_assistant_proposal(pending[0].id, true)
        .await
        .expect("confirm");
    assert!(orch.assistant_proposals().is_empty());
    assert_eq!(
        active_todo_count(&orch, project_id),
        1,
        "the confirmed action archived the to-do"
    );

    server.stop();
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_assistant_proposal_never_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let (project_id, todo_id) = project_with_todo(&orch, dir.path()).await;
    let server = mcp::start(Arc::clone(&orch), dir.path().join("mcp"))
        .await
        .expect("mcp server starts");
    let session = open_session(&server, ASSISTANT_PATH).await;

    call_tool_text(
        &server,
        ASSISTANT_PATH,
        &session,
        2,
        "set_todo_archived",
        serde_json::json!({
            "project_id": project_id.to_string(),
            "todo_id": todo_id.to_string(),
            "archived": true,
        }),
    )
    .await;
    let id = orch.assistant_proposals()[0].id;
    orch.resolve_assistant_proposal(id, false)
        .await
        .expect("reject");

    assert!(orch.assistant_proposals().is_empty());
    assert_eq!(active_todo_count(&orch, project_id), 1, "to-do untouched");
    // The decision is final: the proposal is gone, not re-runnable.
    assert!(orch.resolve_assistant_proposal(id, true).await.is_err());

    server.stop();
    orch.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_autonomy_mode_decides_what_is_gated_and_agents_never_are() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let (project_id, todo_id) = project_with_todo(&orch, dir.path()).await;
    let server = mcp::start(Arc::clone(&orch), dir.path().join("mcp"))
        .await
        .expect("mcp server starts");
    let assistant = open_session(&server, ASSISTANT_PATH).await;

    // propose-all: even adding a to-do is proposed first.
    orch.set_assistant_autonomy(AutonomyMode::ProposeAll)
        .expect("set autonomy");
    let answer = call_tool_text(
        &server,
        ASSISTANT_PATH,
        &assistant,
        2,
        "add_todo",
        serde_json::json!({ "project_id": project_id.to_string(), "text": "draft the release note" }),
    )
    .await;
    assert!(answer.contains("waiting for the user"), "{answer}");
    assert_eq!(active_todo_count(&orch, project_id), 1);
    let id = orch.assistant_proposals()[0].id;
    orch.resolve_assistant_proposal(id, true)
        .await
        .expect("confirm");
    assert_eq!(active_todo_count(&orch, project_id), 2);

    // …while an agent on the ungated endpoint archives without asking, in the
    // very same mode.
    let agent = open_session(&server, "/mcp").await;
    call_tool_text(
        &server,
        "/mcp",
        &agent,
        3,
        "set_todo_archived",
        serde_json::json!({
            "project_id": project_id.to_string(),
            "todo_id": todo_id.to_string(),
            "archived": true,
        }),
    )
    .await;
    assert!(
        orch.assistant_proposals().is_empty(),
        "agents are not gated"
    );
    assert_eq!(active_todo_count(&orch, project_id), 1);

    // full autonomy: the assistant archives straight away too.
    orch.set_assistant_autonomy(AutonomyMode::Full)
        .expect("set autonomy");
    let remaining = orch.list_todos(project_id).expect("list todos")[0].id;
    call_tool_text(
        &server,
        ASSISTANT_PATH,
        &assistant,
        4,
        "set_todo_archived",
        serde_json::json!({
            "project_id": project_id.to_string(),
            "todo_id": remaining.to_string(),
            "archived": true,
        }),
    )
    .await;
    assert!(orch.assistant_proposals().is_empty());
    assert_eq!(active_todo_count(&orch, project_id), 0);

    server.stop();
    orch.shutdown().await;
}

/// Every tool, and whether it is read-only / destructive for the gate. This
/// table is the guard: a newly added tool fails this test until it is
/// classified in `assistant.rs`, so it can never silently escape
/// "confirm destructive".
const TOOL_CLASSIFICATION: &[(&str, bool, bool)] = &[
    // (tool, read_only, destructive)
    ("list_projects", true, false),
    ("list_processes", true, false),
    ("get_process_status", true, false),
    ("get_process_output", true, false),
    ("list_todos", true, false),
    ("list_scratchpads", true, false),
    ("list_worktrees", true, false),
    ("verify_sender", true, false),
    ("spawn_agent", false, false),
    ("start_process", false, false),
    ("stop_process", false, true),
    ("respawn_agent", false, true),
    ("restart_process", false, true),
    ("remove_process", false, true),
    ("add_todo", false, false),
    ("complete_todo", false, false),
    ("update_todo", false, false),
    ("comment_todo", false, false),
    ("add_todo_link", false, false),
    ("set_todo_archived", false, true),
    ("assign_todo", false, false),
    ("rename_session", false, false),
    ("create_scratchpad", false, false),
    ("update_scratchpad", false, false),
    ("add_scratchpad_tag", false, false),
    ("remove_scratchpad_tag", false, true),
    ("set_scratchpad_archived", false, true),
    ("update_assistant_memory", false, false),
    ("create_worktree", false, false),
    ("remove_worktree", false, true),
    ("report_to_assistant", false, false),
];

#[tokio::test(flavor = "multi_thread")]
async fn every_tool_is_classified_for_the_autonomy_gate() {
    use podium_core::assistant::requires_proposal;

    let dir = tempfile::tempdir().expect("tempdir");
    let (_orch, server) = start_server(dir.path().join("mcp")).await;
    let session = open_session(&server, ASSISTANT_PATH).await;

    let (status, response) = http_post_path(
        server.url(),
        ASSISTANT_PATH,
        Some(server.token()),
        Some(&session),
        &tools_list_request(2),
    )
    .await;
    assert_eq!(status, 200);
    let body = extract_json_body(&response);
    let tools: Vec<String> = body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        tools.len(),
        TOOL_CLASSIFICATION.len(),
        "classify every tool for the gate; listed: {tools:?}"
    );

    for name in &tools {
        let (_, read_only, destructive) = TOOL_CLASSIFICATION
            .iter()
            .find(|(tool, _, _)| tool == name)
            .unwrap_or_else(|| panic!("{name} is not classified for the autonomy gate"));
        assert!(!(*read_only && *destructive), "{name}");
        assert!(
            !requires_proposal(AutonomyMode::Full, name),
            "{name} must run in full autonomy"
        );
        assert_eq!(
            requires_proposal(AutonomyMode::ConfirmDestructive, name),
            *destructive,
            "{name} under confirm-destructive"
        );
        assert_eq!(
            requires_proposal(AutonomyMode::ProposeAll, name),
            !*read_only,
            "{name} under propose-all"
        );
    }

    server.stop();
}

// Adds an `Agent`-kind process directly with `command: "sleep 30"`: on
// Windows agent commands bypass the shell and exec the binary directly (see
// FakeAgentAdapter's comment), and whether `sleep` resolves at all there is
// environment-dependent, so the spawn is unreliable.
#[cfg_attr(
    windows,
    ignore = "spawns `sleep` as an agent binary; see FakeAgentAdapter's comment"
)]
#[tokio::test(flavor = "multi_thread")]
async fn verify_sender_answers_from_podiums_own_registry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orch = Arc::new(Orchestrator::new());
    let project_id = orch
        .open_project(dir.path().to_path_buf())
        .await
        .expect("open project");
    let tools = PodiumTools::new(Arc::clone(&orch));

    let verify = |name: &str| {
        let tools = tools.clone();
        let name = name.to_string();
        async move {
            let result = tools
                .verify_sender(Parameters(VerifySenderParams { name }))
                .await
                .expect("verify_sender");
            serde_json::from_str::<serde_json::Value>(&first_text(&result)).expect("json result")
        }
    };

    // The assistant's tag is random per app run, so no local session can
    // claim to be the assistant by picking a well-known name.
    let assistant_tag = orch.assistant_peer_tag();
    assert_eq!(assistant_tag.len(), 8);
    let verdict = verify(&podium_core::agent::peer_session_name(&assistant_tag)).await;
    assert_eq!(verdict["fleet"], true);
    assert_eq!(verdict["kind"], "assistant");

    // A live agent session resolves to its Podium process...
    let agent_id = orch
        .add_process(
            project_id,
            ProcessSpec {
                name: "worker".to_string(),
                command: "sleep 30".to_string(),
                cwd: dir.path().to_path_buf(),
                env: Vec::new(),
                kind: ProcessKind::Agent {
                    adapter: "claude-code".to_string(),
                },
                restart_policy: RestartPolicy::Never,
                color: None,
            },
        )
        .await
        .expect("add process");
    orch.start_process(agent_id).await.expect("start process");
    let agent_tag = podium_core::agent::peer_tag(&agent_id.to_string());
    let verdict = verify(&podium_core::agent::peer_session_name(&agent_tag)).await;
    assert_eq!(verdict["fleet"], true);
    assert_eq!(verdict["kind"], "agent");
    assert_eq!(verdict["processId"], agent_id.to_string());
    assert_eq!(verdict["session"], "worker");

    // ...and any other local session of this OS user does not, however its
    // message describes itself.
    for name in ["release-notes", "podium-assistant", "podium-00000000"] {
        assert_eq!(verify(name).await["fleet"], false, "{name} must not verify");
    }

    // A stopped agent's tag is stale: it cannot be the one talking.
    orch.stop_process(agent_id).await.expect("stop process");
    wait_for_exit(&orch, agent_id).await;
    assert_eq!(
        verify(&podium_core::agent::peer_session_name(&agent_tag)).await["fleet"],
        false,
        "an exited session must not verify"
    );

    orch.shutdown().await;
}
