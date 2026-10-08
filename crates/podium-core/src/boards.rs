//! The last successful Git board per project and scope (`mine` / `all`),
//! persisted as one `boards.json` in the app data dir, so the UI paints at
//! once, agents read it without a fetch, and a failed refresh has something to
//! fall back on. Keyed by project root path like the to-do store; writes are
//! atomic (temp file + rename); a missing or corrupt file is an empty store,
//! and without [`BoardStore::set_path`] it is in-memory only.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreResult;
use crate::issues::{movable, Board, ItemKind};

const LOCK_POISONED: &str = "board store lock poisoned";

/// A board as last fetched, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedBoard {
    pub board: Board,
    /// RFC 3339 on the wire.
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RootBoards {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mine: Option<CachedBoard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    all: Option<CachedBoard>,
}

impl RootBoards {
    fn scope(&mut self, mine: bool) -> &mut Option<CachedBoard> {
        if mine {
            &mut self.mine
        } else {
            &mut self.all
        }
    }
}

type BoardMap = BTreeMap<String, RootBoards>;

struct Inner {
    path: Option<PathBuf>,
    boards: BoardMap,
    /// Per project root, bumped by every local edit (move/unassign), so a
    /// fetch of that project that started before one cannot overwrite it with
    /// the pre-edit board. In memory only: a restart has no fetch in flight.
    edits: HashMap<String, u64>,
    /// Numbers each snapshot taken for disk, so a slower older write can
    /// never land over a newer one (see [`BoardStore::write`]).
    saves: u64,
}

/// The store serialised under its lock, written to disk after it is released.
struct Snapshot {
    path: PathBuf,
    json: String,
    seq: u64,
}

pub(crate) struct BoardStore {
    inner: Mutex<Inner>,
    /// Serialises the disk writes and remembers the newest one written; held
    /// only while writing, never together with `inner`'s lock.
    written: Mutex<u64>,
}

impl BoardStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: None,
                boards: BoardMap::new(),
                edits: HashMap::new(),
                saves: 0,
            }),
            written: Mutex::new(0),
        }
    }

    /// Point the store at its backing file and load whatever is there.
    pub fn set_path(&self, path: PathBuf) {
        let boards = load(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.boards = boards;
    }

    pub fn get(&self, root: &Path, mine: bool) -> Option<CachedBoard> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .boards
            .get_mut(&key(root))
            .and_then(|r| r.scope(mine).clone())
    }

    /// This project's edit counter; take it before a fetch and hand it to
    /// [`Self::put`].
    pub fn edits(&self, root: &Path) -> u64 {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner.edits.get(&key(root)).copied().unwrap_or(0)
    }

    /// Store a freshly fetched board (best-effort persisted), unless a local
    /// edit of this project landed since the fetch started (`edits` taken
    /// before it) — that board predates the edit. Returns the stored entry.
    pub fn put(&self, root: &Path, mine: bool, board: Board, edits: u64) -> Option<CachedBoard> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let key = key(root);
        if inner.edits.get(&key).copied().unwrap_or(0) != edits {
            return None;
        }
        let cached = CachedBoard {
            board,
            fetched_at: Utc::now(),
        };
        *inner.boards.entry(key).or_default().scope(mine) = Some(cached.clone());
        let snapshot = snapshot(&mut inner);
        drop(inner);
        self.write(snapshot);
        Some(cached)
    }

    /// After a successful move: put the item in `column` (and, for GitLab, its
    /// new `labels`) in both scopes. `linked` and `fetched_at` stay as they were.
    pub fn apply_move(
        &self,
        root: &Path,
        kind: ItemKind,
        number: u64,
        column: &str,
        labels: Option<&[String]>,
    ) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        *inner.edits.entry(key(root)).or_default() += 1;
        let Some(root_boards) = inner.boards.get_mut(&key(root)) else {
            return;
        };
        for cached in [&mut root_boards.mine, &mut root_boards.all]
            .into_iter()
            .flatten()
        {
            if let Some(item) = cached
                .board
                .items
                .iter_mut()
                .find(|i| i.kind == kind && i.number == number)
            {
                item.column = column.to_string();
                item.movable = movable(kind, column);
                if let Some(labels) = labels {
                    item.labels = labels.to_vec();
                }
            }
        }
        let snapshot = snapshot(&mut inner);
        drop(inner);
        self.write(snapshot);
    }

    /// After a successful unassign: drop `me` from the item's assignees in
    /// both scopes. An issue leaves the `mine` board outright — that board is
    /// the issues assigned to you (its PRs are the ones you wrote, so they stay).
    pub fn apply_unassign(&self, root: &Path, kind: ItemKind, number: u64, me: &str) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        *inner.edits.entry(key(root)).or_default() += 1;
        let Some(root_boards) = inner.boards.get_mut(&key(root)) else {
            return;
        };
        if let Some(mine) = &mut root_boards.mine {
            if kind == ItemKind::Issue {
                mine.board
                    .items
                    .retain(|i| !(i.kind == kind && i.number == number));
            }
        }
        for cached in [&mut root_boards.mine, &mut root_boards.all]
            .into_iter()
            .flatten()
        {
            if let Some(item) = cached
                .board
                .items
                .iter_mut()
                .find(|i| i.kind == kind && i.number == number)
            {
                item.assignees.retain(|a| a != me);
            }
        }
        let snapshot = snapshot(&mut inner);
        drop(inner);
        self.write(snapshot);
    }
}

