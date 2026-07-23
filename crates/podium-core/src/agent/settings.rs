//! Global, cross-project agent configuration, persisted as one JSON file in
//! the app data dir.
//!
//! Where `podium.yml` sets per-project agent args, this store holds the
//! **global** defaults the user manages from Settings → Agents: a per-adapter
//! command override + default CLI arguments, plus how those defaults combine
//! with a project's `agents.extra_args` ([`MergeMode`]). It is applied
//! server-side in [`crate::Orchestrator::spawn_agent`], so every caller (the
//! UI and agents spawning over MCP) gets the same treatment.
//!
//! Writes are atomic (temp file + rename); a missing or corrupt file is
//! treated as defaults. Until [`AgentSettingsStore::set_path`] is called the
//! store is in-memory only (tests, or a data dir that failed to resolve).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

const LOCK_POISONED: &str = "agent settings store lock poisoned";

/// How global default args combine with a project's `agents.extra_args`.
/// Serialized kebab-case to match the `RestartPolicy` convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeMode {
    /// Global defaults first, then the project's args (project can override).
    #[default]
    Merge,
    /// Use the project's args when it has any; otherwise the global defaults.
    ProjectOverrides,
    /// Use the global defaults when set; otherwise the project's args.
    GlobalOverrides,
}

/// Per-adapter global overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterOverride {
    /// Replacement command/binary; `None` = the adapter's built-in one.
    #[serde(default)]
    pub command: Option<String>,
    /// Default CLI arguments, combined with project args per [`MergeMode`].
    #[serde(default)]
    pub default_args: Vec<String>,
}

/// The whole persisted document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default)]
    pub merge_mode: MergeMode,
    /// The adapter a bare spawn uses across all projects, unless the project
    /// pins its own in `podium.yml`. `None` = fall back to the built-in
    /// default. Set from Settings → Agents.
    #[serde(default)]
    pub default_adapter: Option<String>,
    /// Overrides keyed by adapter id (e.g. `"claude-code"`).
    #[serde(default)]
    pub overrides: BTreeMap<String, AdapterOverride>,
    /// Whether agents are told to offer an isolated git worktree before
    /// their first code change. Defaults to on (also for a legacy
    /// `agents.json` written before this field existed).
    #[serde(default = "default_true")]
    pub suggest_worktree: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            merge_mode: MergeMode::default(),
            default_adapter: None,
            overrides: BTreeMap::new(),
            suggest_worktree: true,
        }
    }
}

impl AgentSettings {
    /// The override configured for `adapter_id`, if any.
    pub fn override_for(&self, adapter_id: &str) -> Option<&AdapterOverride> {
        self.overrides.get(adapter_id)
    }
}

/// Combine global default args with a project's args per `mode`.
pub fn merge_args(mode: MergeMode, global: &[String], project: &[String]) -> Vec<String> {
    match mode {
        MergeMode::Merge => global.iter().chain(project).cloned().collect(),
        MergeMode::ProjectOverrides => {
            if project.is_empty() {
                global.to_vec()
            } else {
                project.to_vec()
            }
        }
        MergeMode::GlobalOverrides => {
            if global.is_empty() {
                project.to_vec()
            } else {
                global.to_vec()
            }
        }
    }
}

struct Inner {
    path: Option<PathBuf>,
    settings: AgentSettings,
}

/// Thread-safe global agent settings shared by the orchestrator's public API.
pub(crate) struct AgentSettingsStore {
    inner: Mutex<Inner>,
}

impl AgentSettingsStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: None,
                settings: AgentSettings::default(),
            }),
        }
    }

    /// Point the store at its backing file and load whatever is there. Any
    /// in-memory settings accumulated before this call are replaced.
    pub fn set_path(&self, path: PathBuf) {
        let settings = load(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.settings = settings;
    }

    /// A clone of the current settings.
    pub fn get(&self) -> AgentSettings {
        self.inner.lock().expect(LOCK_POISONED).settings.clone()
    }

    /// Set the merge mode; returns the updated settings.
    pub fn set_merge_mode(&self, mode: MergeMode) -> CoreResult<AgentSettings> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.settings.merge_mode = mode;
        save(&inner)?;
        Ok(inner.settings.clone())
    }

    /// Set whether agents should offer a worktree before modifying code;
    /// returns the updated settings.
    pub fn set_suggest_worktree(&self, enabled: bool) -> CoreResult<AgentSettings> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.settings.suggest_worktree = enabled;
        save(&inner)?;
        Ok(inner.settings.clone())
    }

    /// Set (or clear) the global default adapter; returns the updated settings.
    /// A blank id clears it (back to the built-in default).
    pub fn set_default_adapter(&self, adapter_id: Option<String>) -> CoreResult<AgentSettings> {
        let adapter_id = adapter_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.settings.default_adapter = adapter_id;
        save(&inner)?;
        Ok(inner.settings.clone())
    }

    /// Set (or clear) one adapter's override; returns the updated settings.
    /// The command and each arg are trimmed and empties dropped; an override
    /// left with no command and no args is removed so the file stays tidy.
    pub fn set_override(
        &self,
        adapter_id: &str,
        command: Option<String>,
        default_args: Vec<String>,
    ) -> CoreResult<AgentSettings> {
        let command = command
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
        let default_args: Vec<String> = default_args
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if command.is_none() && default_args.is_empty() {
            inner.settings.overrides.remove(adapter_id);
        } else {
            inner.settings.overrides.insert(
                adapter_id.to_string(),
                AdapterOverride {
                    command,
                    default_args,
                },
            );
        }
        save(&inner)?;
        Ok(inner.settings.clone())
    }
}

