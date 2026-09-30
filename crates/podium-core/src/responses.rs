//! The assistant's Responses list: the short "needs you" / "FYI" points it
//! files (over MCP, `add_response`) for each agent or to-do update, pinned
//! above the chat so the user does not have to dig them out of the log.
//!
//! One workspace-wide list, persisted as one JSON file in the app data dir so
//! items outlive a restart and a new conversation — they are the user's to
//! clear (tick or reply), not the session's. Writes are atomic (temp file +
//! rename); a missing or corrupt file is an empty list. Until
//! [`ResponseStore::set_path`] is called the list is in-memory only.
//!
//! Names (agent, project, to-do) are resolved once, when the item is added:
//! an item about an agent that has since exited or a project since closed
//! still reads the way it did.

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::ids::{ProcessId, ResponseId, TodoId};

const LOCK_POISONED: &str = "response store lock poisoned";

/// Whether an item asks something of the user or only keeps them posted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ResponseKind {
    /// The user has to decide or act.
    NeedsYou,
    /// Only worth knowing.
    Fyi,
}

/// One item on the Responses list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseItem {
    pub id: ResponseId,
    pub kind: ResponseKind,
    pub text: String,
    /// Project display name at add time.
    pub project: Option<String>,
    /// Agent session name at add time.
    pub agent: Option<String>,
    /// The agent's session colour at add time.
    pub agent_color: Option<String>,
    /// The to-do's text at add time, clipped.
    pub todo: Option<String>,
    /// A free label for an item with no agent or project, e.g. `Slack`.
    pub source: Option<String>,
    /// Supersede key: a newer item about the same agent replaces this one.
    pub process_id: Option<ProcessId>,
    /// Supersede key: a newer item about the same to-do replaces this one.
    pub todo_id: Option<TodoId>,
    pub created_at: DateTime<Utc>,
}

impl ResponseItem {
    /// How the item is named to the user and, in a reply, to the assistant:
    /// whatever of agent, project, to-do and source it has.
    pub fn label(&self) -> String {
        let parts: Vec<&str> = [&self.agent, &self.project, &self.todo, &self.source]
            .into_iter()
            .filter_map(|p| p.as_deref())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            "Responses item".to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// Whether `newer` is about the same agent or the same to-do, so it
    /// replaces this one. An item with neither key never supersedes anything.
    fn superseded_by(&self, newer: &ResponseItem) -> bool {
        (newer.process_id.is_some() && self.process_id == newer.process_id)
            || (newer.todo_id.is_some() && self.todo_id == newer.todo_id)
    }
}

struct Inner {
    path: Option<PathBuf>,
    /// Newest first — the order the panel shows.
    items: Vec<ResponseItem>,
}

/// Thread-safe Responses store shared by the orchestrator.
pub(crate) struct ResponseStore {
    inner: Mutex<Inner>,
}

impl ResponseStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: None,
                items: Vec::new(),
            }),
        }
    }

    /// Point the store at its backing file and load whatever is there.
    pub fn set_path(&self, path: PathBuf) {
        let items = load(&path);
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.path = Some(path);
        inner.items = items;
    }

    /// Every open item, newest first.
    pub fn list(&self) -> Vec<ResponseItem> {
        self.inner.lock().expect(LOCK_POISONED).items.clone()
    }

    pub fn get(&self, id: ResponseId) -> CoreResult<ResponseItem> {
        self.inner
            .lock()
            .expect(LOCK_POISONED)
            .items
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or(CoreError::ResponseNotFound)
    }

    /// Add `item` at the top, dropping any open item it supersedes. Returns
    /// the new list.
    pub fn add(&self, item: ResponseItem) -> CoreResult<Vec<ResponseItem>> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.items.retain(|old| !old.superseded_by(&item));
        inner.items.insert(0, item);
        save(&inner)?;
        Ok(inner.items.clone())
    }

    pub fn remove(&self, id: ResponseId) -> CoreResult<ResponseItem> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let index = inner
            .items
            .iter()
            .position(|i| i.id == id)
            .ok_or(CoreError::ResponseNotFound)?;
        let item = inner.items.remove(index);
        save(&inner)?;
        Ok(item)
    }
}

fn load(path: &std::path::Path) -> Vec<ResponseItem> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("responses file is corrupt, starting fresh: {e}");
        Vec::new()
    })
}

