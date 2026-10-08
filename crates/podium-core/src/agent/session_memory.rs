//! Agent sessions Podium remembers across an app restart, so each agent can
//! be resumed the next time its project opens (Settings → Agents →
//! "Remember agent sessions").
//!
//! The session id itself is Podium's own: every Claude Code agent runs under
//! a Podium-assigned `--session-id` (see [`crate::agent::RestartCommands`]),
//! so capturing it needs no lookup in Claude Code's files — only the restore
//! asks the CLI whether the session is still stored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::ids::{ScratchpadId, TodoId};

/// Whether `id` is a Claude Code session id: a hyphenated UUID. Anything
/// else — a tampered `session_memory.json` — must never reach `--resume`,
/// where a value like `-x` would be read as a flag. The length check pins
/// the hyphenated form (`try_parse` also takes simple, braced and URN
/// spellings).
pub(crate) fn is_valid_session_id(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::try_parse(id).is_ok()
}

const LOCK_POISONED: &str = "session memory store lock poisoned";

/// How long a remembered session stays restorable. Entries are normally
/// consumed on the next open of their project, but a project that keeps
/// failing to open (a drive not mounted) would otherwise bring weeks-old
/// agents back the day it finally opens.
pub(crate) const MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::days(14);

/// One agent session Podium remembers so it can be resumed after a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedSession {
    pub adapter_id: String,
    pub name: String,
    /// The CLI session the agent was on, as `--resume` expects it.
    pub session_id: String,
    pub updated_at: DateTime<Utc>,
    /// To-dos the agent owned, re-assigned to the restored session — the
    /// assignment itself is runtime-only. `default` reads older files.
    #[serde(default)]
    pub todo_ids: Vec<TodoId>,
    /// Scratchpads the agent owned, likewise.
    #[serde(default)]
    pub scratchpad_ids: Vec<ScratchpadId>,
    /// The per-spawn args the agent was started with (e.g. `--model opus`),
    /// replacing the global default args again on restore. `None` = it used
    /// the defaults.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Whether it was spawned to work in a worktree, and under which name —
    /// both shape its identity prompt, so a restore passes them on again.
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub worktree_name: Option<String>,
}

impl RememberedSession {
    /// Older than [`MAX_AGE`] at `now`, so no longer restored.
    pub(crate) fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now - self.updated_at > MAX_AGE
    }
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

    /// Remembered sessions for a project root, oldest first (tests; the app
    /// only ever [`pop_front`](Self::pop_front)s them).
    #[cfg(test)]
    pub fn list(&self, root: &Path) -> Vec<RememberedSession> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .by_root
            .get(root)
            .cloned()
            .unwrap_or_default()
    }

    /// Record (or replace, matched on the session id) a remembered session
    /// for `root`.
    #[cfg(test)]
    pub fn remember(&self, root: &Path, entry: RememberedSession) -> CoreResult<()> {
        self.remember_all(vec![(root.to_path_buf(), entry)])
    }

    /// [`Self::remember`] for many sessions with a single write of the file —
    /// shutdown captures every agent at once, on the app's exit path.
    pub fn remember_all(&self, entries: Vec<(PathBuf, RememberedSession)>) -> CoreResult<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        for (root, entry) in entries {
            let list = inner.by_root.entry(root).or_default();
            list.retain(|e| e.session_id != entry.session_id);
            list.push(entry);
        }
        save(&inner)
    }

    /// Drop every remembered session for every root — the setting was turned
    /// off, so nothing captured earlier may come back once it is on again.
    pub fn clear_all(&self) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if inner.by_root.is_empty() {
            return Ok(());
        }
        inner.by_root.clear();
        save(&inner)
    }

    /// Drop every remembered session for `root` — its project opened with
    /// the setting off, so they must not come back later. No write when the
    /// root has none.
    pub fn clear(&self, root: &Path) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if inner.by_root.remove(root).is_none() {
            return Ok(());
        }
        save(&inner)
    }

    /// Remove and return the oldest remembered session for `root`, saving
    /// the rest at once — so the restore takes one entry off the disk at a
    /// time and a quit or crash part-way leaves the others stored. A failed
    /// save is logged, not returned: it must not cancel the restore.
    pub fn pop_front(&self, root: &Path) -> Option<RememberedSession> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let list = inner.by_root.get_mut(root)?;
        let entry = (!list.is_empty()).then(|| list.remove(0));
        if list.is_empty() {
            inner.by_root.remove(root);
        }
        if let Err(e) = save(&inner) {
            tracing::warn!("saving remembered agent sessions failed: {e}");
        }
        entry
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
mod tests {
    use super::*;

    fn entry(name: &str, session_id: &str) -> RememberedSession {
        RememberedSession {
            adapter_id: "claude-code".to_string(),
            name: name.to_string(),
            session_id: session_id.to_string(),
            updated_at: Utc::now(),
            todo_ids: Vec::new(),
            scratchpad_ids: Vec::new(),
            args: None,
            worktree: false,
            worktree_name: None,
        }
    }

