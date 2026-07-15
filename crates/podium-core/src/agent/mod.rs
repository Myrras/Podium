//! Agent adapter abstraction: turns "spawn an agent in this project" into a
//! concrete shell command line + environment (a [`LaunchPlan`]).
//!
//! Adapters are pure planners — the core PTY machinery does the spawning
//! (`$SHELL -lic "<command>"`), so an adapter only decides what the command
//! line and environment look like. The MCP seam ([`McpConnectInfo`]) is
//! designed in now but stays `None` until the built-in MCP server lands.

pub mod settings;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::CoreResult;
use crate::ids::{ProcessId, ProjectId};

/// Everything an adapter may need to build a [`LaunchPlan`].
pub struct AgentLaunchCtx<'a> {
    pub project_id: ProjectId,
    pub process_id: ProcessId,
    pub project_root: &'a Path,
    /// Initial prompt, passed as a positional argument when supported.
    pub prompt: Option<&'a str>,
    /// CLI args for the launch: the global default args and the project's
    /// `agents.extra_args`, already combined per the user's merge mode.
    pub extra_args: &'a [String],
    /// Global command override; replaces the adapter's built-in binary when
    /// set (from Settings → Agents). `None` = use [`AgentAdapter::binary`].
    pub command_override: Option<&'a str>,
    /// How to reach Podium's MCP server; `None` until phase 6 provides it.
    pub mcp: Option<&'a McpConnectInfo>,
    /// Whether the process already got a descriptive window name (explicit,
    /// to-do or prompt-derived). When false, the launch plan tells the agent
    /// in its system prompt to rename itself right after the first user
    /// message — models never act on the buried MCP-server instruction alone.
    pub named: bool,
}

/// Connection details for the built-in MCP server.
#[derive(Clone)]
pub struct McpConnectInfo {
    pub url: String,
    /// Bearer token — never logged; only written to a 0600 config file.
    pub token: String,
    /// Directory where per-agent MCP config files are written.
    pub config_dir: PathBuf,
}

// Manual impl so an accidental `{:?}` can never leak the token.
impl fmt::Debug for McpConnectInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpConnectInfo")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .field("config_dir", &self.config_dir)
            .finish()
    }
}

/// A ready-to-run launch: a full shell command line (run via `$SHELL -lic`)
/// plus extra environment variables for the process.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub command: String,
    pub env: Vec<(String, String)>,
}

/// One supported agent CLI (Claude Code today; more later).
pub trait AgentAdapter: Send + Sync {
    /// Stable identifier used in config and over IPC (e.g. `"claude-code"`).
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    /// The CLI binary the adapter drives (also the default name prefix).
    fn binary(&self) -> &'static str;
    fn build_launch(&self, ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan>;

    /// Whether the binary resolves on the user's login-shell `PATH`. Probed
    /// via the login shell (`command -v <binary>` on Unix, `where` on Windows)
    /// so shell-profile PATH edits (nvm, homebrew, …) are honoured; output is
    /// discarded, only the exit status matters.
    fn is_available(&self) -> bool {
        crate::platform::run_shell_ok(&crate::platform::command_exists_query(self.binary()))
    }
}

/// A data-driven [`AgentAdapter`]. Claude Code and Auggie both take the prompt
/// as a positional arg and consume the same `--mcp-config <file>` shape, so the
/// launch plan is identical — they differ only in the binary they drive. One
/// struct with three fields covers both; add another CLI by adding a `const`.
#[derive(Clone, Copy)]
pub struct CliAdapter {
    pub id: &'static str,
    pub display_name: &'static str,
    /// The CLI binary the adapter drives (also the default name prefix).
    pub binary: &'static str,
    /// CLI flag that appends text to the agent's system prompt; used to hand
    /// the agent its Podium identity (see [`identity_prompt`]). `None` when
    /// the CLI has no such flag.
    pub system_prompt_flag: Option<&'static str>,
}

/// The Claude Code CLI (`claude`).
pub const CLAUDE_CODE: CliAdapter = CliAdapter {
    id: "claude-code",
    display_name: "Claude Code",
    binary: "claude",
    system_prompt_flag: Some("--append-system-prompt"),
};

/// The Augment / Auggie CLI (`auggie`).
pub const AUGGIE: CliAdapter = CliAdapter {
    id: "auggie",
    display_name: "Auggie",
    binary: "auggie",
    // ponytail: no known system-prompt flag; add one when Auggie grows it.
    system_prompt_flag: None,
};

impl AgentAdapter for CliAdapter {
    fn id(&self) -> &'static str {
        self.id
    }

