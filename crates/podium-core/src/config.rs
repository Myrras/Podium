//! Serde types for `podium.yml`, the optional per-project configuration.
//!
//! Parsing is strict (`deny_unknown_fields`) so typos surface as readable
//! errors instead of silently ignored keys. Validation and defaulting into
//! ready-to-run specs happens in [`crate::project`].

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::process::RestartPolicy;

/// Raw shape of a `podium.yml` file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PodiumConfig {
    /// Display name; defaults to the project folder name.
    #[serde(default)]
    pub name: Option<String>,
    /// Sidebar badge initials (max 2 chars); defaults to initials derived
    /// from the effective name.
    #[serde(default)]
    pub icon_initials: Option<String>,
    #[serde(default)]
    pub processes: Vec<ProcessConfig>,
    #[serde(default)]
    pub agents: AgentsConfig,
}

/// One configured (service) process.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub name: String,
    /// Shell command line, run via `$SHELL -lc`.
    pub command: String,
    /// Working directory relative to the project root (defaults to the root).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Start the process as soon as the project opens.
    #[serde(default)]
    pub auto_start: bool,
    /// Supervisor restart policy (`never` | `on-crash` | `always`).
    #[serde(default)]
    pub auto_restart: RestartPolicy,
    /// Extra environment variables for the process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Agent defaults for the project.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsConfig {
    /// The adapter a bare spawn uses in this project. `None` when the project
    /// doesn't pin one, in which case the global Settings → Agents default (or
    /// the built-in fallback) is used. An explicit value here wins over the
    /// global default — a project's own choice is more specific.
    #[serde(default)]
    pub default_adapter: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_config_parses() {
        let yml = r#"
name: Webshop
icon_initials: WP
processes:
  - name: web
    command: pnpm dev
    cwd: apps/web
    auto_start: true
    auto_restart: on-crash
    env: { PORT: "3000" }
agents:
  default_adapter: claude-code
  extra_args: ["--verbose"]
"#;
        let cfg: PodiumConfig = serde_yaml_ng::from_str(yml).unwrap();
        assert_eq!(cfg.name.as_deref(), Some("Webshop"));
        assert_eq!(cfg.icon_initials.as_deref(), Some("WP"));
        assert_eq!(cfg.processes.len(), 1);
        let p = &cfg.processes[0];
        assert_eq!(p.name, "web");
        assert_eq!(p.command, "pnpm dev");
        assert_eq!(p.cwd.as_deref(), Some("apps/web"));
        assert!(p.auto_start);
        assert_eq!(p.auto_restart, RestartPolicy::OnCrash);
        assert_eq!(p.env.get("PORT").map(String::as_str), Some("3000"));
        assert_eq!(cfg.agents.default_adapter.as_deref(), Some("claude-code"));
        assert_eq!(cfg.agents.extra_args, vec!["--verbose"]);
    }

    #[test]
    fn minimal_process_gets_defaults() {
        let yml = "processes:\n  - name: web\n    command: pnpm dev\n";
        let cfg: PodiumConfig = serde_yaml_ng::from_str(yml).unwrap();
        assert!(cfg.name.is_none());
        assert!(cfg.icon_initials.is_none());
        let p = &cfg.processes[0];
        assert!(p.cwd.is_none());
        assert!(!p.auto_start);
        assert_eq!(p.auto_restart, RestartPolicy::Never);
        assert!(p.env.is_empty());
        // Unset in yml → None (the global/ built-in default applies at spawn).
        assert_eq!(cfg.agents.default_adapter, None);
        assert!(cfg.agents.extra_args.is_empty());
    }

    #[test]
    fn empty_document_is_a_valid_config() {
        let cfg: PodiumConfig = serde_yaml_ng::from_str("{}").unwrap();
        assert!(cfg.processes.is_empty());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = serde_yaml_ng::from_str::<PodiumConfig>("nmae: Webshop\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("nmae"), "error should name the bad key: {err}");
    }

    #[test]
    fn unknown_restart_policy_is_rejected() {
        let yml = "processes:\n  - name: web\n    command: x\n    auto_restart: sometimes\n";
        assert!(serde_yaml_ng::from_str::<PodiumConfig>(yml).is_err());
    }
}
