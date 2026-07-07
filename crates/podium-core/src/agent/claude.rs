//! Adapter for the Claude Code CLI (`claude`).

use std::borrow::Cow;

use crate::error::{CoreError, CoreResult};

use super::{write_mcp_config, AgentAdapter, AgentLaunchCtx, LaunchPlan};

/// Plans launches of the `claude` CLI.
pub struct ClaudeCodeAdapter;

impl AgentAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn binary(&self) -> &'static str {
        "claude"
    }

    fn build_launch(&self, ctx: &AgentLaunchCtx) -> CoreResult<LaunchPlan> {
        let bin = ctx.command_override.unwrap_or(self.binary());
        let mut args: Vec<String> = vec![quote(bin)?];
        if let Some(prompt) = ctx.prompt {
            args.push(quote(prompt)?);
        }
        for arg in ctx.extra_args {
            args.push(quote(arg)?);
        }
        if let Some(mcp) = ctx.mcp {
            let path = write_mcp_config(mcp, ctx.process_id)?;
            args.push("--mcp-config".to_string());
            args.push(quote(&path.to_string_lossy())?);
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

fn quote(arg: &str) -> CoreResult<String> {
    shlex::try_quote(arg)
        .map(Cow::into_owned)
        .map_err(|e| CoreError::InvalidInput(format!("cannot quote argument: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::McpConnectInfo;
    use crate::ids::{ProcessId, ProjectId};
    use std::fs;
    use std::path::Path;

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
        };
        let plan = ClaudeCodeAdapter.build_launch(&ctx).expect("build_launch");
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
        let tokens = shlex::split(&plan.command).expect("valid shell line");
        assert_eq!(tokens, vec!["claude", prompt, "--verbose", "two words"]);
    }

    #[test]
    fn command_override_replaces_the_binary() {
        let (plan, _, _) = plan_with_override(None, &[], Some("/opt/bin/claude"), None);
        let tokens = shlex::split(&plan.command).expect("valid shell line");
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

        let tokens = shlex::split(&plan.command).expect("valid shell line");
        assert_eq!(
            tokens,
            vec![
                "claude".to_string(),
                "hello".to_string(),
                "--mcp-config".to_string(),
                path.to_string_lossy().into_owned(),
            ]
        );

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
}
