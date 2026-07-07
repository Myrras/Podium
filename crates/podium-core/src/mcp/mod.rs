//! The built-in MCP server: rmcp's streamable-HTTP transport nested in an
//! axum router, bound to `127.0.0.1:0` (ephemeral port) behind a per-run
//! bearer token. All rmcp API usage is isolated to this module and
//! [`tools`].
//!
//! ## Secrets
//! The bearer token is generated per app run and lives only in memory and in
//! the 0600 config files under `config_dir` (wiped on every [`start`]): the
//! per-agent client configs and `server.json`, the discovery file for the
//! stdio bridge ([`bridge`]). It is never logged and never crosses the Tauri
//! IPC bridge.

pub mod bridge;
pub mod install;
pub mod tools;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::agent::McpConnectInfo;
use crate::error::CoreResult;
use crate::orchestrator::Orchestrator;
use tools::PodiumTools;

/// Handle to the running MCP server. Dropping it (or calling [`stop`])
/// shuts the server down, terminating all active sessions.
///
/// [`stop`]: McpServer::stop
pub struct McpServer {
    url: String,
    token: String,
    cancel: CancellationToken,
}

impl McpServer {
    /// Base URL, e.g. `http://127.0.0.1:49152` (the endpoint is `/mcp`).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The per-run bearer token. Never log this and never send it to the
    /// frontend; it is exposed only so adapters/tests can authenticate.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Shut the server down (idempotent).
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Start the MCP server on an ephemeral localhost port and wire the
/// orchestrator so subsequently spawned agents receive connection details.
///
/// `config_dir` is where per-agent MCP config files (carrying the token) are
/// written; it is wiped first so stale tokens from previous runs vanish.
pub async fn start(orchestrator: Arc<Orchestrator>, config_dir: PathBuf) -> CoreResult<McpServer> {
    if config_dir.exists() {
        std::fs::remove_dir_all(&config_dir)?;
    }
    std::fs::create_dir_all(&config_dir)?;

    let token = generate_token();
    let cancel = CancellationToken::new();

    let service = StreamableHttpService::new(
        {
            let orchestrator = Arc::clone(&orchestrator);
            move || Ok(PodiumTools::new(Arc::clone(&orchestrator)))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
    );

    let auth = AuthState {
        token: Arc::from(token.as_str()),
    };
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(auth, require_bearer));

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let addr = listener.local_addr()?;
    let url = format!("http://{addr}");

    // Discovery point for the stdio bridge (`podium mcp-bridge`): external
    // MCP clients launch the bridge with a config line that never changes,
    // and the bridge re-reads the current URL + token from this 0600 file on
    // every (re)connect.
    crate::agent::write_private(
        &config_dir.join("server.json"),
        &serde_json::json!({ "url": url, "token": token }).to_string(),
    )?;

    let shutdown = cancel.clone();
    tokio::spawn(async move {
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
        if let Err(e) = served {
            tracing::error!("mcp server terminated: {e}");
        }
    });

    orchestrator.set_mcp_connect_info(McpConnectInfo {
        url: url.clone(),
        token: token.clone(),
        config_dir,
    });
    tracing::info!("mcp server listening on {url}/mcp");

    Ok(McpServer { url, token, cancel })
}

#[derive(Clone)]
struct AuthState {
    token: Arc<str>,
}

/// Reject any request whose `Authorization: Bearer …` does not match the
/// per-run token. Without this, any local process could drive Podium.
async fn require_bearer(
    State(auth): State<AuthState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|presented| constant_time_eq(presented.as_bytes(), auth.token.as_bytes()));
    if authorized {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// 64 hex chars (~244 bits) from two v4 UUIDs — no extra RNG dependency.
fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
