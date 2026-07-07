//! The stdio ↔ streamable-HTTP proxy behind `podium mcp-bridge`.
//!
//! External MCP clients cannot point at the built-in server directly: its
//! port and bearer token rotate on every app launch. Instead they launch
//! this bridge (`{"command": "…/Podium", "args": ["mcp-bridge"]}`) — a
//! config line that never goes stale. The bridge reads the current URL +
//! token from the 0600 `server.json` the server writes on startup, relays
//! newline-delimited JSON-RPC from stdin as HTTP POSTs (unwrapping SSE
//! response bodies), and reconnects transparently when Podium restarts:
//! it re-reads `server.json` with backoff and replays the client's
//! `initialize` handshake so the client never notices.
//!
//! ## Secrets
//! The token travels only from `server.json` to the `Authorization` header;
//! it is never logged and never appears on the client-facing stdio stream.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::header;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::time::Instant;

use crate::error::{CoreError, CoreResult};

/// Retry timings, injectable so tests run in milliseconds.
#[derive(Debug, Clone)]
pub struct BridgeTiming {
    /// Delay before the first retry when Podium is unreachable.
    pub initial_backoff: Duration,
    /// Cap for the exponential (doubling) backoff.
    pub max_backoff: Duration,
    /// Total time to keep retrying one message before answering it with a
    /// JSON-RPC error.
    pub give_up_after: Duration,
}

impl Default for BridgeTiming {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(5),
            give_up_after: Duration::from_secs(120),
        }
    }
}

/// Run the bridge over real stdin/stdout until the client closes stdin.
pub async fn run_stdio(server_json: PathBuf) -> CoreResult<()> {
    run_bridge(
        tokio::io::stdin(),
        tokio::io::stdout(),
        server_json,
        BridgeTiming::default(),
    )
    .await
}

/// Relay `input` (client → server) and `output` (server → client) through
/// the MCP server described by `server_json`. Messages are handled one at a
/// time — MCP clients tolerate delayed responses, and serialising keeps the
/// reconnect/replay logic straightforward.
pub async fn run_bridge<R, W>(
    input: R,
    output: W,
    server_json: PathBuf,
    timing: BridgeTiming,
) -> CoreResult<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CoreError::Config(format!("http client: {e}")))?;
    let mut bridge = Bridge {
        http,
        server_json,
        timing,
        connect: None,
        session_id: None,
        protocol_version: None,
        init_request: None,
        initialized_note: None,
        needs_reinit: false,
        out: output,
    };
    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        // Framing is one JSON-RPC message per line; skip anything else.
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        bridge.handle(msg).await?;
    }
    Ok(())
}

/// Contents of `server.json` (written by [`super::start`]).
#[derive(Clone, serde::Deserialize)]
struct ConnectInfo {
    url: String,
    token: String,
}

enum DeliverError {
    /// Transient — Podium may be restarting; retry with backoff.
    Retry,
    /// Permanent for this message — answer it with a JSON-RPC error.
    Fail(String),
}

struct Bridge<W> {
    http: reqwest::Client,
    server_json: PathBuf,
    timing: BridgeTiming,
    /// Cached contents of `server.json`; dropped on connection failure so
    /// the next attempt picks up a restarted server's URL + token.
    connect: Option<ConnectInfo>,
    /// `Mcp-Session-Id` issued by the current server instance.
    session_id: Option<String>,
    /// Negotiated protocol version, echoed as `MCP-Protocol-Version`.
    protocol_version: Option<String>,
    /// The client's `initialize` request, kept for replay after a restart.
    init_request: Option<Value>,
    /// The client's `notifications/initialized`, replayed likewise.
    initialized_note: Option<Value>,
    /// The old session died with its server — redo the handshake first.
    needs_reinit: bool,
    out: W,
}

impl<W: AsyncWrite + Unpin> Bridge<W> {
    /// Relay one client message, retrying with backoff while Podium is
    /// unreachable. Errors returned here are stdio errors (client gone).
    async fn handle(&mut self, msg: Value) -> std::io::Result<()> {
        let is_initialize = msg["method"] == "initialize";
        if is_initialize {
            // A (re-)initialize starts a fresh session; cache it for replay.
            self.session_id = None;
            self.needs_reinit = false;
            self.init_request = Some(msg.clone());
        } else if msg["method"] == "notifications/initialized" {
            self.initialized_note = Some(msg.clone());
        }

        let deadline = Instant::now() + self.timing.give_up_after;
        let mut backoff = self.timing.initial_backoff;
        loop {
            match self.try_deliver(&msg, is_initialize).await {
                Ok(responses) => {
                    for response in &responses {
                        self.emit(response).await?;
                    }
                    return Ok(());
                }
                Err(DeliverError::Fail(reason)) => return self.fail(&msg, &reason).await,
                Err(DeliverError::Retry) => {
                    if Instant::now() >= deadline {
                        return self.fail(&msg, "Podium is not reachable").await;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.timing.max_backoff);
                }
            }
        }
    }

    async fn try_deliver(
        &mut self,
        msg: &Value,
        is_initialize: bool,
    ) -> Result<Vec<Value>, DeliverError> {
        let connect = self.connect_info()?;
        if self.needs_reinit && !is_initialize {
            self.replay_handshake(&connect).await?;
        }
        self.post(&connect, msg).await
    }