    fn display_name(&self) -> &'static str {
        self.display_name
    }

    fn binary(&self) -> &'static str {
        self.binary
    }

    fn build_launch(&self, ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
        let bin = ctx.command_override.unwrap_or(self.binary);
        let mut args: Vec<String> = vec![crate::platform::quote_arg(bin)?];
        if let Some(prompt) = ctx.prompt {
            args.push(crate::platform::quote_arg(prompt)?);
        }
        for arg in ctx.extra_args {
            args.push(crate::platform::quote_arg(arg)?);
        }
        if let Some(mcp) = ctx.mcp {
            let path = write_mcp_config(mcp, ctx.process_id)?;
            args.push("--mcp-config".to_string());
            args.push(crate::platform::quote_arg(&path.to_string_lossy())?);
            // Identity only makes sense alongside the Podium MCP tools.
            if let Some(flag) = self.system_prompt_flag {
                args.push(flag.to_string());
                args.push(crate::platform::quote_arg(&identity_prompt(ctx))?);
            }
        }
        Ok(LaunchPlan {
            command: args.join(" "),
            env: vec![
                ("PODIUM_PROJECT_ID".to_string(), ctx.project_id.to_string()),
                ("PODIUM_PROCESS_ID".to_string(), ctx.process_id.to_string()),
            ],
        })
    }
}

/// System-prompt blurb handing the agent its Podium identity. The MCP tools
/// that act on the calling agent (`rename_session`, `assign_todo`) need its
/// process id, but models don't fish `PODIUM_PROCESS_ID` out of the
/// environment or act on the MCP server instructions on their own — so spell
/// the ids out in-context, and tell a generically named session outright to
/// rename itself.
fn identity_prompt(ctx: &AgentLaunchCtx) -> String {
    let mut prompt = format!(
        "You run as a Podium-managed agent: your process_id is {} and your \
         project_id is {}. Use these ids for Podium MCP tools that ask for \
         them (rename_session, assign_todo).",
        ctx.process_id, ctx.project_id
    );
    if !ctx.named {
        prompt.push_str(
            " This session has a generic window name: immediately after the \
             user's first message, call the podium rename_session tool with a \
             short name describing the session, then continue with the task.",
        );
    }
    prompt
}

/// Write `contents` to `path` with owner-only permissions (0600) — used for
/// every file that carries the MCP bearer token.
#[cfg(unix)]
pub(crate) fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// Write the per-agent MCP client config and return its path. The file carries
/// the bearer token, so it is written 0600 (via [`write_private`]). Claude Code
/// and Auggie both consume the same `--mcp-config <file>` shape, so this is
/// shared across those adapters.
pub(crate) fn write_mcp_config(mcp: &McpConnectInfo, process_id: ProcessId) -> CoreResult<PathBuf> {
    let url = if mcp.url.ends_with("/mcp") {
        mcp.url.clone()
    } else {
        format!("{}/mcp", mcp.url.trim_end_matches('/'))
    };
    let config = serde_json::json!({
        "mcpServers": {
            "podium": {
                "type": "http",
                "url": url,
                "headers": { "Authorization": format!("Bearer {}", mcp.token) },
            }
        }
    });
    fs::create_dir_all(&mcp.config_dir)?;
    let path = mcp.config_dir.join(format!("agent-{process_id}.json"));
    write_private(&path, &config.to_string())?;
    Ok(path)
}

