//! Per-project to-do lists, persisted as one JSON file in the app data dir.
//!
//! To-dos are keyed by the project's **root path** (not the per-run
//! [`ProjectId`]), so they survive app restarts and project close/re-open.
//! Writes are atomic (temp file + rename); a missing or corrupt file is
//! treated as an empty store. Until [`TodoStore::set_path`] is called the
//! store is in-memory only (tests, or a data dir that failed to resolve).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::assignment::AssignedAgent;
use crate::error::{CoreError, CoreResult};
use crate::ids::{CommentId, LinkId, ProjectId, TodoId};

const LOCK_POISONED: &str = "todo store lock poisoned";

/// One progress note on a to-do. Agents post these over MCP (`comment_todo`)
/// so the user — and other agents — can track what has been done.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoComment {
    /// Stable id so a comment can be edited or removed. Defaults to a fresh
    /// id when loading files written before comments had ids.
    #[serde(default)]
    pub id: CommentId,
    /// Who left the note (e.g. an agent's name); never blank.
    pub author: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
    /// When the note was last edited, if ever (`None` for untouched notes).
    #[serde(default)]
    pub edited_at: Option<DateTime<Utc>>,
}

/// An issue/PR (or other) link pinned to the top of a to-do. Agents add
/// these over MCP (`add_todo_link`) when they open a GitLab issue or MR/PR
/// while working, so the user can jump straight to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoLink {
    /// Stable id so a link can be removed. Defaults to a fresh id when
    /// loading files written before links had ids.
    #[serde(default)]
    pub id: LinkId,
    /// Human-readable label (e.g. `"#42 Fix login"`); never blank.
    pub label: String,
    /// The `http(s)` URL to open.
    pub url: String,
    pub created_at: DateTime<Utc>,
}

/// Read-only snapshot of one to-do, for listing over IPC/MCP.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoInfo {
    pub id: TodoId,
    pub project_id: ProjectId,
    pub text: String,
    /// Longer detail an agent may keep current as scope evolves.
    pub description: Option<String>,
    pub done: bool,
    pub created_at: DateTime<Utc>,
    /// When the to-do was last marked done (`None` while open). Drives the
    /// next-day auto-archive.
    pub done_at: Option<DateTime<Utc>>,
    /// Whether the to-do is archived (hidden from the main list, shown in the
    /// Archive view).
    pub archived: bool,
    /// When the to-do was archived (`None` while active).
    pub archived_at: Option<DateTime<Utc>>,
    /// Issue/PR links pinned to the top of the to-do, oldest first.
    pub links: Vec<TodoLink>,
    /// Progress notes, oldest first.
    pub comments: Vec<TodoComment>,
    /// The agent currently working on this to-do, if any. Runtime-only, so
    /// it is `None` after a restart until an agent is (re)assigned.
    pub assigned_agent: Option<AssignedAgent>,
}

/// One to-do as persisted on disk; the project association is the map key
/// (root path), so the per-run `ProjectId` is attached only at read time.
/// `description`/`comments` default so files written before they existed
/// still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTodo {
    id: TodoId,
    text: String,
    #[serde(default)]
    description: Option<String>,
    done: bool,
    created_at: DateTime<Utc>,
    #[serde(default)]
    done_at: Option<DateTime<Utc>>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    archived_at: Option<DateTime<Utc>>,
    #[serde(default)]
    links: Vec<TodoLink>,
    #[serde(default)]
    comments: Vec<TodoComment>,
}

impl StoredTodo {
    fn info(&self, project_id: ProjectId) -> TodoInfo {
        TodoInfo {
            id: self.id,
            project_id,
            text: self.text.clone(),
            description: self.description.clone(),
            done: self.done,
            created_at: self.created_at,
            done_at: self.done_at,
            archived: self.archived,
            archived_at: self.archived_at,
            links: self.links.clone(),
            comments: self.comments.clone(),
            // Assignments live in the orchestrator (runtime-only); listing
            // code enriches this after fetching from the store.
            assigned_agent: None,
        }
    }
}

