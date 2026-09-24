//! Finding an interactive agent's Claude Code session id after the fact, and
//! remembering it so the agent can be resumed after a restart.
//!
//! Podium's agents run as real interactive PTYs (the user's terminal view),
//! not the piped `stream-json` mode `assistant.rs` parses for its own
//! session id. Claude Code itself already writes every session — headless
//! or interactive — to `~/.claude/projects/<sanitized-cwd>/<session-id>.jsonl`
//! (verified empirically: the filename stem is the exact id `--resume`
//! takes), so this module reads that directory instead of the PTY stream.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

/// Claude Code's own scheme for naming a cwd's project directory: every path
/// separator becomes `-` (empirically verified against real
/// `~/.claude/projects/` entries).
pub(crate) fn sanitize_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy().replace(['/', '\\'], "-")
}

/// The newest `.jsonl` session file under `projects_root`'s directory for
/// `cwd` whose modification time is at or after `after`, if any. `after`
/// should be the agent's own spawn time, so a session file left over from an
/// unrelated earlier run in the same directory is never picked up.
pub(crate) fn find_latest_session_id_in(
    projects_root: &Path,
    cwd: &Path,
    after: SystemTime,
) -> Option<String> {
    let dir = projects_root.join(sanitize_cwd(cwd));
    let mut newest: Option<(SystemTime, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < after {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, stem.to_string()));
        }
    }
    newest.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn touch(path: &Path) {
        fs::write(path, "{}").unwrap();
    }

    #[test]
    fn sanitize_cwd_replaces_separators() {
        assert_eq!(sanitize_cwd(Path::new("/home/x/proj")), "-home-x-proj");
    }

    #[test]
    fn missing_directory_returns_none() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/nonexistent/project/cwd");
        assert_eq!(
            find_latest_session_id_in(projects_root.path(), cwd, SystemTime::UNIX_EPOCH),
            None
        );
    }

    #[test]
    fn picks_the_newest_jsonl_at_or_after_the_cutoff() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/some/project/root");
        let sessions_dir = projects_root.path().join(sanitize_cwd(cwd));
        fs::create_dir_all(&sessions_dir).unwrap();

        let before = sessions_dir.join("old-session.jsonl");
        touch(&before);
        let cutoff = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));

        let after_id = "new-session-id";
        let after_path = sessions_dir.join(format!("{after_id}.jsonl"));
        touch(&after_path);

        assert_eq!(
            find_latest_session_id_in(projects_root.path(), cwd, cutoff),
            Some(after_id.to_string())
        );
    }

    #[test]
    fn ignores_non_jsonl_entries() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/another/project");
        let sessions_dir = projects_root.path().join(sanitize_cwd(cwd));
        fs::create_dir_all(&sessions_dir).unwrap();
        touch(&sessions_dir.join("notes.txt"));

        assert_eq!(
            find_latest_session_id_in(projects_root.path(), cwd, SystemTime::UNIX_EPOCH),
            None
        );
    }
}

const LOCK_POISONED: &str = "session memory store lock poisoned";

/// One agent session Podium remembers so it can be resumed after a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedSession {
    pub adapter_id: String,
    pub name: String,
    /// Absolute cwd the agent ran in (project root or a worktree under it).
    pub cwd: PathBuf,
    /// Claude Code's own session id, as `--resume` expects it.
    pub session_id: String,
    pub updated_at: DateTime<Utc>,
}

struct Inner {
    path: Option<PathBuf>,
    /// Keyed by project root path, mirroring `TodoStore`/`ScratchpadStore` —
    /// survives the project's per-run `ProjectId` changing across restarts.
    by_root: BTreeMap<PathBuf, Vec<RememberedSession>>,
}

/// Thread-safe store for remembered agent sessions, one JSON file in the app
/// data dir. Writes are atomic (temp file + rename); a missing or corrupt
/// file is treated as empty. Until [`SessionMemoryStore::set_path`] is
/// called the store is in-memory only (tests).
pub(crate) struct SessionMemoryStore {
    inner: Mutex<Inner>,
}

impl SessionMemoryStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: None,
                by_root: BTreeMap::new(),
            }),
        }
    }

    pub fn set_path(&self, path: PathBuf) {
        let by_root = load(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.by_root = by_root;
    }

    /// Remembered sessions for a project root, oldest first.
    pub fn list(&self, root: &Path) -> Vec<RememberedSession> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .by_root
            .get(root)
            .cloned()
            .unwrap_or_default()
    }

    /// Record (or replace, matched on `cwd` + `name`) a remembered session
    /// for `root`.
    pub fn remember(&self, root: &Path, entry: RememberedSession) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let list = inner.by_root.entry(root.to_path_buf()).or_default();
        list.retain(|e| !(e.cwd == entry.cwd && e.name == entry.name));
        list.push(entry);
        save(&inner)
    }

    /// Drop every remembered session for `root` (called once its sessions
    /// have been restored, so a second `open_project` in the same run does
    /// not re-spawn them).
    pub fn clear(&self, root: &Path) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.by_root.remove(root);
        save(&inner)
    }
}

fn load(path: &Path) -> BTreeMap<PathBuf, Vec<RememberedSession>> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("session memory file is corrupt, starting fresh: {e}");
        BTreeMap::new()
    })
}

fn save(inner: &Inner) -> CoreResult<()> {
    let Some(path) = &inner.path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&inner.by_root).map_err(|e| {
        crate::error::CoreError::InvalidInput(format!("failed to encode session memory: {e}"))
    })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn entry(name: &str, cwd: &str, session_id: &str) -> RememberedSession {
        RememberedSession {
            adapter_id: "claude-code".to_string(),
            name: name.to_string(),
            cwd: PathBuf::from(cwd),
            session_id: session_id.to_string(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn remembers_and_lists_by_root() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(root, entry("agent-1", "/project/root", "sess-a"))
            .unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "sess-a");
        assert!(store.list(Path::new("/other/root")).is_empty());
    }

    #[test]
    fn remember_replaces_same_cwd_and_name() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(root, entry("agent-1", "/project/root", "sess-a"))
            .unwrap();
        store
            .remember(root, entry("agent-1", "/project/root", "sess-b"))
            .unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "sess-b");
    }

    #[test]
    fn clear_drops_everything_for_that_root() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(root, entry("agent-1", "/project/root", "sess-a"))
            .unwrap();
        store.clear(root).unwrap();
        assert!(store.list(root).is_empty());
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        let root = Path::new("/project/root");

        let store = SessionMemoryStore::new();
        store.set_path(file.clone());
        store
            .remember(root, entry("agent-1", "/project/root", "sess-a"))
            .unwrap();

        let reloaded = SessionMemoryStore::new();
        reloaded.set_path(file);
        let listed = reloaded.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "sess-a");
    }

    #[test]
    fn corrupt_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        std::fs::write(&file, "not json").unwrap();

        let store = SessionMemoryStore::new();
        store.set_path(file);
        assert!(store.list(Path::new("/anything")).is_empty());
    }
}
