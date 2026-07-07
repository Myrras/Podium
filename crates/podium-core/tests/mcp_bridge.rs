//! Integration tests for the stdio ↔ HTTP MCP bridge (`podium mcp-bridge`):
//! plain proxying, and transparent recovery when the server restarts on a
//! new port with a new token (only `server.json`'s path stays the same).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use podium_core::mcp::bridge::{run_bridge, BridgeTiming};
use podium_core::mcp::{self, McpServer};
use podium_core::Orchestrator;
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf,
};
use tokio::time::{sleep, timeout};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"podium-bridge-test","version":"0.0.0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

fn tools_list(id: u64) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#)
}

async fn start_server(config_dir: PathBuf) -> (Arc<Orchestrator>, McpServer) {
    let orch = Arc::new(Orchestrator::new());
    let server = mcp::start(Arc::clone(&orch), config_dir)
        .await
        .expect("mcp server starts");
    (orch, server)
}

/// The MCP client's side of the bridge's stdio.
struct Client {
    to_bridge: WriteHalf<DuplexStream>,
    from_bridge: Lines<BufReader<ReadHalf<DuplexStream>>>,
}

impl Client {
    async fn send(&mut self, line: &str) {
        self.to_bridge
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("write to bridge");
    }

    async fn recv(&mut self) -> serde_json::Value {
        let line = timeout(TEST_TIMEOUT, self.from_bridge.next_line())
            .await
            .expect("timely bridge response")
            .expect("read from bridge")
            .expect("bridge output stays open");
        serde_json::from_str(&line).expect("bridge emits valid json")
    }
}

fn spawn_bridge(server_json: PathBuf) -> Client {
    let (client_side, bridge_side) = tokio::io::duplex(64 * 1024);
    let (bridge_read, bridge_write) = tokio::io::split(bridge_side);
    let timing = BridgeTiming {
        initial_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(100),
        give_up_after: Duration::from_secs(15),
    };
    tokio::spawn(run_bridge(bridge_read, bridge_write, server_json, timing));
    let (from_bridge, to_bridge) = tokio::io::split(client_side);
    Client {
        to_bridge,
        from_bridge: BufReader::new(from_bridge).lines(),
    }
}

/// Poll until the old server's port stops accepting connections.
async fn wait_until_down(url: &str) {
    let addr = url.strip_prefix("http://").expect("http url").to_string();
    timeout(TEST_TIMEOUT, async {
        while tokio::net::TcpStream::connect(&addr).await.is_ok() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("old server shuts down");
}

#[tokio::test(flavor = "multi_thread")]
async fn proxies_initialize_and_tool_calls_over_stdio() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_dir = dir.path().join("mcp");
    let (_orch, server) = start_server(config_dir.clone()).await;
    let mut client = spawn_bridge(config_dir.join("server.json"));

    client.send(INITIALIZE).await;
    let response = client.recv().await;
    assert_eq!(response["id"], 1);
    assert!(
        response["result"]["protocolVersion"].is_string(),
        "initialize result relayed: {response}"
    );

    client.send(INITIALIZED).await;
    client.send(&tools_list(2)).await;
    let response = client.recv().await;
    assert_eq!(response["id"], 2);
    let tools = response["result"]["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|t| t["name"] == "list_projects"),
        "tool listing relayed: {response}"
    );
    server.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn survives_an_app_restart_with_new_port_and_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_dir = dir.path().join("mcp");
    let (_orch1, server1) = start_server(config_dir.clone()).await;
    let mut client = spawn_bridge(config_dir.join("server.json"));

    client.send(INITIALIZE).await;
    assert_eq!(client.recv().await["id"], 1);
    client.send(INITIALIZED).await;
    client.send(&tools_list(2)).await;
    assert_eq!(client.recv().await["id"], 2);

    // "Restart the app": stop the server, start a fresh one on a new
    // ephemeral port with a new token, same config dir.
    let old_url = server1.url().to_string();
    server1.stop();
    drop(server1);
    wait_until_down(&old_url).await;
    let (_orch2, server2) = start_server(config_dir).await;

    // The bridge must recover on its own: re-read server.json, replay the
    // handshake, and answer with the client's own request id.
    client.send(&tools_list(3)).await;
    let response = client.recv().await;
    assert_eq!(response["id"], 3);
    assert!(
        response["result"]["tools"].is_array(),
        "recovered across restart: {response}"
    );
    server2.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn waits_for_podium_when_started_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_dir = dir.path().join("mcp");
    // No server yet — server.json does not even exist.
    let mut client = spawn_bridge(config_dir.join("server.json"));

    client.send(INITIALIZE).await;
    sleep(Duration::from_millis(100)).await; // bridge is now retrying
    let (_orch, server) = start_server(config_dir).await;

    let response = client.recv().await;
    assert_eq!(response["id"], 1);
    assert!(
        response["result"]["protocolVersion"].is_string(),
        "initialize answered once Podium came up: {response}"
    );
    server.stop();
}
