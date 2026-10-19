//! Pre-accepting Claude Code's folder-trust dialog for the assistant's
//! workspace (`{app_data_dir}/assistant-workspace`), so a sub-agent never
//! sits on it.
//!
//! Claude Code records an accepted dialog in its global config file as
//! `projects["<abs path>"].hasTrustDialogAccepted = true` and, before showing
//! the dialog, checks that key for the session's cwd and its parent dirs (up
//! to a git root). The file is `$CLAUDE_CONFIG_DIR/.claude.json`, else
//! `~/.claude.json` (checked against Claude Code 2.1.301). Podium owns the
//! workspace folder and launches nothing else there, so it accepts on the
//! user's behalf — exactly the one key, for exactly that path.
//!
//! The CLI rewrites that file itself, holding a `<file>.lock` directory
//! (proper-lockfile's mkdir lock) and re-reading the file under it, so a
//! write made under the same lock is never lost to it. Podium takes that
//! lock, re-reads, changes only its key, and replaces the file atomically.

use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The key Claude Code stores an accepted trust dialog under.
const TRUST_KEY: &str = "hasTrustDialogAccepted";

/// How often, and how long apart, the CLI's config lock is tried before the
/// write is left for the next sub-agent start. Short: the CLI holds it for a
/// read-modify-write only, and this runs on the start path.
const LOCK_TRIES: u32 = 5;
const LOCK_RETRY: Duration = Duration::from_millis(40);

/// Claude Code's global config file, from Podium's own environment.
// ponytail: like `claude_config_dir`, a CLAUDE_CONFIG_DIR set only in the
// login shell profile is missed; the file then usually does not exist and the
// write is skipped (the trust-prompt detection still reports the dialog).
pub fn claude_global_config() -> Option<PathBuf> {
    global_config_in(|key| std::env::var_os(key))
}

/// [`claude_global_config`] over an injectable environment: the CLI's own
/// rule — `$CLAUDE_CONFIG_DIR`, else the home dir, then `.claude.json`
/// (`.claude-custom-oauth.json` with `CLAUDE_CODE_CUSTOM_OAUTH_URL` set).
fn global_config_in(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |key: &str| env(key).filter(|v| !v.is_empty());
    let dir = set("CLAUDE_CONFIG_DIR")
        .or_else(|| set("HOME"))
        .or_else(|| set("USERPROFILE"))?;
    let suffix = if set("CLAUDE_CODE_CUSTOM_OAUTH_URL").is_some() {
        "-custom-oauth"
    } else {
        ""
    };
    Some(PathBuf::from(dir).join(format!(".claude{suffix}.json")))
}

/// The `projects` key the CLI uses for `dir`: the absolute path as is, with
/// `/` separators on Windows. `dir` is expected canonical already — the CLI
/// keys on its physical cwd. `None` for a path that is not UTF-8.
fn project_key(dir: &Path) -> Option<String> {
    let key = dir.to_str()?;
    Some(if cfg!(windows) {
        key.replace('\\', "/")
    } else {
        key.to_string()
    })
}

/// What [`ensure_trusted`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustOutcome {
    /// The key was already true; the file was not touched.
    AlreadyTrusted,
    /// The key was set.
    Written,
    /// Nothing was written, and why — Podium-owned text, never file content.
    Skipped(&'static str),
}

/// Make sure `config` (Claude Code's global config file) records the trust
/// dialog as accepted for `dir`. Never fails: anything unexpected leaves the
/// file alone and says why, and the trust-prompt detection is the fallback.
///
/// A missing file is not created. It means Claude Code never ran with this
/// config dir (or Podium resolved a different one than the CLI does), and the
/// CLI treats a missing config specially — restoring it from its own backups
/// — which a file holding only Podium's key would pre-empt.
pub fn ensure_trusted(config: &Path, dir: &Path) -> TrustOutcome {
    let Some(key) = project_key(dir) else {
        return TrustOutcome::Skipped("the workspace path is not UTF-8");
    };
    // Fast path without the lock: most starts find the key already set.
    match read_config(config) {
        Ok((_, value)) if is_trusted(&value, &key) => return TrustOutcome::AlreadyTrusted,
        Ok(_) => {}
        Err(reason) => return TrustOutcome::Skipped(reason),
    }
    let Some(_lock) = ConfigLock::acquire(config) else {
        return TrustOutcome::Skipped("Claude Code holds its config lock");
    };
    // Re-read under the lock: the CLI may have written since.
    let (raw, mut value) = match read_config(config) {
        Ok(read) => read,
        Err(reason) => return TrustOutcome::Skipped(reason),
    };
    if is_trusted(&value, &key) {
        return TrustOutcome::AlreadyTrusted;
    }
    if let Err(reason) = set_trusted(&mut value, &key) {
        return TrustOutcome::Skipped(reason);
    }
    match write_config(config, &raw, &value) {
        Ok(()) => TrustOutcome::Written,
        Err(reason) => TrustOutcome::Skipped(reason),
    }
}

