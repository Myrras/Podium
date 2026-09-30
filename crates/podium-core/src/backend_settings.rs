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

/// Like [`save`], but a WSL distro must be one that is installed: a typo
/// would otherwise only surface after a restart, as every spawn failing.
/// The stored name takes the installed distro's own casing. Off Windows the
/// field is ignored anyway, so nothing is checked there.
pub fn save_checked(path: &Path, settings: BackendSettings) -> CoreResult<BackendSettings> {
    let mut settings = settings.normalized();
    if cfg!(windows) {
        if let Some(requested) = &settings.wsl_distro {
            settings.wsl_distro = Some(resolve_distro(requested, &installed_wsl_distros()?)?);
        }
    }
    save(path, settings)
}

/// The installed WSL distros, from `wsl.exe -l -q`.
fn installed_wsl_distros() -> CoreResult<Vec<String>> {
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(["-l", "-q"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = cmd
        .output()
        .ok()
        .filter(|o| o.status.success())
        .ok_or_else(|| {
            CoreError::InvalidInput("cannot list WSL distros: is WSL installed?".to_string())
        })?;
    Ok(parse_wsl_list(&output.stdout))
}

/// Parse `wsl.exe -l -q` output: one distro per line, written as UTF-16LE
/// (with a BOM on some builds) unless `WSL_UTF8=1` makes it UTF-8.
fn parse_wsl_list(stdout: &[u8]) -> Vec<String> {
    let text = if stdout.len() % 2 == 0 && stdout.contains(&0) {
        let units: Vec<u16> = stdout
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(stdout).into_owned()
    };
    text.lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}'))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// `requested` as spelled by the installed distro it names (WSL matches
/// distro names case-insensitively), or a readable error listing what is
/// installed.
fn resolve_distro(requested: &str, installed: &[String]) -> CoreResult<String> {
    installed
        .iter()
        .find(|d| d.eq_ignore_ascii_case(requested))
        .cloned()
        .ok_or_else(|| {
            let available = if installed.is_empty() {
                "none".to_string()
            } else {
                installed.join(", ")
            };
            CoreError::InvalidInput(format!(
                "WSL distro \"{requested}\" is not installed (installed: {available})"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn wsl_list_parses_utf16_with_a_bom() {
        let out = utf16le("\u{feff}Ubuntu\r\ndocker-desktop\r\n\r\n");
        assert_eq!(parse_wsl_list(&out), ["Ubuntu", "docker-desktop"]);
    }

    #[test]
    fn wsl_list_parses_utf8() {
        assert_eq!(parse_wsl_list(b"Ubuntu-22.04\n"), ["Ubuntu-22.04"]);
        assert!(parse_wsl_list(b"").is_empty());
    }

    #[test]
    fn distro_resolves_case_insensitively_to_its_own_spelling() {
        let installed = vec!["Ubuntu".to_string(), "Debian".to_string()];
        assert_eq!(resolve_distro("ubuntu", &installed).unwrap(), "Ubuntu");
    }

    #[test]
    fn unknown_distro_is_refused_with_what_is_installed() {
        let installed = vec!["Ubuntu".to_string()];
        let err = resolve_distro("Ubunto", &installed)
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"Ubunto\" is not installed"), "{err}");
        assert!(err.contains("installed: Ubuntu"), "{err}");
        let none = resolve_distro("Ubuntu", &[]).unwrap_err().to_string();
        assert!(none.contains("installed: none"), "{none}");
    }

    #[cfg(not(windows))]
    #[test]
    fn save_checked_skips_the_check_off_windows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backend.json");
        let saved = save_checked(
            &path,
            BackendSettings {
                wsl_distro: Some("Anything".to_string()),
            },
        )
        .unwrap();
        assert_eq!(saved.wsl_distro.as_deref(), Some("Anything"));
    }

    #[test]
    fn save_checked_clears_without_asking_wsl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backend.json");
        let saved = save_checked(
            &path,
            BackendSettings {
                wsl_distro: Some("  ".to_string()),
            },
        )
        .unwrap();
        assert_eq!(saved.wsl_distro, None);
    }

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
