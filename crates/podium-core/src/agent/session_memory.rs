//! Finding an interactive agent's Claude Code session id after the fact, and
//! remembering it so the agent can be resumed after a restart.
//!
//! Podium's agents run as real interactive PTYs (the user's terminal view),
//! not the piped `stream-json` mode `assistant.rs` parses for its own
//! session id. Claude Code itself already writes every session — headless
//! or interactive — to `~/.claude/projects/<sanitized-cwd>/<session-id>.jsonl`
//! (verified: the filename stem is the exact id `--resume`
//! takes), so this module reads that directory instead of the PTY stream.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;

/// Claude Code's own scheme for naming a cwd's project directory: every
/// character that is not ASCII alphanumeric becomes `-` (verified against real
/// `~/.claude/projects/` entries, e.g. `/home/x/.local/com.app` →
/// `-home-x--local-com-app`).
// ponytail: Claude Code truncates very long names (~200 chars) with a hash
// suffix; such cwds are not handled and simply find no session.
pub(crate) fn sanitize_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether `id` looks like a Claude Code session id (a UUID). Anything else —
/// a tampered `session_memory.json`, a stray file — must never reach
/// `--resume`, where a value like `-x` would be read as a flag.
pub(crate) fn is_valid_session_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Every session file in `projects_root`'s directory for `cwd` whose stem is
/// a valid session id, as (modified, id, path).
fn session_files(projects_root: &Path, cwd: &Path) -> Vec<(SystemTime, String, PathBuf)> {
    let Ok(dir) = std::fs::read_dir(projects_root.join(sanitize_cwd(cwd))) else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            if !is_valid_session_id(&id) {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, id, path))
        })
        .collect()
}

fn newest(files: impl Iterator<Item = (SystemTime, String, PathBuf)>) -> Option<String> {
    files.max_by_key(|(t, ..)| *t).map(|(_, id, _)| id)
}

/// The newest session file for `cwd` modified at or after `after`, if any.
/// `after` should be the agent's own spawn time, so a session file left over
/// from an unrelated earlier run in the same directory is never picked up.
/// Only unambiguous when a single agent runs in `cwd`.
pub(crate) fn find_latest_session_id_in(
    projects_root: &Path,
    cwd: &Path,
    after: SystemTime,
) -> Option<String> {
    newest(
        session_files(projects_root, cwd)
            .into_iter()
            .filter(|(t, ..)| *t >= after),
    )
}

/// The newest session file for `cwd` that Claude Code tagged with session
/// name `title` (`-n`): it writes a `{"type":"custom-title","customTitle":…}`
/// line into the session's jsonl. This tells apart several agents sharing one
/// cwd. The closing quote is left off so a variant suffix Claude Code appends
/// on a name clash still matches.
pub(crate) fn find_titled_session_id_in(
    projects_root: &Path,
    cwd: &Path,
    title: &str,
) -> Option<String> {
    let needle = format!("\"customTitle\":\"{title}");
    newest(
        session_files(projects_root, cwd)
            .into_iter()
            .filter(|(_, _, path)| {
                std::fs::read_to_string(path).is_ok_and(|raw| raw.contains(&needle))
            }),
    )
}