/// The file's text and its parsed form.
fn read_config(config: &Path) -> Result<(String, serde_json::Value), &'static str> {
    let raw = match fs::read_to_string(config) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("Claude Code's config file does not exist")
        }
        Err(_) => return Err("Claude Code's config file could not be read"),
    };
    let value =
        serde_json::from_str(&raw).map_err(|_| "Claude Code's config file is not valid JSON")?;
    Ok((raw, value))
}

fn is_trusted(config: &serde_json::Value, key: &str) -> bool {
    config
        .get("projects")
        .and_then(|p| p.get(key))
        .and_then(|e| e.get(TRUST_KEY))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Set the one key, creating `projects` and the entry when missing. Any
/// other shape than the CLI's own (objects all the way) is left alone.
fn set_trusted(config: &mut serde_json::Value, key: &str) -> Result<(), &'static str> {
    const UNEXPECTED: &str = "Claude Code's config file has an unexpected shape";
    let root = config.as_object_mut().ok_or(UNEXPECTED)?;
    let projects = root
        .entry("projects")
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or(UNEXPECTED)?;
    let entry = projects
        .entry(key)
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or(UNEXPECTED)?;
    entry.insert(TRUST_KEY.to_string(), serde_json::Value::Bool(true));
    Ok(())
}

/// Replace `config` with `value` atomically: a temp file in the same dir as
/// the real file (a symlinked config keeps its link), the original's
/// permissions, then a rename. Written the way the CLI writes it — two-space
/// indent — or compact when the original had no line breaks, with the
/// original's trailing newline (or none) kept.
fn write_config(config: &Path, raw: &str, value: &serde_json::Value) -> Result<(), &'static str> {
    const WRITE_FAILED: &str = "Claude Code's config file could not be written";
    let target = fs::canonicalize(config).map_err(|_| WRITE_FAILED)?;
    let permissions = fs::metadata(&target)
        .map_err(|_| WRITE_FAILED)?
        .permissions();
    let mut text = if raw.trim_end().contains('\n') {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|_| WRITE_FAILED)?;
    if raw.ends_with('\n') {
        text.push('\n');
    }
    let dir = target.parent().ok_or(WRITE_FAILED)?;
    let name = target.file_name().ok_or(WRITE_FAILED)?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.podium-{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Owner-only until the original's permissions are applied: the file
        // can hold account details.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.set_permissions(permissions)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(WRITE_FAILED);
    }
    Ok(())
}

/// The CLI's own config lock: the directory `<config>.lock`, made with
/// `mkdir` (atomic). Removed again on drop. A lock someone else holds is
/// never taken over, even a stale one — the CLI clears that itself.
struct ConfigLock(PathBuf);

