//! Load and validate `podium.yml` into ready-to-run process specs.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::config::{AgentsConfig, PodiumConfig};
use crate::error::{CoreError, CoreResult};
use crate::process::{ProcessKind, ProcessSpec};

/// File name of the optional per-project configuration.
pub const CONFIG_FILE_NAME: &str = "podium.yml";

/// Effective, validated project configuration.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Display name (yml `name`, or the folder name).
    pub name: String,
    /// Badge initials (yml value truncated to 2 chars uppercase, or derived).
    pub icon_initials: String,
    pub processes: Vec<ConfiguredProcess>,
    pub agents: AgentsConfig,
}

/// A config-defined process, resolved into a launchable spec.
#[derive(Debug, Clone)]
pub struct ConfiguredProcess {
    pub spec: ProcessSpec,
    pub auto_start: bool,
}

/// Load `root/podium.yml`. A missing file is `Ok(None)` — a plain folder is
/// a valid project. Parse and validation failures are `CoreError::Config`
/// with a readable message (parse errors include line/column).
pub fn load_project_config(root: &Path) -> CoreResult<Option<ProjectConfig>> {
    let path = root.join(CONFIG_FILE_NAME);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let config: PodiumConfig =
        serde_yaml_ng::from_str(&raw).map_err(|e| CoreError::Config(format!("podium.yml: {e}")))?;

    let name = config
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| folder_name(root));
    let icon_initials = match config
        .icon_initials
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.chars().take(2).collect::<String>().to_uppercase(),
        None => derive_icon_initials(&name),
    };

    let mut seen = HashSet::new();
    let mut processes = Vec::with_capacity(config.processes.len());
    for p in config.processes {
        if p.name.trim().is_empty() {
            return Err(CoreError::Config(
                "podium.yml: process name must not be empty".to_string(),
            ));
        }
        if !seen.insert(p.name.clone()) {
            return Err(CoreError::Config(format!(
                "podium.yml: duplicate process name '{}'",
                p.name
            )));
        }
        if p.command.trim().is_empty() {
            return Err(CoreError::Config(format!(
                "podium.yml: process '{}': command must not be empty",
                p.name
            )));
        }
        let cwd = resolve_cwd(root, p.cwd.as_deref())
            .map_err(|e| CoreError::Config(format!("podium.yml: process '{}': {e}", p.name)))?;
        processes.push(ConfiguredProcess {
            spec: ProcessSpec {
                name: p.name,
                command: p.command,
                cwd,
                env: p.env.into_iter().collect(),
                kind: ProcessKind::Service,
                restart_policy: p.auto_restart,
            },
            auto_start: p.auto_start,
        });
    }

    Ok(Some(ProjectConfig {
        name,
        icon_initials,
        processes,
        agents: config.agents,
    }))
}

/// Resolve `cwd` against the project root, rejecting absolute paths and any
/// traversal (`..`) so a process can never be spawned outside its project.
pub fn resolve_cwd(root: &Path, cwd: Option<&str>) -> CoreResult<PathBuf> {
    let Some(rel) = cwd.filter(|s| !s.is_empty()) else {
        return Ok(root.to_path_buf());
    };
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(CoreError::InvalidInput(
            "cwd must be relative to the project root".to_string(),
        ));
    }
    if !rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(CoreError::InvalidInput(
            "cwd may not escape the project root".to_string(),
        ));
    }
    let full = root.join(rel_path);
    if !full.is_dir() {
        return Err(CoreError::InvalidInput(format!(
            "cwd is not a directory: {rel}"
        )));
    }
    Ok(full)
}

/// Display name of a project folder.
pub fn folder_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// Badge initials derived from a name: first letters of the first two words,
/// or the first two chars of a single word, uppercased.
pub fn derive_icon_initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let initials: String = match (words.next(), words.next()) {
        (Some(a), Some(b)) => a.chars().take(1).chain(b.chars().take(1)).collect(),
        (Some(a), None) => a.chars().take(2).collect(),
        (None, _) => String::new(),
    };
    if initials.is_empty() {
        "?".to_string()
    } else {
        initials.to_uppercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::RestartPolicy;

    fn write_config(dir: &Path, yml: &str) {
        std::fs::write(dir.join(CONFIG_FILE_NAME), yml).unwrap();
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_project_config(dir.path()).unwrap().is_none());
    }

    #[test]
    fn defaults_fill_in_name_and_initials() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: pnpm dev\n",
        );
        let cfg = load_project_config(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.name, folder_name(dir.path()));
        assert_eq!(cfg.icon_initials, derive_icon_initials(&cfg.name));
        let p = &cfg.processes[0];
        assert_eq!(p.spec.cwd, dir.path());
        assert_eq!(p.spec.kind, ProcessKind::Service);
        assert_eq!(p.spec.restart_policy, RestartPolicy::Never);
        assert!(!p.auto_start);
    }

    #[test]
    fn explicit_name_and_initials_win() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "name: Webshop\nicon_initials: wxyz\n");
        let cfg = load_project_config(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.name, "Webshop");
        assert_eq!(cfg.icon_initials, "WX", "truncated to 2 chars, uppercased");
    }

    #[test]
    fn parse_error_is_config_error_with_location() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "name: [unclosed\n");
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)), "got {err:?}");
        assert!(err.to_string().contains("podium.yml"));
    }

    #[test]
    fn unknown_field_is_config_error() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "naem: Webshop\n");
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
        assert!(err.to_string().contains("naem"));
    }

    #[test]
    fn traversal_cwd_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: x\n    cwd: ../outside\n",
        );
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
        assert!(err.to_string().contains("escape"));
    }

    #[test]
    fn absolute_cwd_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: x\n    cwd: /tmp\n",
        );
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(err.to_string().contains("relative"));
    }

    #[test]
    fn relative_cwd_resolves_under_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("apps/web")).unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: x\n    cwd: apps/web\n",
        );
        let cfg = load_project_config(dir.path()).unwrap().unwrap();
        assert_eq!(cfg.processes[0].spec.cwd, dir.path().join("apps/web"));
    }

    #[test]
    fn duplicate_process_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: a\n  - name: web\n    command: b\n",
        );
        let err = load_project_config(dir.path()).unwrap_err();
        assert!(matches!(err, CoreError::Config(_)));
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn env_map_becomes_spec_env_pairs() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "processes:\n  - name: web\n    command: x\n    env: { PORT: \"3000\", HOST: localhost }\n",
        );
        let cfg = load_project_config(dir.path()).unwrap().unwrap();
        // BTreeMap keeps keys sorted deterministically.
        assert_eq!(
            cfg.processes[0].spec.env,
            vec![
                ("HOST".to_string(), "localhost".to_string()),
                ("PORT".to_string(), "3000".to_string()),
            ]
        );
    }

    #[test]
    fn icon_initials_derivation() {
        assert_eq!(derive_icon_initials("Webshop"), "WE");
        assert_eq!(derive_icon_initials("My Webshop"), "MW");
        assert_eq!(derive_icon_initials("a b c"), "AB");
        assert_eq!(derive_icon_initials("x"), "X");
        assert_eq!(derive_icon_initials("  "), "?");
    }
}