impl BoardStore {
    /// Best-effort atomic write (temp file, then rename) of a snapshot, unless
    /// a newer one already made it to disk.
    fn write(&self, snapshot: Option<Snapshot>) {
        let Some(snapshot) = snapshot else { return };
        let mut written = self.written.lock().expect(LOCK_POISONED);
        if snapshot.seq <= *written {
            return;
        }
        if let Err(e) = write_atomic(&snapshot.path, &snapshot.json) {
            tracing::warn!("could not save the board cache: {e}");
            return;
        }
        *written = snapshot.seq;
    }
}

fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

fn load(path: &Path) -> BoardMap {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BoardMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("boards file is corrupt, starting fresh: {e}");
        BoardMap::new()
    })
}

/// The store as JSON for disk (taken under the lock); `None` when it is
/// in-memory only or does not encode.
fn snapshot(inner: &mut Inner) -> Option<Snapshot> {
    let path = inner.path.clone()?;
    let json = serde_json::to_string(&inner.boards).ok()?;
    inner.saves += 1;
    Some(Snapshot {
        path,
        json,
        seq: inner.saves,
    })
}

/// Atomic write: temp file, then rename over the old one.
fn write_atomic(path: &Path, json: &str) -> CoreResult<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issues::{BoardItem, Column, IssueProvider, PrCounts};

    fn board() -> Board {
        let item = |kind, number, column: &str| BoardItem {
            kind,
            number,
            title: "t".into(),
            url: "u".into(),
            body: String::new(),
            labels: vec!["bug".into(), "doing".into()],
            updated_at: String::new(),
            column: column.into(),
            draft: false,
            author: "sam".into(),
            movable: true,
            linked: Vec::new(),
            assignees: vec!["sam".into(), "kim".into()],
            approvals: None,
        };
        Board {
            provider: IssueProvider::Gitlab,
            items: vec![
                item(ItemKind::Issue, 1, "label:doing"),
                item(ItemKind::Pr, 2, "review"),
            ],
            columns: vec![Column {
                id: "open".into(),
                title: "Open".into(),
                color: None,
                accepts: vec![ItemKind::Issue],
            }],
            source: Some("Dev".into()),
            hint: None,
            pr_counts: PrCounts::default(),
            me: Some("sam".into()),
        }
    }

    #[test]
    fn round_trips_through_disk_and_applies_moves_to_both_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.json");
        let root = PathBuf::from("/tmp/fixture-project");
        let store = BoardStore::new();
        store.set_path(path.clone());
        assert_eq!(store.get(&root, true), None);
        store.put(&root, true, board(), 0).unwrap();
        store.put(&root, false, board(), 0).unwrap();

        // A second store on the same file sees both scopes, RFC 3339 on disk.
        let reloaded = BoardStore::new();
        reloaded.set_path(path.clone());
        assert_eq!(reloaded.get(&root, true), store.get(&root, true));
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw["/tmp/fixture-project"]["mine"]["fetchedAt"]
            .as_str()
            .unwrap()
            .contains('T'));

        let labels = vec!["bug".to_string()];
        reloaded.apply_move(&root, ItemKind::Issue, 1, "open", Some(&labels));
        reloaded.apply_move(&root, ItemKind::Pr, 2, "done", None);
        for mine in [true, false] {
            let b = reloaded.get(&root, mine).unwrap().board;
            assert_eq!(
                (b.items[0].column.as_str(), &b.items[0].labels),
                ("open", &labels)
            );
            assert_eq!(b.items[1].column, "done");
            assert!(!b.items[1].movable && b.items[0].movable);
        }
        // Persisted too.
        let again = BoardStore::new();
        again.set_path(path);
        assert_eq!(
            again.get(&root, false).unwrap().board.items[0].column,
            "open"
        );
    }

    #[test]
    fn unassign_drops_the_issue_from_mine_and_the_name_everywhere() {
        let root = PathBuf::from("/tmp/fixture-project");
        let store = BoardStore::new();
        store.put(&root, true, board(), 0).unwrap();
        store.put(&root, false, board(), 0).unwrap();
        store.apply_unassign(&root, ItemKind::Issue, 1, "sam");
        store.apply_unassign(&root, ItemKind::Pr, 2, "sam");
        let mine = store.get(&root, true).unwrap().board;
        assert_eq!(mine.items.len(), 1);
        assert_eq!(mine.items[0].assignees, vec!["kim".to_string()]);
        let all = store.get(&root, false).unwrap().board;
        assert_eq!(all.items.len(), 2);
        assert!(all.items.iter().all(|i| i.assignees == ["kim"]));
    }

    #[test]
    fn a_fetch_started_before_an_edit_is_not_stored() {
        let root = PathBuf::from("/tmp/fixture-project");
        let other = PathBuf::from("/tmp/fixture-other");
        let store = BoardStore::new();
        assert!(store
            .put(&root, false, board(), store.edits(&root))
            .is_some());
        let before = store.edits(&root);
        let other_before = store.edits(&other);
        store.apply_move(&root, ItemKind::Issue, 1, "open", None);
        // The in-flight fetch lands after the move: dropped, the move stays.
        assert!(store.put(&root, false, board(), before).is_none());
        let b = store.get(&root, false).unwrap().board;
        assert_eq!(b.items[0].column, "open");
        // Another project's fetch is not affected by this project's edit.
        assert!(store.put(&other, false, board(), other_before).is_some());
        // A fetch started after it is stored as usual.
        assert!(store
            .put(&root, false, board(), store.edits(&root))
            .is_some());
        let before = store.edits(&root);
        store.apply_unassign(&root, ItemKind::Pr, 2, "sam");
        assert!(store.put(&root, false, board(), before).is_none());
    }

    #[test]
    fn an_older_snapshot_never_lands_over_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.json");
        let root = PathBuf::from("/tmp/fixture-project");
        let store = BoardStore::new();
        store.set_path(path.clone());
        let older = {
            let mut inner = store.inner.lock().unwrap();
            snapshot(&mut inner)
        };
        // A later change is written first; the older write then arrives.
        store.put(&root, true, board(), 0).unwrap();
        store.write(older);
        let again = BoardStore::new();
        again.set_path(path);
        assert!(again.get(&root, true).is_some(), "the newer write stands");
    }

    #[test]
    fn corrupt_file_is_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boards.json");
        std::fs::write(&path, "{nope").unwrap();
        let store = BoardStore::new();
        store.set_path(path);
        assert_eq!(store.get(Path::new("/x"), true), None);
    }
}