/// Read the settings from disk; a missing or corrupt file is defaults.
fn load(path: &Path) -> AgentSettings {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return AgentSettings::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("agent settings file is corrupt, starting fresh: {e}");
        AgentSettings::default()
    })
}

/// Atomically persist the settings: write a temp file, then rename it over the
/// old one so a crash mid-write can never leave a torn file. A store without a
/// path (in-memory) is a no-op.
fn save(inner: &Inner) -> CoreResult<()> {
    let Some(path) = &inner.path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&inner.settings)
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode agent settings: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn merge_concatenates_global_then_project() {
        let global = args(&["--model", "opus"]);
        let project = args(&["--verbose"]);
        assert_eq!(
            merge_args(MergeMode::Merge, &global, &project),
            args(&["--model", "opus", "--verbose"])
        );
    }

    #[test]
    fn project_overrides_uses_project_when_present_else_global() {
        let global = args(&["--model", "opus"]);
        assert_eq!(
            merge_args(MergeMode::ProjectOverrides, &global, &args(&["--verbose"])),
            args(&["--verbose"])
        );
        assert_eq!(
            merge_args(MergeMode::ProjectOverrides, &global, &[]),
            global
        );
    }

    #[test]
    fn global_overrides_uses_global_when_present_else_project() {
        let project = args(&["--verbose"]);
        assert_eq!(
            merge_args(
                MergeMode::GlobalOverrides,
                &args(&["--model", "opus"]),
                &project
            ),
            args(&["--model", "opus"])
        );
        assert_eq!(
            merge_args(MergeMode::GlobalOverrides, &[], &project),
            project
        );
    }

    #[test]
    fn settings_json_round_trips() {
        let mut settings = AgentSettings {
            merge_mode: MergeMode::ProjectOverrides,
            ..Default::default()
        };
        settings.overrides.insert(
            "claude-code".to_string(),
            AdapterOverride {
                command: Some("claude".to_string()),
                default_args: args(&["--model", "opus"]),
            },
        );
        let json = serde_json::to_string(&settings).unwrap();
        // Enum values are kebab-case on the wire.
        assert!(json.contains("project-overrides"), "json: {json}");
        let back: AgentSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn set_override_trims_and_drops_empty() {
        let store = AgentSettingsStore::new();
        let updated = store
            .set_override(
                "claude-code",
                Some("  claude  ".to_string()),
                args(&["  --model  ", "opus", "  "]),
            )
            .unwrap();
        let ov = updated.override_for("claude-code").unwrap();
        assert_eq!(ov.command.as_deref(), Some("claude"));
        assert_eq!(ov.default_args, args(&["--model", "opus"]));

        // An empty command + no args removes the override entirely.
        let cleared = store
            .set_override("claude-code", Some("   ".to_string()), vec![])
            .unwrap();
        assert!(cleared.override_for("claude-code").is_none());
    }

    #[test]
    fn set_default_adapter_trims_and_clears() {
        let store = AgentSettingsStore::new();
        let updated = store
            .set_default_adapter(Some("  auggie  ".to_string()))
            .unwrap();
        assert_eq!(updated.default_adapter.as_deref(), Some("auggie"));

        // Blank / None clears it back to the built-in default.
        let cleared = store.set_default_adapter(Some("  ".to_string())).unwrap();
        assert_eq!(cleared.default_adapter, None);
        let cleared = store.set_default_adapter(None).unwrap();
        assert_eq!(cleared.default_adapter, None);
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.json");

        let store = AgentSettingsStore::new();
        store.set_path(file.clone());
        store.set_merge_mode(MergeMode::GlobalOverrides).unwrap();
        store
            .set_default_adapter(Some("auggie".to_string()))
            .unwrap();
        store
            .set_override("claude-code", None, args(&["--permission-mode", "plan"]))
            .unwrap();

        let reloaded = AgentSettingsStore::new();
        reloaded.set_path(file);
        let settings = reloaded.get();
        assert_eq!(settings.merge_mode, MergeMode::GlobalOverrides);
        assert_eq!(settings.default_adapter.as_deref(), Some("auggie"));
        assert_eq!(
            settings.override_for("claude-code").unwrap().default_args,
            args(&["--permission-mode", "plan"])
        );
    }

    #[test]
    fn suggest_worktree_defaults_on_and_persists() {
        assert!(AgentSettings::default().suggest_worktree);
        // A legacy file without the field loads as enabled.
        let legacy: AgentSettings = serde_json::from_str(r#"{"mergeMode":"merge"}"#).unwrap();
        assert!(legacy.suggest_worktree);

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.json");
        let store = AgentSettingsStore::new();
        store.set_path(file.clone());
        let updated = store.set_suggest_worktree(false).unwrap();
        assert!(!updated.suggest_worktree);

        let reloaded = AgentSettingsStore::new();
        reloaded.set_path(file);
        assert!(!reloaded.get().suggest_worktree);
    }

    #[test]
    fn corrupt_file_is_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.json");
        std::fs::write(&file, "not json").unwrap();

        let store = AgentSettingsStore::new();
        store.set_path(file);
        assert_eq!(store.get(), AgentSettings::default());
    }
}