    /// Current URL + token, (re)read from `server.json` on demand.
    fn connect_info(&mut self) -> Result<ConnectInfo, DeliverError> {
        if self.connect.is_none() {
            let raw =
                std::fs::read_to_string(&self.server_json).map_err(|_| DeliverError::Retry)?;
            self.connect = serde_json::from_str(&raw).ok();
        }
        self.connect.clone().ok_or(DeliverError::Retry)
    }

    /// The old session died with the previous server instance. Re-run the
    /// client's cached handshake against the new one so the client never
    /// notices the restart — the replay uses a private request id and its
    /// response is swallowed, so the client's own ids are untouched.
    async fn replay_handshake(&mut self, connect: &ConnectInfo) -> Result<(), DeliverError> {
        let Some(mut init) = self.init_request.clone() else {
            return Ok(());
        };
        init["id"] = json!("podium-bridge-reinit");
        self.post(connect, &init).await?;
        let note = self
            .initialized_note
            .clone()
            .unwrap_or_else(|| json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        self.post(connect, &note).await?;
        self.needs_reinit = false;
        Ok(())
    }

    /// One POST to the server; returns the JSON-RPC messages it produced
    /// (empty for accepted notifications; SSE bodies are unwrapped).
    async fn post(
        &mut self,
        connect: &ConnectInfo,
        msg: &Value,
    ) -> Result<Vec<Value>, DeliverError> {
        let url = format!("{}/mcp", connect.url.trim_end_matches('/'));
        let mut request = self
            .http
            .post(&url)
            .bearer_auth(&connect.token)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .body(msg.to_string());
        if let Some(session) = &self.session_id {
            request = request.header("mcp-session-id", session.clone());
        }
        if let Some(version) = &self.protocol_version {
            request = request.header("mcp-protocol-version", version.clone());
        }
        // A send error means the request never reached a server, so
        // retrying cannot double-execute anything.
        let response = request.send().await.map_err(|_| self.server_lost())?;

        match response.status().as_u16() {
            200 => {}
            202 => return Ok(Vec::new()),
            // The token rotated (a new app run) — re-read `server.json`.
            401 | 403 => return Err(self.server_lost()),
            // Our session is gone (server restarted behind a live file).
            400 | 404 if self.session_id.take().is_some() => {
                self.needs_reinit = self.init_request.is_some();
                return Err(DeliverError::Retry);
            }
            status => {
                return Err(DeliverError::Fail(format!(
                    "Podium's MCP server answered HTTP {status}"
                )))
            }
        }

        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(session.to_string());
        }
        let is_sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.starts_with("text/event-stream"));
        // The server ends the body once the response message is delivered,
        // so collecting it is finite. A drop mid-body is not retryable: the
        // request may already have executed.
        let body = response
            .bytes()
            .await
            .map_err(|_| DeliverError::Fail("response from Podium was interrupted".to_string()))?;
        let messages = if is_sse {
            parse_sse(&String::from_utf8_lossy(&body))
        } else {
            serde_json::from_slice::<Value>(&body)
                .map(|v| vec![v])
                .unwrap_or_default()
        };
        // Remember the negotiated version (from the `initialize` result) to
        // echo as `MCP-Protocol-Version` on subsequent requests.
        for message in &messages {
            if let Some(version) = message["result"]["protocolVersion"].as_str() {
                self.protocol_version = Some(version.to_string());
            }
        }
        Ok(messages)
    }

    /// Drop cached state so the next attempt re-reads `server.json` and,
    /// if a session had been established, redoes the handshake.
    fn server_lost(&mut self) -> DeliverError {
        self.connect = None;
        if self.session_id.take().is_some() {
            self.needs_reinit = self.init_request.is_some();
        }
        DeliverError::Retry
    }

    async fn emit(&mut self, msg: &Value) -> std::io::Result<()> {
        self.out.write_all(msg.to_string().as_bytes()).await?;
        self.out.write_all(b"\n").await?;
        self.out.flush().await
    }

    /// Answer an undeliverable request with a JSON-RPC error; notifications
    /// and client responses get no reply, per spec.
    async fn fail(&mut self, msg: &Value, reason: &str) -> std::io::Result<()> {
        if msg["id"].is_null() || msg["method"].is_null() {
            return Ok(());
        }
        self.emit(&json!({
            "jsonrpc": "2.0",
            "id": msg["id"],
            "error": { "code": -32000, "message": format!("podium mcp-bridge: {reason}") },
        }))
        .await
    }
}

/// Extract the `data:` payloads from a complete SSE body.
fn parse_sse(body: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut data = String::new();
    for line in body.lines().chain(std::iter::once("")) {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        } else if line.is_empty() && !data.is_empty() {
            if let Ok(value) = serde_json::from_str(&data) {
                messages.push(value);
            }
            data.clear();
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::parse_sse;
    use serde_json::json;

    #[test]
    fn parses_data_events_and_ignores_other_fields() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n: keepalive\n\ndata: {\"a\":1}\n";
        assert_eq!(
            parse_sse(body),
            vec![
                json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
                json!({ "a": 1 }),
            ]
        );
    }

    #[test]
    fn joins_multi_line_data() {
        let body = "data: {\"a\":\ndata: 1}\n\n";
        assert_eq!(parse_sse(body), vec![json!({ "a": 1 })]);
    }
}