/// Serializable adapter listing for UI pickers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub id: String,
    pub display_name: String,
    /// The adapter's built-in CLI binary (the default when no command
    /// override is set); shown as the placeholder in Settings → Agents.
    pub binary: String,
    pub available: bool,
}

/// How long a probed availability snapshot stays fresh. Each probe spawns a
/// login+interactive shell per adapter, which is slow; the UI re-lists on every
/// modal open, so without a cache that cost is paid on every open. A newly
/// installed/removed CLI is picked up within this window (or on restart).
const AVAILABILITY_TTL: Duration = Duration::from_secs(60);

/// Cached availability snapshot: when it was probed + the resulting infos.
type AvailabilityCache = Arc<Mutex<Option<(Instant, Vec<AdapterInfo>)>>>;

/// The set of adapters an [`crate::Orchestrator`] can spawn. Injectable so
/// tests can register fakes; defaults to the real registry.
#[derive(Clone)]
pub struct AdapterRegistry {
    adapters: Vec<Arc<dyn AgentAdapter>>,
    /// Shared across clones so every caller hits the same cache. `None` until
    /// the first probe.
    cache: AvailabilityCache,
}

impl AdapterRegistry {
    pub fn new(adapters: Vec<Arc<dyn AgentAdapter>>) -> Self {
        Self {
            adapters,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<dyn AgentAdapter>> {
        self.adapters.iter().find(|a| a.id() == id).cloned()
    }

    /// Snapshot for listing; probes each adapter's binary availability, cached
    /// for [`AVAILABILITY_TTL`] so rapid re-lists don't re-shell every time.
    pub fn infos(&self) -> Vec<AdapterInfo> {
        let mut cache = self.cache.lock().expect("adapter cache mutex poisoned");
        if let Some((at, infos)) = cache.as_ref() {
            if at.elapsed() < AVAILABILITY_TTL {
                return infos.clone();
            }
        }
        let infos: Vec<AdapterInfo> = self
            .adapters
            .iter()
            .map(|a| AdapterInfo {
                id: a.id().to_string(),
                display_name: a.display_name().to_string(),
                binary: a.binary().to_string(),
                available: a.is_available(),
            })
            .collect();
        *cache = Some((Instant::now(), infos.clone()));
        infos
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new(vec![Arc::new(CLAUDE_CODE), Arc::new(AUGGIE)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Round-trips a built [`LaunchPlan::command`] back into argv, using
    /// whichever tokenizer matches the shell `login_shell()` actually runs it
    /// through on this OS — `shlex` for `$SHELL -lc`, the Windows argv parser
    /// for `cmd.exe /C`.
    fn split_command(cmd: &str) -> Vec<String> {
        #[cfg(unix)]
        {
            shlex::split(cmd).expect("valid shell line")
        }
        #[cfg(windows)]
        {
            crate::platform::parse_windows_argv(cmd)
        }
    }

    #[test]
    fn infos_caches_the_availability_probe() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingAdapter(Arc<AtomicUsize>);
        impl AgentAdapter for CountingAdapter {
            fn id(&self) -> &'static str {
                "counting"
            }
            fn display_name(&self) -> &'static str {
                "Counting"
            }
            fn binary(&self) -> &'static str {
                "counting"
            }
            fn build_launch(&self, _ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
                unreachable!("not used in this test")
            }
            fn is_available(&self) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                true
            }
        }

        let probes = Arc::new(AtomicUsize::new(0));
        let registry = AdapterRegistry::new(vec![Arc::new(CountingAdapter(Arc::clone(&probes)))]);
        let first = registry.infos();
        let second = registry.infos();
        assert_eq!(first.len(), 1);
        assert!(second[0].available);
        assert_eq!(
            probes.load(Ordering::SeqCst),
            1,
            "second list within the TTL must reuse the cache, not re-probe"
        );
    }

