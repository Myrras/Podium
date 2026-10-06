//! Finding an interactive agent's Claude Code session id after the fact, and
//! remembering it so the agent can be resumed after a restart.
//!
//! Podium's agents run as real interactive PTYs (the user's terminal view),
//! not the piped `stream-json` mode `assistant.rs` parses for its own
//! session id. Claude Code itself records it in two places this module reads
//! instead of the PTY stream, both under its config dir (`~/.claude`):
//!
//! - `sessions/<pid>.json`, one small file per *live* CLI process, carrying
//!   its session `name` (Podium's `-n podium-<tag>`) and its **current**
//!   `sessionId`. That follows `/clear`, which starts a new session id in a
//!   new transcript file that carries no title (verified against real files).
//! - `projects/<sanitized-cwd>/<session-id>.jsonl`, every session's
//!   transcript (the filename stem is the exact id `--resume` takes), with a
//!   `custom-title` line for a named session. The fallback for a CLI that
//!   does not write the live registry.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::ids::{ScratchpadId, TodoId};

/// Claude Code's own scheme for naming a cwd's project directory: every
/// character that is not ASCII alphanumeric becomes `-` (verified against real
/// `~/.claude/projects/` entries, e.g. `/home/x/.local/com.app` →
/// `-home-x--local-com-app`).
// The cwd is taken in the backend's spelling: in WSL mode Claude Code sees
// `/home/me/app` (or `/mnt/c/...`), not the `\\wsl.localhost\...` path Podium
// holds, and names the directory after that.
// ponytail: Claude Code truncates very long names (~200 chars) with a hash
// suffix; such cwds are not handled and simply find no session.
pub(crate) fn sanitize_cwd(cwd: &Path) -> String {
    crate::platform::backend_path(cwd)
        .unwrap_or_else(|_| cwd.to_string_lossy().into_owned())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Whether `id` is a Claude Code session id: a hyphenated UUID. Anything
/// else — a tampered `session_memory.json`, a stray file — must never reach
/// `--resume`, where a value like `-x` would be read as a flag. The length
/// check pins the hyphenated form (`try_parse` also takes simple, braced and
/// URN spellings).
pub(crate) fn is_valid_session_id(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::try_parse(id).is_ok()
}

/// Slack on the "modified since spawn" filter, for filesystems whose mtime
/// is coarser than the gap between spawn and the first write (HFS+ keeps
/// whole seconds). It only has to be generous: the title match, not the
/// timestamp, decides which session is the agent's.
const MTIME_SLACK: Duration = Duration::from_secs(5);

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

/// The current session id of the live CLI process named `name`, from Claude
/// Code's `sessions/` registry (one `<pid>.json` per running process). It is
/// the authority while the agent runs: it tracks `/clear`, which the
/// transcript title does not. The files are a few hundred bytes each, so
/// reading them all costs nothing at exit. Should two carry the name (a
/// stale file from a crashed process), the most recently updated wins.
pub(crate) fn find_live_session_id(sessions_dir: &Path, name: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct LiveSession {
        session_id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        updated_at: u64,
    }
    std::fs::read_dir(sessions_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|raw| serde_json::from_str::<LiveSession>(&raw).ok())
        .filter(|s| s.name.as_deref() == Some(name) && is_valid_session_id(&s.session_id))
        .max_by_key(|s| s.updated_at)
        .map(|s| s.session_id)
}

/// The newest session file for `cwd` that Claude Code tagged with session
/// name `title` (`-n`): it writes a `{"type":"custom-title","customTitle":…}`
/// line into the session's jsonl. This tells apart several agents sharing one
/// cwd — and a session the user runs themselves in the same repo, which has
/// no Podium title. The closing quote is left off so a variant suffix Claude
/// Code appends on a name clash still matches. A fallback behind
/// [`find_live_session_id`]: after `/clear` the titled file is the
/// *previous* session.
///
/// Only files modified since `after` (the agent's spawn time, less
/// [`MTIME_SLACK`]) are read, line by line and only up to the title: a
/// running agent's session has been written to since it started, and session
/// files can be tens of MB each, read synchronously at app exit.
pub(crate) fn find_titled_session_id_in(
    projects_root: &Path,
    cwd: &Path,
    title: &str,
    after: SystemTime,
) -> Option<String> {
    let needle = format!("\"customTitle\":\"{title}");
    let cutoff = after
        .checked_sub(MTIME_SLACK)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    session_files(projects_root, cwd)
        .into_iter()
        .filter(|(modified, ..)| *modified >= cutoff)
        .filter(|(_, _, path)| file_contains_line_with(path, &needle))
        .max_by_key(|(t, ..)| *t)
        .map(|(_, id, _)| id)
}

/// Whether any line of the file at `path` contains `needle`, stopping at the
/// first match.
fn file_contains_line_with(path: &Path, needle: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    std::io::BufReader::new(file)
        .split(b'\n')
        .map_while(Result::ok)
        .any(|line| String::from_utf8_lossy(&line).contains(needle))
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
        // Right length and alphabet, but no UUID.
        assert!(!is_valid_session_id(&"-".repeat(36)));
        assert!(!is_valid_session_id("-0000000-0000-4000-8000-00000000000a"));
        // A UUID, but not in the hyphenated form Claude Code names files by.
        assert!(!is_valid_session_id(
            "{00000000-0000-4000-8000-00000000000a}"
        ));
        assert!(!is_valid_session_id("00000000000040008000000000000000"));
    }

    fn titled(title: &str, id: &str) -> String {
        format!("{{}}\n{{\"type\":\"custom-title\",\"customTitle\":\"{title}\",\"sessionId\":\"{id}\"}}\n")
    }

    #[test]
    fn missing_directory_returns_none() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/nonexistent/project/cwd");
        assert_eq!(
            find_titled_session_id_in(
                projects_root.path(),
                cwd,
                "podium-aaaaaaaa",
                SystemTime::UNIX_EPOCH
            ),
            None
        );
    }

    #[test]
    fn ignores_non_jsonl_and_non_uuid_entries() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/another/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        let body = titled("podium-aaaaaaaa", ID_A);
        fs::write(dir.join("notes.txt"), &body).unwrap();
        fs::write(dir.join("-x.jsonl"), &body).unwrap();
        touch(&dir.join(format!("{ID_B}.jsonl")));

        assert_eq!(
            find_titled_session_id_in(
                projects_root.path(),
                cwd,
                "podium-aaaaaaaa",
                SystemTime::UNIX_EPOCH
            ),
            None
        );
    }

    #[test]
    fn titled_lookup_skips_files_untouched_since_the_spawn() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/stale/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        let path = dir.join(format!("{ID_A}.jsonl"));
        fs::write(&path, titled("podium-aaaaaaaa", ID_A)).unwrap();
        let spawn = SystemTime::now();
        // Last written well before this agent started.
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(spawn - Duration::from_secs(3600))
            .unwrap();

        let find =
            |after| find_titled_session_id_in(projects_root.path(), cwd, "podium-aaaaaaaa", after);
        assert_eq!(find(spawn), None);
        // Within the slack for coarse mtimes it still counts.
        assert_eq!(
            find(spawn - Duration::from_secs(3600) + MTIME_SLACK / 2),
            Some(ID_A.to_string())
        );
    }

    #[test]
    fn titled_lookup_picks_the_file_carrying_that_title() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/shared/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        fs::write(
            dir.join(format!("{ID_A}.jsonl")),
            titled("podium-aaaaaaaa", ID_A),
        )
        .unwrap();
        fs::write(
            dir.join(format!("{ID_B}.jsonl")),
            titled("podium-bbbbbbbb", ID_B),
        )
        .unwrap();

        let find =
            |t| find_titled_session_id_in(projects_root.path(), cwd, t, SystemTime::UNIX_EPOCH);
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

    fn live(name: &str, id: &str, updated_at: u64) -> String {
        format!(
            "{{\"pid\":4242,\"sessionId\":\"{id}\",\"cwd\":\"/a/b\",\"kind\":\"interactive\",\"name\":\"{name}\",\"updatedAt\":{updated_at}}}"
        )
    }

    #[test]
    fn live_registry_maps_a_name_to_its_current_session() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("101.json"),
            live("podium-aaaaaaaa", ID_A, 1),
        )
        .unwrap();
        fs::write(
            dir.path().join("102.json"),
            live("podium-bbbbbbbb", ID_B, 1),
        )
        .unwrap();
        // Not JSON, not a session file, an unnamed session: all skipped.
        fs::write(dir.path().join("103.json"), "garbage").unwrap();
        fs::write(dir.path().join("104.key"), live("podium-cccccccc", ID_A, 1)).unwrap();
        fs::write(
            dir.path().join("105.json"),
            format!("{{\"sessionId\":\"{ID_B}\"}}"),
        )
        .unwrap();

        let find = |n| find_live_session_id(dir.path(), n);
        assert_eq!(find("podium-aaaaaaaa"), Some(ID_A.to_string()));
        assert_eq!(find("podium-bbbbbbbb"), Some(ID_B.to_string()));
        assert_eq!(find("podium-cccccccc"), None);
        assert_eq!(find_live_session_id(&dir.path().join("missing"), "x"), None);
    }

    #[test]
    fn live_registry_prefers_the_latest_update_and_refuses_non_uuids() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("1.json"), live("podium-aaaaaaaa", ID_A, 5)).unwrap();
        fs::write(dir.path().join("2.json"), live("podium-aaaaaaaa", ID_B, 9)).unwrap();
        fs::write(dir.path().join("3.json"), live("podium-dddddddd", "-x", 1)).unwrap();
        assert_eq!(
            find_live_session_id(dir.path(), "podium-aaaaaaaa"),
            Some(ID_B.to_string())
        );
        assert_eq!(find_live_session_id(dir.path(), "podium-dddddddd"), None);
    }

    #[test]
    fn titled_lookup_finds_a_title_far_into_a_large_file() {
        let projects_root = tempfile::tempdir().unwrap();
        let cwd = Path::new("/big/project");
        let dir = sessions_dir(projects_root.path(), cwd);
        let mut body = "{\"type\":\"user\"}\n".repeat(10_000);
        body.push_str(&titled("podium-aaaaaaaa", ID_A));
        fs::write(dir.join(format!("{ID_A}.jsonl")), body).unwrap();
        assert_eq!(
            find_titled_session_id_in(
                projects_root.path(),
                cwd,
                "podium-aaaaaaaa",
                SystemTime::UNIX_EPOCH
            ),
            Some(ID_A.to_string())
        );
    }
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
    /// Absolute cwd the agent ran in (project root or a worktree under it).
    pub cwd: PathBuf,
    /// Claude Code's own session id, as `--resume` expects it.
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
            list.retain(|e| !(e.cwd == entry.cwd && e.name == entry.name));
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
            todo_ids: Vec::new(),
            scratchpad_ids: Vec::new(),
            args: None,
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
    fn remember_all_writes_every_entry_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session_memory.json");
        let store = SessionMemoryStore::new();
        store.set_path(file.clone());
        store
            .remember_all(vec![
                (
                    PathBuf::from("/root/a"),
                    entry("agent-1", "/root/a", "00000000-0000-4000-8000-000000000001"),
                ),
                (
                    PathBuf::from("/root/b"),
                    entry("agent-2", "/root/b", "00000000-0000-4000-8000-000000000002"),
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
                    entry("agent-1", root, "00000000-0000-4000-8000-000000000001"),
                )
                .unwrap();
        }
        store.clear_all().unwrap();
        assert!(store.list(Path::new("/root/a")).is_empty());
        assert!(store.list(Path::new("/root/b")).is_empty());
    }

    #[test]
    fn entries_expire_after_max_age() {
        let e = entry("agent-1", "/root", "00000000-0000-4000-8000-000000000001");
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