    #[test]
    fn session_ids_must_be_uuid_shaped() {
        assert!(is_valid_session_id("00000000-0000-4000-8000-00000000000a"));
        assert!(!is_valid_session_id("-x"));
        assert!(!is_valid_session_id("sess-a"));
        assert!(!is_valid_session_id("00000000-0000-4000-8000-00000000000g"));
        assert!(!is_valid_session_id(
            "00000000-0000-4000-8000-00000000000a0"
        ));
        // Right length and alphabet, but no UUID.
        assert!(!is_valid_session_id(&"-".repeat(36)));
        assert!(!is_valid_session_id("-0000000-0000-4000-8000-00000000000a"));
        // A UUID, but not in the hyphenated form Claude Code names files by.
        assert!(!is_valid_session_id(
            "{00000000-0000-4000-8000-00000000000a}"
        ));
        assert!(!is_valid_session_id("00000000000040008000000000000000"));
    }

    #[test]
    fn remembers_and_lists_by_root() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(
                root,
                entry("agent-1", "00000000-0000-4000-8000-000000000001"),
            )
            .unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "00000000-0000-4000-8000-000000000001");
        assert!(store.list(Path::new("/other/root")).is_empty());
    }

    #[test]
    fn remember_replaces_the_same_session() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        let id = "00000000-0000-4000-8000-000000000001";
        store.remember(root, entry("agent-1", id)).unwrap();
        store.remember(root, entry("agent-1 renamed", id)).unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "agent-1 renamed");
    }

    #[test]
    fn pop_front_takes_one_entry_off_the_disk_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        let store = SessionMemoryStore::new();
        store.set_path(file.clone());
        let root = Path::new("/project/root");
        // Nothing remembered: nothing popped, and no file written for it.
        assert!(store.pop_front(root).is_none());
        store.clear(root).unwrap();
        assert!(!file.exists());
        store
            .remember_all(vec![
                (
                    root.to_path_buf(),
                    entry("agent-1", "00000000-0000-4000-8000-000000000001"),
                ),
                (
                    root.to_path_buf(),
                    entry("agent-2", "00000000-0000-4000-8000-000000000002"),
                ),
            ])
            .unwrap();
        assert_eq!(store.pop_front(root).unwrap().name, "agent-1");
        // The rest is already on disk without it.
        let reloaded = SessionMemoryStore::new();
        reloaded.set_path(file);
        let left = reloaded.list(root);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "agent-2");
        assert_eq!(store.pop_front(root).unwrap().name, "agent-2");
        assert!(store.pop_front(root).is_none());
    }

    #[test]
    fn a_failed_save_still_pops_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionMemoryStore::new();
        // A path whose parent is a file: every save fails.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "").unwrap();
        store.set_path(blocker.join("session_memory.json"));
        let root = Path::new("/project/root");
        // `remember` fails to save but keeps the entry in memory.
        let _ = store.remember(
            root,
            entry("agent-1", "00000000-0000-4000-8000-000000000001"),
        );
        assert_eq!(store.pop_front(root).unwrap().name, "agent-1");
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        let root = Path::new("/project/root");

        let store = SessionMemoryStore::new();
        store.set_path(file.clone());
        store
            .remember(
                root,
                entry("agent-1", "00000000-0000-4000-8000-000000000001"),
            )
            .unwrap();

        let reloaded = SessionMemoryStore::new();
        reloaded.set_path(file);
        let listed = reloaded.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "00000000-0000-4000-8000-000000000001");
    }

    #[test]
    fn remember_all_writes_every_entry_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        let store = SessionMemoryStore::new();
        store.set_path(file.clone());
        store
            .remember_all(vec![
                (
                    PathBuf::from("/root/a"),
                    entry("agent-1", "00000000-0000-4000-8000-000000000001"),
                ),
                (
                    PathBuf::from("/root/b"),
                    entry("agent-2", "00000000-0000-4000-8000-000000000002"),
                ),
            ])
            .unwrap();
        let reloaded = SessionMemoryStore::new();
        reloaded.set_path(file);
        assert_eq!(reloaded.list(Path::new("/root/a")).len(), 1);
        assert_eq!(reloaded.list(Path::new("/root/b")).len(), 1);
    }

    #[test]
    fn clear_all_drops_every_root() {
        let store = SessionMemoryStore::new();
        for root in ["/root/a", "/root/b"] {
            store
                .remember(
                    Path::new(root),
                    entry("agent-1", "00000000-0000-4000-8000-000000000001"),
                )
                .unwrap();
        }
        store.clear_all().unwrap();
        assert!(store.list(Path::new("/root/a")).is_empty());
        assert!(store.list(Path::new("/root/b")).is_empty());
    }

    #[test]
    fn entries_expire_after_max_age() {
        let e = entry("agent-1", "00000000-0000-4000-8000-000000000001");
        assert!(!e.is_expired(e.updated_at + MAX_AGE));
        assert!(e.is_expired(e.updated_at + MAX_AGE + chrono::TimeDelta::seconds(1)));
    }

    #[test]
    fn older_files_without_the_newer_fields_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        std::fs::write(
            &file,
            r#"{"/root":[{"adapterId":"claude-code","name":"agent-1","cwd":"/root","sessionId":"00000000-0000-4000-8000-000000000001","updatedAt":"2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();
        let store = SessionMemoryStore::new();
        store.set_path(file);
        let listed = store.list(Path::new("/root"));
        assert_eq!(listed.len(), 1);
        assert!(listed[0].todo_ids.is_empty());
        assert_eq!(listed[0].args, None);
        assert!(!listed[0].worktree);
        assert_eq!(listed[0].worktree_name, None);
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