    #[test]
    fn default_registry_exposes_claude_and_auggie() {
        let registry = AdapterRegistry::default();
        assert!(registry.by_id("claude-code").is_some());
        assert!(registry.by_id("auggie").is_some());

        let ids: Vec<String> = registry.infos().into_iter().map(|i| i.id).collect();
        assert!(ids.contains(&"claude-code".to_string()));
        assert!(ids.contains(&"auggie".to_string()));
    }

    fn plan(
        prompt: Option<&str>,
        extra_args: &[String],
        mcp: Option<&McpConnectInfo>,
    ) -> (LaunchPlan, ProjectId, ProcessId) {
        plan_with_override(prompt, extra_args, None, mcp)
    }

    fn plan_with_override(
        prompt: Option<&str>,
        extra_args: &[String],
        command_override: Option<&str>,
        mcp: Option<&McpConnectInfo>,
    ) -> (LaunchPlan, ProjectId, ProcessId) {
        let project_id = ProjectId::new();
        let process_id = ProcessId::new();
        let ctx = AgentLaunchCtx {
            project_id,
            process_id,
            project_root: Path::new("/tmp"),
            prompt,
            extra_args,
            command_override,
            mcp,
            named: false,
        };
        let plan = CLAUDE_CODE.build_launch(&ctx).expect("build_launch");
        (plan, project_id, process_id)
    }

    #[test]
    fn bare_launch_is_just_the_binary() {
        let (plan, _, _) = plan(None, &[], None);
        assert_eq!(plan.command, "claude");
    }

    #[test]
    fn prompt_and_extra_args_are_shell_quoted() {
        let prompt = r#"fix the "login" bug; $HOME"#;
        let extra = vec!["--verbose".to_string(), "two words".to_string()];
        let (plan, _, _) = plan(Some(prompt), &extra, None);
        // Round-trip through a shell tokenizer: quoting must preserve args.
        let tokens = split_command(&plan.command);
        assert_eq!(tokens, vec!["claude", prompt, "--verbose", "two words"]);
    }

    #[test]
    fn prompt_with_apostrophe_survives_intact() {
        // Regression: a to-do-assignment prompt starting "You're the
        // assigned agent…" must arrive as one argument, not get truncated at
        // the apostrophe by whichever shell `login_shell()` runs it through.
        let prompt = "You're the assigned agent for to-do 'Fix login bug'";
        let (plan, _, _) = plan(Some(prompt), &[], None);
        let tokens = split_command(&plan.command);
        assert_eq!(tokens, vec!["claude", prompt]);
    }

    #[test]
    fn command_override_replaces_the_binary() {
        let (plan, _, _) = plan_with_override(None, &[], Some("/opt/bin/claude"), None);
        let tokens = split_command(&plan.command);
        assert_eq!(tokens, vec!["/opt/bin/claude"]);
    }

    #[test]
    fn env_identifies_project_and_process() {
        let (plan, project_id, process_id) = plan(None, &[], None);
        assert!(plan
            .env
            .contains(&("PODIUM_PROJECT_ID".to_string(), project_id.to_string())));
        assert!(plan
            .env
            .contains(&("PODIUM_PROCESS_ID".to_string(), process_id.to_string())));
    }

    #[test]
    fn mcp_config_file_has_exact_shape_and_is_referenced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mcp = McpConnectInfo {
            url: "http://127.0.0.1:39217".to_string(),
            token: "sekret-token".to_string(),
            config_dir: dir.path().to_path_buf(),
        };
        let (plan, _, process_id) = plan(Some("hello"), &[], Some(&mcp));

