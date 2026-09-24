//! Which backend runs what Podium executes, persisted as one JSON file in the
//! app data dir (`backend.json`).
//!
//! Unlike the agent settings this is read once, at startup, to fix
//! [`crate::platform::init_wsl`] before anything spawns; Settings → General
//! only rewrites the file, and the change applies on the next launch. So
//! there is no live store here — just load/save of the document.
//!
//! Writes are atomic (temp file + rename); a missing or corrupt file is
//! treated as defaults (the native backend).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// The whole persisted document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendSettings {
    /// WSL distro to run everything in (Windows only; ignored elsewhere).
    /// `None` = the native backend.
    #[serde(default)]
    pub wsl_distro: Option<String>,
}

impl BackendSettings {
    /// A copy with `wsl_distro` trimmed and a blank one cleared.
    pub fn normalized(mut self) -> Self {
        self.wsl_distro = self
            .wsl_distro
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        self
    }
}

/// Read the settings from disk; a missing or corrupt file is defaults.
pub fn load(path: &Path) -> BackendSettings {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BackendSettings::default();
    };
    serde_json::from_str::<BackendSettings>(&raw)
        .map(BackendSettings::normalized)
        .unwrap_or_else(|e| {
            tracing::warn!("backend settings file is corrupt, using defaults: {e}");
            BackendSettings::default()
        })
}

/// Atomically persist `settings` (normalized first); returns what was written.
pub fn save(path: &Path, settings: BackendSettings) -> CoreResult<BackendSettings> {
    let settings = settings.normalized();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode backend settings: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_the_native_backend() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&dir.path().join("backend.json")),
            BackendSettings::default()
        );
    }

    #[test]
    fn corrupt_file_is_the_native_backend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backend.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), BackendSettings::default());
    }

    #[test]
    fn a_file_without_the_field_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backend.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(load(&path).wsl_distro, None);
    }

    #[test]
    fn save_round_trips_and_normalizes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("backend.json");
        let written = save(
            &path,
            BackendSettings {
                wsl_distro: Some("  Ubuntu ".to_string()),
            },
        )
        .unwrap();
        assert_eq!(written.wsl_distro.as_deref(), Some("Ubuntu"));
        assert_eq!(load(&path), written);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"wslDistro\""));

        let cleared = save(
            &path,
            BackendSettings {
                wsl_distro: Some("   ".to_string()),
            },
        )
        .unwrap();
        assert_eq!(cleared.wsl_distro, None);
        assert_eq!(load(&path).wsl_distro, None);
    }
}
