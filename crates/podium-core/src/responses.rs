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

/// Most items the list holds; the oldest go first. The assistant files these
/// from content it read (agent reports, a Slack sweep), so a runaway or
/// prompt-injected session must not grow the file and the pinned list
/// without bound.
// ponytail: flat cap, oldest dropped.
pub const MAX_ITEMS: usize = 100;

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
        // Whitespace collapsed per part: the label is the first line of a
        // reply row and sits inside a one-line bracket on the wire, and an
        // agent name or source may carry a newline.
        let parts: Vec<String> = [&self.agent, &self.project, &self.todo, &self.source]
            .into_iter()
            .filter_map(|p| p.as_deref())
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
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

    // Every mutation hands the new list to `changed` while still holding the
    // lock, so snapshots are published in the order the changes happened: an
    // add racing a dismiss can never publish its older list last and bring
    // the dismissed item back on screen. `changed` must not block (a
    // broadcast send does not).

    /// Add `item` at the top, dropping any open item it supersedes and,
    /// past [`MAX_ITEMS`], the oldest.
    pub fn add(&self, item: ResponseItem, changed: impl FnOnce(&[ResponseItem])) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        inner.items.retain(|old| !old.superseded_by(&item));
        inner.items.insert(0, item);
        inner.items.truncate(MAX_ITEMS);
        save(&inner)?;
        changed(&inner.items);
        Ok(())
    }

    /// Take one item off the list.
    pub fn remove(
        &self,
        id: ResponseId,
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<ResponseItem> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let index = inner
            .items
            .iter()
            .position(|i| i.id == id)
            .ok_or(CoreError::ResponseNotFound)?;
        let item = inner.items.remove(index);
        save(&inner)?;
        changed(&inner.items);
        Ok(item)
    }

    /// Put back an item [`remove`](Self::remove) took, in its place by age —
    /// unless a newer item about the same agent or to-do arrived meanwhile,
    /// which is the one the user should see.
    pub fn restore(&self, item: ResponseItem, changed: impl FnOnce(&[ResponseItem])) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if inner.items.iter().any(|i| item.superseded_by(i)) {
            return;
        }
        let at = inner
            .items
            .iter()
            .position(|i| i.created_at < item.created_at)
            .unwrap_or(inner.items.len());
        inner.items.insert(at, item);
        let _ = save(&inner);
        changed(&inner.items);
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

    /// Add, returning the list `add` announced.
    fn add(store: &ResponseStore, i: ResponseItem) -> Vec<ResponseItem> {
        let mut announced = Vec::new();
        store.add(i, |l| announced = l.to_vec()).unwrap();
        announced
    }

    fn texts(items: &[ResponseItem]) -> Vec<&str> {
        items.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn items_list_newest_first() {
        let store = ResponseStore::new();
        add(&store, item(None, None, "first"));
        add(&store, item(None, None, "second"));
        assert_eq!(texts(&store.list()), vec!["second", "first"]);
    }

    #[test]
    fn a_newer_item_about_the_same_agent_replaces_the_open_one() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        add(&store, item(Some(agent), None, "tests failing"));
        add(&store, item(None, None, "unrelated"));
        let list = add(&store, item(Some(agent), None, "tests green"));
        assert_eq!(texts(&list), vec!["tests green", "unrelated"]);
    }

    #[test]
    fn a_newer_item_about_the_same_to_do_replaces_the_open_one() {
        let store = ResponseStore::new();
        let todo = TodoId::new();
        add(
            &store,
            item(Some(ProcessId::new()), Some(todo), "old agent"),
        );
        let list = add(
            &store,
            item(Some(ProcessId::new()), Some(todo), "new agent"),
        );
        assert_eq!(texts(&list), vec!["new agent"]);
    }

    #[test]
    fn items_without_keys_never_supersede() {
        let store = ResponseStore::new();
        add(&store, item(Some(ProcessId::new()), None, "keyed"));
        add(&store, item(None, None, "a"));
        let list = add(&store, item(None, None, "b"));
        assert_eq!(texts(&list), vec!["b", "a", "keyed"]);
    }

    #[test]
    fn removing_drops_the_item_and_an_unknown_id_fails() {
        let store = ResponseStore::new();
        let kept = add(&store, item(None, None, "kept"))[0].id;
        let gone = add(&store, item(None, None, "gone"))[0].id;
        let mut announced = Vec::new();
        let removed = store.remove(gone, |l| announced = l.to_vec()).unwrap();
        assert_eq!(removed.text, "gone");
        assert_eq!(texts(&announced), vec!["kept"]);
        assert_eq!(store.list()[0].id, kept);
        assert!(matches!(
            store.remove(gone, |_| panic!("nothing changed")),
            Err(CoreError::ResponseNotFound)
        ));
    }

    #[test]
    fn a_restored_item_goes_back_in_its_place_unless_it_was_replaced() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        let mut older = item(None, None, "older");
        older.created_at -= chrono::Duration::minutes(2);
        let mut middle = item(Some(agent), None, "middle");
        middle.created_at -= chrono::Duration::minutes(1);
        add(&store, older);
        let id = add(&store, middle)[0].id;
        add(&store, item(None, None, "newer"));

        let taken = store.remove(id, |_| {}).unwrap();
        store.restore(taken.clone(), |_| {});
        assert_eq!(texts(&store.list()), vec!["newer", "middle", "older"]);

        let taken = store.remove(id, |_| {}).unwrap();
        add(&store, item(Some(agent), None, "replacement"));
        store.restore(taken, |_| panic!("a replaced item stays gone"));
        assert_eq!(texts(&store.list()), vec!["replacement", "newer", "older"]);
    }

    #[test]
    fn the_list_keeps_only_the_newest_items() {
        let store = ResponseStore::new();
        for n in 0..=MAX_ITEMS {
            add(&store, item(None, None, &n.to_string()));
        }
        let list = store.list();
        assert_eq!(list.len(), MAX_ITEMS);
        assert_eq!(list[0].text, MAX_ITEMS.to_string());
        assert_eq!(list[MAX_ITEMS - 1].text, "1");
    }

    #[test]
    fn items_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("assistant-responses.json");
        let store = ResponseStore::new();
        store.set_path(file.clone());
        add(&store, item(None, None, "older"));
        add(&store, item(None, None, "newer"));

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
        let multiline = ResponseItem {
            agent: Some("checkout\n  fix".to_string()),
            project: Some("  ".to_string()),
            ..item(None, None, "x")
        };
        assert_eq!(multiline.label(), "checkout fix");
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