fn save(inner: &Inner) -> CoreResult<()> {
    let Some(path) = &inner.path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(&inner.items)
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode responses: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(process_id: Option<ProcessId>, todo_id: Option<TodoId>, text: &str) -> ResponseItem {
        ResponseItem {
            id: ResponseId::new(),
            kind: ResponseKind::Fyi,
            text: text.to_string(),
            project: Some("Webshop".to_string()),
            agent: Some("checkout-fix".to_string()),
            agent_color: None,
            todo: None,
            source: None,
            process_id,
            todo_id,
            created_at: Utc::now(),
        }
    }

    fn texts(items: &[ResponseItem]) -> Vec<&str> {
        items.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn items_list_newest_first() {
        let store = ResponseStore::new();
        store.add(item(None, None, "first")).unwrap();
        store.add(item(None, None, "second")).unwrap();
        assert_eq!(texts(&store.list()), vec!["second", "first"]);
    }

    #[test]
    fn a_newer_item_about_the_same_agent_replaces_the_open_one() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        store.add(item(Some(agent), None, "tests failing")).unwrap();
        store.add(item(None, None, "unrelated")).unwrap();
        let list = store.add(item(Some(agent), None, "tests green")).unwrap();
        assert_eq!(texts(&list), vec!["tests green", "unrelated"]);
    }

    #[test]
    fn a_newer_item_about_the_same_to_do_replaces_the_open_one() {
        let store = ResponseStore::new();
        let todo = TodoId::new();
        store
            .add(item(Some(ProcessId::new()), Some(todo), "old agent"))
            .unwrap();
        let list = store
            .add(item(Some(ProcessId::new()), Some(todo), "new agent"))
            .unwrap();
        assert_eq!(texts(&list), vec!["new agent"]);
    }

    #[test]
    fn items_without_keys_never_supersede() {
        let store = ResponseStore::new();
        store
            .add(item(Some(ProcessId::new()), None, "keyed"))
            .unwrap();
        store.add(item(None, None, "a")).unwrap();
        let list = store.add(item(None, None, "b")).unwrap();
        assert_eq!(texts(&list), vec!["b", "a", "keyed"]);
    }

    #[test]
    fn removing_drops_the_item_and_an_unknown_id_fails() {
        let store = ResponseStore::new();
        let kept = store.add(item(None, None, "kept")).unwrap()[0].id;
        let gone = store.add(item(None, None, "gone")).unwrap()[0].id;
        assert_eq!(store.remove(gone).unwrap().text, "gone");
        assert_eq!(texts(&store.list()), vec!["kept"]);
        assert!(matches!(
            store.remove(gone),
            Err(CoreError::ResponseNotFound)
        ));
        assert!(matches!(
            store.get(ResponseId::new()),
            Err(CoreError::ResponseNotFound)
        ));
        assert_eq!(store.get(kept).unwrap().text, "kept");
    }

    #[test]
    fn items_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("assistant-responses.json");
        let store = ResponseStore::new();
        store.set_path(file.clone());
        store.add(item(None, None, "older")).unwrap();
        store.add(item(None, None, "newer")).unwrap();

        let reloaded = ResponseStore::new();
        reloaded.set_path(file);
        assert_eq!(texts(&reloaded.list()), vec!["newer", "older"]);
    }

    #[test]
    fn a_corrupt_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("assistant-responses.json");
        std::fs::write(&file, "{ not json").unwrap();
        let store = ResponseStore::new();
        store.set_path(file);
        assert!(store.list().is_empty());
    }

    #[test]
    fn the_label_joins_what_is_known() {
        let mut i = item(None, None, "x");
        i.todo = Some("Fix the login redirect".to_string());
        assert_eq!(i.label(), "checkout-fix · Webshop · Fix the login redirect");
        let bare = ResponseItem {
            agent: None,
            project: None,
            todo: None,
            source: Some("Slack".to_string()),
            ..item(None, None, "x")
        };
        assert_eq!(bare.label(), "Slack");
        let empty = ResponseItem {
            source: None,
            ..bare
        };
        assert_eq!(empty.label(), "Responses item");
    }

    #[test]
    fn items_serialize_camel_case() {
        let json = serde_json::to_value(item(None, None, "x")).unwrap();
        assert_eq!(json["kind"], "fyi");
        assert!(json.get("agentColor").is_some());
        assert!(json.get("createdAt").is_some());
        assert_eq!(
            serde_json::to_value(ResponseKind::NeedsYou).unwrap(),
            "needsYou"
        );
    }
}