/// Every project's to-dos, keyed by project root path (BTreeMap for a
/// stable on-disk order).
type TodoMap = BTreeMap<String, Vec<StoredTodo>>;

struct Inner {
    path: Option<PathBuf>,
    todos: TodoMap,
}

/// Thread-safe to-do store shared by the orchestrator's public API.
pub(crate) struct TodoStore {
    inner: Mutex<Inner>,
}

impl TodoStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: None,
                todos: TodoMap::new(),
            }),
        }
    }

    /// Point the store at its backing file and load whatever is there.
    /// Any in-memory to-dos accumulated before this call are replaced.
    pub fn set_path(&self, path: PathBuf) {
        let todos = load(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.todos = todos;
    }

    /// The active (non-archived) to-dos for a project. Before listing, any
    /// done to-do whose completion fell on an earlier day than today is
    /// auto-archived, so yesterday's finished items drop out of the list the
    /// next day. A best-effort save persists that transition.
    pub fn list(&self, project_id: ProjectId, root: &Path) -> Vec<TodoInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let changed = auto_archive_stale_done(&mut inner, root, Utc::now());
        if changed {
            let _ = save(&inner);
        }
        inner
            .todos
            .get(&key(root))
            .map(|items| {
                items
                    .iter()
                    .filter(|t| !t.archived)
                    .map(|t| t.info(project_id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The archived to-dos for a project, most recently archived first.
    pub fn list_archived(&self, project_id: ProjectId, root: &Path) -> Vec<TodoInfo> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        let mut archived: Vec<TodoInfo> = inner
            .todos
            .get(&key(root))
            .map(|items| {
                items
                    .iter()
                    .filter(|t| t.archived)
                    .map(|t| t.info(project_id))
                    .collect()
            })
            .unwrap_or_default();
        archived.sort_by_key(|t| std::cmp::Reverse(t.archived_at));
        archived
    }

    pub fn add(&self, project_id: ProjectId, root: &Path, text: &str) -> CoreResult<TodoInfo> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "to-do text must not be empty".to_string(),
            ));
        }
        let todo = StoredTodo {
            id: TodoId::new(),
            text: text.to_string(),
            description: None,
            done: false,
            created_at: Utc::now(),
            done_at: None,
            archived: false,
            archived_at: None,
            links: Vec::new(),
            comments: Vec::new(),
        };
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.todos.entry(key(root)).or_default().push(todo.clone());
        save(&inner)?;
        Ok(todo.info(project_id))
    }

    /// One to-do by id, if it exists (used to seed an agent's launch prompt).
    pub fn get(&self, project_id: ProjectId, root: &Path, id: TodoId) -> Option<TodoInfo> {
        let inner = self.inner.lock().expect(LOCK_POISONED);
        inner
            .todos
            .get(&key(root))
            .and_then(|items| items.iter().find(|t| t.id == id))
            .map(|t| t.info(project_id))
    }

    /// Revise a to-do's text and/or description. Each `Some` field is
    /// applied; a blank `description` clears it; blank replacement `text` is
    /// rejected. At least one field must be provided.
    pub fn update(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        text: Option<&str>,
        description: Option<&str>,
    ) -> CoreResult<TodoInfo> {
        if text.is_none() && description.is_none() {
            return Err(CoreError::InvalidInput(
                "update must set text and/or description".to_string(),
            ));
        }
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        if let Some(text) = text {
            let text = text.trim();
            if text.is_empty() {
                return Err(CoreError::InvalidInput(
                    "to-do text must not be empty".to_string(),
                ));
            }
            todo.text = text.to_string();
        }
        if let Some(description) = description {
            let description = description.trim();
            todo.description = (!description.is_empty()).then(|| description.to_string());
        }
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Append a progress note. Blank text is rejected; a blank author
    /// defaults to `"agent"`.
    pub fn add_comment(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        author: &str,
        text: &str,
    ) -> CoreResult<TodoInfo> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "comment text must not be empty".to_string(),
            ));
        }
        let author = author.trim();
        let author = if author.is_empty() { "agent" } else { author };
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        todo.comments.push(TodoComment {
            id: CommentId::new(),
            author: author.to_string(),
            text: text.to_string(),
            created_at: Utc::now(),
            edited_at: None,
        });
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Revise a comment's text. Blank text is rejected; `edited_at` is stamped.
    pub fn edit_comment(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        comment_id: CommentId,
        text: &str,
    ) -> CoreResult<TodoInfo> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CoreError::InvalidInput(
                "comment text must not be empty".to_string(),
            ));
        }
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        let comment = todo
            .comments
            .iter_mut()
            .find(|c| c.id == comment_id)
            .ok_or(CoreError::CommentNotFound)?;
        comment.text = text.to_string();
        comment.edited_at = Some(Utc::now());
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Remove a comment from a to-do.
    pub fn remove_comment(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        comment_id: CommentId,
    ) -> CoreResult<TodoInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        let before = todo.comments.len();
        todo.comments.retain(|c| c.id != comment_id);
        if todo.comments.len() == before {
            return Err(CoreError::CommentNotFound);
        }
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Pin an issue/PR link to a to-do. Blank label/url are rejected and the
    /// url must be `http(s)`. A blank label falls back to the url. Adding a
    /// link whose url already exists is idempotent (no duplicate).
    pub fn add_link(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        label: &str,
        url: &str,
    ) -> CoreResult<TodoInfo> {
        let url = url.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(CoreError::InvalidInput(
                "link url must start with http:// or https://".to_string(),
            ));
        }
        let label = label.trim();
        let label = if label.is_empty() { url } else { label };
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        if !todo.links.iter().any(|l| l.url == url) {
            todo.links.push(TodoLink {
                id: LinkId::new(),
                label: label.to_string(),
                url: url.to_string(),
                created_at: Utc::now(),
            });
        }
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Remove a pinned link from a to-do.
    pub fn remove_link(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        link_id: LinkId,
    ) -> CoreResult<TodoInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        let before = todo.links.len();
        todo.links.retain(|l| l.id != link_id);
        if todo.links.len() == before {
            return Err(CoreError::LinkNotFound);
        }
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    pub fn set_done(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        done: bool,
    ) -> CoreResult<TodoInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        todo.done = done;
        // Stamp the completion time so next-day auto-archive can fire; clear
        // it when reopening.
        todo.done_at = if done { Some(Utc::now()) } else { None };
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    /// Archive or unarchive a to-do (regardless of its done state). Archiving
    /// stamps `archived_at`; unarchiving clears it and returns the item to the
    /// active list.
    ///
    /// Restoring a *done* item also reopens it (clears `done`/`done_at`).
    /// Otherwise `auto_archive_stale_done` — which sweeps done items completed
    /// on an earlier day — would re-archive it on the very next `list()` (the
    /// refresh the restore itself triggers), so Restore would appear to do
    /// nothing. An open item is never auto-archived, so reopening keeps it in
    /// the active list until the user acts on it again.
    pub fn set_archived(
        &self,
        project_id: ProjectId,
        root: &Path,
        id: TodoId,
        archived: bool,
    ) -> CoreResult<TodoInfo> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let todo = inner
            .todos
            .get_mut(&key(root))
            .and_then(|items| items.iter_mut().find(|t| t.id == id))
            .ok_or(CoreError::TodoNotFound)?;
        todo.archived = archived;
        todo.archived_at = if archived { Some(Utc::now()) } else { None };
        if !archived && todo.done {
            todo.done = false;
            todo.done_at = None;
        }
        let info = todo.info(project_id);
        save(&inner)?;
        Ok(info)
    }

    pub fn remove(&self, root: &Path, id: TodoId) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let items = inner
            .todos
            .get_mut(&key(root))
            .ok_or(CoreError::TodoNotFound)?;
        let before = items.len();
        items.retain(|t| t.id != id);
        if items.len() == before {
            return Err(CoreError::TodoNotFound);
        }
        save(&inner)?;
        Ok(())
    }
}

fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// Archive any done, not-yet-archived to-do whose completion date is strictly
/// before `now`'s date. Returns whether anything changed (so the caller can
/// persist). Dates are compared in UTC — good enough for a "yesterday's done
/// items drop off" rule.
fn auto_archive_stale_done(inner: &mut Inner, root: &Path, now: DateTime<Utc>) -> bool {
    let today = now.date_naive();
    let Some(items) = inner.todos.get_mut(&key(root)) else {
        return false;
    };
    let mut changed = false;
    for t in items.iter_mut() {
        if t.done && !t.archived {
            let done_day = t.done_at.unwrap_or(t.created_at).date_naive();
            if done_day < today {
                t.archived = true;
                t.archived_at = Some(now);
                changed = true;
            }
        }
    }
    changed
}

/// Read the map from disk; a missing or corrupt file is an empty store.
fn load(path: &Path) -> TodoMap {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return TodoMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("todos file is corrupt, starting fresh: {e}");
        TodoMap::new()
    })
}

/// Atomically persist the map: write a temp file, then rename it over the
/// old one so a crash mid-write can never leave a torn file. A store without
/// a path (in-memory) is a no-op.
fn save(inner: &Inner) -> CoreResult<()> {
    let Some(path) = &inner.path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&inner.todos)
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode to-dos: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (ProjectId, PathBuf) {
        (ProjectId::new(), PathBuf::from("/tmp/fixture-project"))
    }

    #[test]
    fn add_list_toggle_remove_round_trip() {
        let store = TodoStore::new();
        let (project, root) = ids();

        let added = store.add(project, &root, "  write tests  ").unwrap();
        assert_eq!(added.text, "write tests");
        assert!(!added.done);

        let listed = store.list(project, &root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, added.id);

        let toggled = store.set_done(project, &root, added.id, true).unwrap();
        assert!(toggled.done);
        assert!(store.list(project, &root)[0].done);

        store.remove(&root, added.id).unwrap();
        assert!(store.list(project, &root).is_empty());
    }

    #[test]
    fn update_edits_text_and_description() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let added = store.add(project, &root, "draft").unwrap();
        assert!(added.description.is_none());

        let updated = store
            .update(
                project,
                &root,
                added.id,
                Some("  ship it  "),
                Some(" details "),
            )
            .unwrap();
        assert_eq!(updated.text, "ship it");
        assert_eq!(updated.description.as_deref(), Some("details"));

        // A blank description clears it; text left untouched.
        let cleared = store
            .update(project, &root, added.id, None, Some("  "))
            .unwrap();
        assert_eq!(cleared.text, "ship it");
        assert!(cleared.description.is_none());

        assert!(matches!(
            store.update(project, &root, added.id, Some("  "), None),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            store.update(project, &root, added.id, None, None),
            Err(CoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn comments_append_in_order_with_author_fallback() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let added = store.add(project, &root, "task").unwrap();

        store
            .add_comment(project, &root, added.id, "claude", "started")
            .unwrap();
        let after = store
            .add_comment(project, &root, added.id, "  ", "  done  ")
            .unwrap();
        assert_eq!(after.comments.len(), 2);
        assert_eq!(after.comments[0].author, "claude");
        assert_eq!(after.comments[0].text, "started");
        assert_eq!(after.comments[1].author, "agent");
        assert_eq!(after.comments[1].text, "done");

        assert!(matches!(
            store.add_comment(project, &root, added.id, "x", "   "),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            store.add_comment(project, &root, TodoId::new(), "x", "y"),
            Err(CoreError::TodoNotFound)
        ));
    }

    #[test]
    fn comments_can_be_edited_and_removed_by_id() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let added = store.add(project, &root, "task").unwrap();
        let after = store
            .add_comment(project, &root, added.id, "claude", "frist draft")
            .unwrap();
        let comment_id = after.comments[0].id;
        assert!(after.comments[0].edited_at.is_none());

        let edited = store
            .edit_comment(project, &root, added.id, comment_id, "  first draft  ")
            .unwrap();
        assert_eq!(edited.comments[0].text, "first draft");
        assert!(edited.comments[0].edited_at.is_some());

        // Blank replacement text and unknown ids are rejected.
        assert!(matches!(
            store.edit_comment(project, &root, added.id, comment_id, "   "),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            store.edit_comment(project, &root, added.id, CommentId::new(), "x"),
            Err(CoreError::CommentNotFound)
        ));

        let removed = store
            .remove_comment(project, &root, added.id, comment_id)
            .unwrap();
        assert!(removed.comments.is_empty());
        assert!(matches!(
            store.remove_comment(project, &root, added.id, comment_id),
            Err(CoreError::CommentNotFound)
        ));
    }

    #[test]
    fn links_are_added_deduped_and_removed() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let added = store.add(project, &root, "task").unwrap();
        assert!(added.links.is_empty());

        let url = "https://gitlab.example.com/acme/web/-/issues/42";
        let after = store
            .add_link(project, &root, added.id, "  #42 Fix login  ", url)
            .unwrap();
        assert_eq!(after.links.len(), 1);
        assert_eq!(after.links[0].label, "#42 Fix login");
        assert_eq!(after.links[0].url, url);

        // Same url again is idempotent (no duplicate).
        let again = store
            .add_link(project, &root, added.id, "dup", url)
            .unwrap();
        assert_eq!(again.links.len(), 1);

        // A blank label falls back to the url; non-http(s) is rejected.
        let blank = store
            .add_link(project, &root, added.id, "   ", "https://example.com/x")
            .unwrap();
        assert_eq!(blank.links[1].label, "https://example.com/x");
        assert!(matches!(
            store.add_link(project, &root, added.id, "x", "ftp://nope"),
            Err(CoreError::InvalidInput(_))
        ));

        let link_id = after.links[0].id;
        let removed = store
            .remove_link(project, &root, added.id, link_id)
            .unwrap();
        assert_eq!(removed.links.len(), 1);
        assert!(removed.links.iter().all(|l| l.id != link_id));
        assert!(matches!(
            store.remove_link(project, &root, added.id, link_id),
            Err(CoreError::LinkNotFound)
        ));
    }

    #[test]
    fn manual_archive_hides_from_list_and_unarchive_restores() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let a = store.add(project, &root, "keep").unwrap();
        let b = store.add(project, &root, "archive me").unwrap();

        let archived = store.set_archived(project, &root, b.id, true).unwrap();
        assert!(archived.archived);
        assert!(archived.archived_at.is_some());

        // The active list drops it; the archived list holds it.
        let active = store.list(project, &root);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, a.id);
        let arch = store.list_archived(project, &root);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch[0].id, b.id);

        // Unarchiving returns it to the active list.
        let restored = store.set_archived(project, &root, b.id, false).unwrap();
        assert!(!restored.archived);
        assert!(restored.archived_at.is_none());
        assert_eq!(store.list(project, &root).len(), 2);
        assert!(store.list_archived(project, &root).is_empty());
    }

    #[test]
    fn done_todos_from_a_previous_day_auto_archive_on_list() {
        use chrono::Duration;
        let store = TodoStore::new();
        let (project, root) = ids();
        let yesterday_done = store.add(project, &root, "done yesterday").unwrap();
        let today_done = store.add(project, &root, "done today").unwrap();
        let open = store.add(project, &root, "still open").unwrap();

        // Mark two done, then back-date one completion into yesterday.
        store
            .set_done(project, &root, yesterday_done.id, true)
            .unwrap();
        store.set_done(project, &root, today_done.id, true).unwrap();
        {
            let mut inner = store.inner.lock().unwrap();
            let items = inner.todos.get_mut(&key(&root)).unwrap();
            let t = items
                .iter_mut()
                .find(|t| t.id == yesterday_done.id)
                .unwrap();
            t.done_at = Some(Utc::now() - Duration::days(1));
        }

        // Listing auto-archives only yesterday's done item.
        let active = store.list(project, &root);
        let ids: Vec<_> = active.iter().map(|t| t.id).collect();
        assert!(ids.contains(&today_done.id), "today's done item stays");
        assert!(ids.contains(&open.id), "open item stays");
        assert!(
            !ids.contains(&yesterday_done.id),
            "yesterday's done item is archived"
        );
        let arch = store.list_archived(project, &root);
        assert_eq!(arch.len(), 1);
        assert_eq!(arch[0].id, yesterday_done.id);
    }

    #[test]
    fn restoring_a_stale_done_todo_keeps_it_active() {
        use chrono::Duration;
        let store = TodoStore::new();
        let (project, root) = ids();
        let t = store.add(project, &root, "finished last week").unwrap();

        // Complete it and back-date the completion so it auto-archives.
        store.set_done(project, &root, t.id, true).unwrap();
        {
            let mut inner = store.inner.lock().unwrap();
            let items = inner.todos.get_mut(&key(&root)).unwrap();
            let item = items.iter_mut().find(|i| i.id == t.id).unwrap();
            item.done_at = Some(Utc::now() - Duration::days(3));
        }
        store.list(project, &root); // triggers auto-archive
        assert_eq!(store.list_archived(project, &root).len(), 1);

        // Restoring reopens it and it must survive the next list() (the refresh
        // the restore triggers) instead of being swept straight back.
        let restored = store.set_archived(project, &root, t.id, false).unwrap();
        assert!(!restored.archived);
        assert!(!restored.done, "restore reopens a done item");
        let active = store.list(project, &root);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, t.id);
        assert!(store.list_archived(project, &root).is_empty());
    }

    #[test]
    fn get_returns_the_todo_or_none() {
        let store = TodoStore::new();
        let (project, root) = ids();
        let added = store.add(project, &root, "task").unwrap();
        assert_eq!(store.get(project, &root, added.id).unwrap().id, added.id);
        assert!(store.get(project, &root, TodoId::new()).is_none());
    }

    #[test]
    fn rejects_empty_text_and_unknown_ids() {
        let store = TodoStore::new();
        let (project, root) = ids();

        assert!(matches!(
            store.add(project, &root, "   "),
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            store.set_done(project, &root, TodoId::new(), true),
            Err(CoreError::TodoNotFound)
        ));
        assert!(matches!(
            store.remove(&root, TodoId::new()),
            Err(CoreError::TodoNotFound)
        ));
    }

    #[test]
    fn todos_are_scoped_per_project_root() {
        let store = TodoStore::new();
        let project = ProjectId::new();
        let root_a = PathBuf::from("/tmp/fixture-a");
        let root_b = PathBuf::from("/tmp/fixture-b");

        store.add(project, &root_a, "only in a").unwrap();
        assert_eq!(store.list(project, &root_a).len(), 1);
        assert!(store.list(project, &root_b).is_empty());
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("todos.json");
        let (project, root) = ids();

        let store = TodoStore::new();
        store.set_path(file.clone());
        let added = store.add(project, &root, "survive a restart").unwrap();
        store.set_done(project, &root, added.id, true).unwrap();

        let reloaded = TodoStore::new();
        reloaded.set_path(file);
        let listed = reloaded.list(project, &root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].text, "survive a restart");
        assert!(listed[0].done);
    }

    #[test]
    fn corrupt_file_is_an_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("todos.json");
        std::fs::write(&file, "not json").unwrap();

        let store = TodoStore::new();
        store.set_path(file);
        let (project, root) = ids();
        assert!(store.list(project, &root).is_empty());
    }
}