/// Whether Claude Code still has session `id` for `cwd` (it prunes old
/// sessions on its own retention schedule).
pub(crate) fn session_file_exists(projects_root: &Path, cwd: &Path, id: &str) -> bool {
    projects_root
        .join(sanitize_cwd(cwd))
        .join(format!("{id}.jsonl"))
        .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    const ID_A: &str = "00000000-0000-4000-8000-00000000000a";
    const ID_B: &str = "00000000-0000-4000-8000-00000000000b";

    fn touch(path: &Path) {
        fs::write(path, "{}").unwrap();
    }

    fn sessions_dir(projects_root: &Path, cwd: &Path) -> PathBuf {
        let dir = projects_root.join(sanitize_cwd(cwd));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitize_cwd_replaces_every_non_alphanumeric() {
        assert_eq!(sanitize_cwd(Path::new("/home/x/proj")), "-home-x-proj");
        assert_eq!(sanitize_cwd(Path::new("/a/b.c_d e")), "-a-b-c-d-e");
        assert_eq!(sanitize_cwd(Path::new(r"C:\x\y")), "C--x-y");
        assert_eq!(
            sanitize_cwd(Path::new("/home/someone/.local/share/com.podium.app")),
            "-home-someone--local-share-com-podium-app"
        );
    }

    #[test]
    fn session_ids_must_be_uuid_shaped() {
        assert!(is_valid_session_id(ID_A));
        assert!(!is_valid_session_id("-x"));
        assert!(!is_valid_session_id("sess-a"));
        assert!(!is_valid_session_id("00000000-0000-4000-8000-00000000000g"));
        assert!(!is_valid_session_id(
            "00000000-0000-4000-8000-00000000000a0"
        ));
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
        let dir = sessions_dir(projects_root.path(), cwd);

        touch(&dir.join(format!("{ID_A}.jsonl")));
        let cutoff = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        touch(&dir.join(format!("{ID_B}.jsonl")));

        assert_eq!(
            find_latest_session_id_in(projects_root.path(), cwd, cutoff),
            Some(ID_B.to_string())
        );
    }

    #[test]
    fn ignores_non_jsonl_and_non_uuid_entries() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/another/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        touch(&dir.join("notes.txt"));
        touch(&dir.join("-x.jsonl"));

        assert_eq!(
            find_latest_session_id_in(projects_root.path(), cwd, SystemTime::UNIX_EPOCH),
            None
        );
    }

    #[test]
    fn titled_lookup_picks_the_file_carrying_that_title() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/shared/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        let line = |title: &str, id: &str| {
            format!(
                "{{}}\n{{\"type\":\"custom-title\",\"customTitle\":\"{title}\",\"sessionId\":\"{id}\"}}\n"
            )
        };
        fs::write(
            dir.join(format!("{ID_A}.jsonl")),
            line("podium-aaaaaaaa", ID_A),
        )
        .unwrap();
        fs::write(
            dir.join(format!("{ID_B}.jsonl")),
            line("podium-bbbbbbbb", ID_B),
        )
        .unwrap();

        let find = |t| find_titled_session_id_in(projects_root.path(), cwd, t);
        assert_eq!(find("podium-aaaaaaaa"), Some(ID_A.to_string()));
        assert_eq!(find("podium-bbbbbbbb"), Some(ID_B.to_string()));
        assert_eq!(find("podium-cccccccc"), None);
        assert!(session_file_exists(projects_root.path(), cwd, ID_A));
        assert!(!session_file_exists(
            projects_root.path(),
            cwd,
            "00000000-0000-4000-8000-00000000000c"
        ));
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
            .remember(
                root,
                entry(
                    "agent-1",
                    "/project/root",
                    "00000000-0000-4000-8000-000000000001",
                ),
            )
            .unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "00000000-0000-4000-8000-000000000001");
        assert!(store.list(Path::new("/other/root")).is_empty());
    }

    #[test]
    fn remember_replaces_same_cwd_and_name() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(
                root,
                entry(
                    "agent-1",
                    "/project/root",
                    "00000000-0000-4000-8000-000000000001",
                ),
            )
            .unwrap();
        store
            .remember(
                root,
                entry(
                    "agent-1",
                    "/project/root",
                    "00000000-0000-4000-8000-000000000002",
                ),
            )
            .unwrap();
        let listed = store.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "00000000-0000-4000-8000-000000000002");
    }

    #[test]
    fn clear_drops_everything_for_that_root() {
        let store = SessionMemoryStore::new();
        let root = Path::new("/project/root");
        store
            .remember(
                root,
                entry(
                    "agent-1",
                    "/project/root",
                    "00000000-0000-4000-8000-000000000001",
                ),
            )
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
            .remember(
                root,
                entry(
                    "agent-1",
                    "/project/root",
                    "00000000-0000-4000-8000-000000000001",
                ),
            )
            .unwrap();

        let reloaded = SessionMemoryStore::new();
        reloaded.set_path(file);
        let listed = reloaded.list(root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "00000000-0000-4000-8000-000000000001");
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