impl ConfigLock {
    fn acquire(config: &Path) -> Option<Self> {
        let mut path = config.as_os_str().to_owned();
        path.push(".lock");
        let path = PathBuf::from(path);
        for attempt in 0..LOCK_TRIES {
            match fs::create_dir(&path) {
                Ok(()) => return Some(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt + 1 < LOCK_TRIES {
                        std::thread::sleep(LOCK_RETRY);
                    }
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str =
        "/Users/someone/Library/Application Support/com.podium.app/assistant-workspace";

    fn config_with(dir: &Path, text: &str) -> PathBuf {
        let path = dir.join(".claude.json");
        fs::write(&path, text).unwrap();
        path
    }

    fn read(path: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn it_adds_the_key_for_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_with(dir.path(), "{\n  \"numStartups\": 3\n}");
        assert_eq!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::Written
        );
        let value = read(&config);
        assert!(is_trusted(&value, &project_key(Path::new(WS)).unwrap()));
        assert_eq!(value["numStartups"], 3);
        // The lock is released again.
        assert!(!dir.path().join(".claude.json.lock").exists());
    }

    #[test]
    fn every_other_key_is_kept_in_place() {
        let dir = tempfile::tempdir().unwrap();
        // Unsorted keys, long floats, an existing entry for the workspace with
        // keys of its own, another project, nested values, unicode: written
        // back exactly as the CLI wrote them, plus the one key.
        let before = r#"{
  "zeta": 1,
  "score": 0.30000000000000004,
  "ratio": 0.1234567890123456,
  "firstStartTime": 1760000000000,
  "alpha": {
    "nested": [
      1,
      2.5,
      "naïve"
    ],
    "empty": {}
  },
  "projects": {
    "/other/project": {
      "hasTrustDialogAccepted": false,
      "allowedTools": []
    },
    "WS": {
      "lastVersionBase": "2.1.301",
      "hasTrustDialogAccepted": false
    }
  },
  "oauthAccount": {
    "emailAddress": "someone@example.invalid"
  }
}"#
        .replace("\"WS\"", &format!("{WS:?}"));
        let config = config_with(dir.path(), &before);
        assert_eq!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::Written
        );
        let expected = before.replacen(
            "\"lastVersionBase\": \"2.1.301\",\n      \"hasTrustDialogAccepted\": false",
            "\"lastVersionBase\": \"2.1.301\",\n      \"hasTrustDialogAccepted\": true",
            1,
        );
        assert_ne!(expected, before);
        assert_eq!(fs::read_to_string(&config).unwrap(), expected);
    }

    #[test]
    fn a_trusted_workspace_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_with(dir.path(), "{\"a\":1}");
        assert_eq!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::Written
        );
        let once = fs::read_to_string(&config).unwrap();
        // Compact in, compact out, no newline added.
        assert!(!once.contains('\n'), "{once}");
        let modified = fs::metadata(&config).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::AlreadyTrusted
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), once);
        assert_eq!(fs::metadata(&config).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn malformed_or_missing_config_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        for text in ["{\"projects\": {", "[1, 2]", "{\"projects\": []}", ""] {
            let config = config_with(dir.path(), text);
            assert!(
                matches!(
                    ensure_trusted(&config, Path::new(WS)),
                    TrustOutcome::Skipped(_)
                ),
                "{text}"
            );
            assert_eq!(fs::read_to_string(&config).unwrap(), text);
        }
        let missing = dir.path().join("absent").join(".claude.json");
        assert!(matches!(
            ensure_trusted(&missing, Path::new(WS)),
            TrustOutcome::Skipped(_)
        ));
        assert!(!missing.exists());
        // No temp file was left behind either.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [".claude.json"]);
    }

    #[test]
    fn a_held_lock_is_waited_for_then_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_with(dir.path(), "{}\n");
        let lock = dir.path().join(".claude.json.lock");
        fs::create_dir(&lock).unwrap();
        assert!(matches!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::Skipped(_)
        ));
        assert_eq!(fs::read_to_string(&config).unwrap(), "{}\n");
        assert!(lock.is_dir(), "someone else's lock is not removed");
        fs::remove_dir(&lock).unwrap();
        assert_eq!(
            ensure_trusted(&config, Path::new(WS)),
            TrustOutcome::Written
        );
        // The trailing newline the file had is kept.
        assert!(fs::read_to_string(&config).unwrap().ends_with("}\n"));
    }

    #[cfg(unix)]
    #[test]
    fn the_files_permissions_and_symlink_are_kept() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let real = config_with(dir.path(), "{}");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("linked.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(ensure_trusted(&link, Path::new(WS)), TrustOutcome::Written);
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(is_trusted(&read(&real), WS));
        let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn the_config_file_follows_claude_config_dir() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                vars.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert_eq!(
            global_config_in(env(&[
                ("CLAUDE_CONFIG_DIR", "/custom/claude"),
                ("HOME", "/home/someone"),
            ])),
            Some(PathBuf::from("/custom/claude/.claude.json"))
        );
        assert_eq!(
            global_config_in(env(&[("CLAUDE_CONFIG_DIR", ""), ("HOME", "/home/someone")])),
            Some(PathBuf::from("/home/someone/.claude.json"))
        );
        assert_eq!(
            global_config_in(env(&[
                ("HOME", "/home/someone"),
                ("CLAUDE_CODE_CUSTOM_OAUTH_URL", "https://example.invalid"),
            ])),
            Some(PathBuf::from("/home/someone/.claude-custom-oauth.json"))
        );
        assert_eq!(global_config_in(env(&[])), None);
    }
}