        let path = dir.path().join(format!("agent-{process_id}.json"));
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("config written"))
                .expect("valid json");
        assert_eq!(
            written,
            serde_json::json!({
                "mcpServers": {
                    "podium": {
                        "type": "http",
                        "url": "http://127.0.0.1:39217/mcp",
                        "headers": { "Authorization": "Bearer sekret-token" },
                    }
                }
            })
        );

        let tokens = split_command(&plan.command);
        assert_eq!(tokens.len(), 6, "expected identity prompt appended");
        assert_eq!(
            tokens[..4],
            [
                "claude".to_string(),
                "hello".to_string(),
                "--mcp-config".to_string(),
                path.to_string_lossy().into_owned(),
            ]
        );
        assert_eq!(tokens[4], "--append-system-prompt");
        assert!(tokens[5].contains(&process_id.to_string()));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "config must be private");
        }
    }

    #[test]
    fn url_already_ending_in_mcp_is_kept_as_given() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mcp = McpConnectInfo {
            url: "http://localhost:1234/mcp".to_string(),
            token: "t".to_string(),
            config_dir: dir.path().to_path_buf(),
        };
        let (_, _, process_id) = plan(None, &[], Some(&mcp));
        let path = dir.path().join(format!("agent-{process_id}.json"));
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("config written"))
                .expect("valid json");
        assert_eq!(
            written["mcpServers"]["podium"]["url"],
            "http://localhost:1234/mcp"
        );
    }

    #[test]
    fn auggie_drives_its_own_binary() {
        let ctx = AgentLaunchCtx {
            project_id: ProjectId::new(),
            process_id: ProcessId::new(),
            project_root: Path::new("/tmp"),
            prompt: None,
            extra_args: &[],
            command_override: None,
            mcp: None,
            named: false,
        };
        assert_eq!(
            AUGGIE.build_launch(&ctx).expect("build_launch").command,
            "auggie"
        );
    }

    #[test]
    fn identity_prompt_nudges_unnamed_agents_to_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mcp = McpConnectInfo {
            url: "http://127.0.0.1:1".to_string(),
            token: "t".to_string(),
            config_dir: dir.path().to_path_buf(),
        };
        let (plan, project_id, process_id) = plan(None, &[], Some(&mcp));
        let tokens = split_command(&plan.command);
        let blurb = &tokens[tokens.len() - 1];
        assert!(blurb.contains(&process_id.to_string()));
        assert!(blurb.contains(&project_id.to_string()));
        assert!(blurb.contains("rename_session"));
        assert!(blurb.contains("generic window name"));
    }

    #[test]
    fn named_agents_get_identity_but_no_rename_nudge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mcp = McpConnectInfo {
            url: "http://127.0.0.1:1".to_string(),
            token: "t".to_string(),
            config_dir: dir.path().to_path_buf(),
        };
        let ctx = AgentLaunchCtx {
            project_id: ProjectId::new(),
            process_id: ProcessId::new(),
            project_root: Path::new("/tmp"),
            prompt: None,
            extra_args: &[],
            command_override: None,
            mcp: Some(&mcp),
            named: true,
        };
        let plan = CLAUDE_CODE.build_launch(&ctx).expect("build_launch");
        let tokens = split_command(&plan.command);
        let blurb = &tokens[tokens.len() - 1];
        assert!(blurb.contains(&ctx.process_id.to_string()));
        assert!(!blurb.contains("generic window name"));
    }

    #[test]
    fn adapters_without_system_prompt_flag_skip_the_identity_prompt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mcp = McpConnectInfo {
            url: "http://127.0.0.1:1".to_string(),
            token: "t".to_string(),
            config_dir: dir.path().to_path_buf(),
        };
        let ctx = AgentLaunchCtx {
            project_id: ProjectId::new(),
            process_id: ProcessId::new(),
            project_root: Path::new("/tmp"),
            prompt: None,
            extra_args: &[],
            command_override: None,
            mcp: Some(&mcp),
            named: false,
        };
        let plan = AUGGIE.build_launch(&ctx).expect("build_launch");
        assert!(!plan.command.contains("--append-system-prompt"));
    }
}
