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
use serde::{Deserialize, Serialize, Serializer};

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

/// Where an item stands with the user. A dismissed item stays on disk, so a
/// re-file with the same key and fingerprint does not bring it back, but is
/// never shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseState {
    #[default]
    Open,
    Parked,
    Dismissed,
}

/// Why an item is on the list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseReason {
    Question,
    Crashed,
    Review,
    Message,
    #[default]
    Other,
}

/// What an [`upsert`](ResponseStore::upsert) did, so a caller can skip
/// notifying about an item the user already knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    /// A key the list did not have.
    New,
    /// Same key, new fingerprint: replaced and back to open.
    Changed,
    /// Same key and fingerprint: nothing written, state kept.
    Unchanged,
}

/// One item on the Responses list, as stored. What crosses to the UI (and
/// back to the assistant over MCP) is a [`LabelledResponse`].
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
    /// Identity: `agent:<processId>`, `msg:<slack|teams>:<conversation>:<thread>`,
    /// or for what the assistant files [`legacy_key`]. Empty only in a file
    /// from before keys; [`load`] derives it.
    #[serde(default)]
    pub key: String,
    /// What "it changed" means for this key; empty allowed.
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub state: ResponseState,
    #[serde(default)]
    pub reason: ResponseReason,
    /// Slack/Teams author; `source` is the place.
    #[serde(default)]
    pub sender: Option<String>,
}

/// The key of an item the assistant (or Podium) files itself: its old
/// supersede identity, the agent before the to-do, else the item alone.
pub fn legacy_key(
    id: ResponseId,
    process_id: Option<ProcessId>,
    todo_id: Option<TodoId>,
) -> String {
    match (process_id, todo_id) {
        (Some(p), _) => format!("process:{p}"),
        (None, Some(t)) => format!("todo:{t}"),
        (None, None) => format!("item:{id}"),
    }
}

fn is_legacy(key: &str) -> bool {
    ["process:", "todo:", "item:"]
        .iter()
        .any(|p| key.starts_with(p))
}

/// A [`ResponseItem`] plus its computed [`label`](ResponseItem::label), the
/// shape it has on the wire, so the panel shows the same name a reply
/// carries back to the session. Never written to disk.
#[derive(Debug, Clone, Serialize)]
pub struct LabelledResponse {
    #[serde(flatten)]
    pub item: ResponseItem,
    pub label: String,
}

impl From<ResponseItem> for LabelledResponse {
    fn from(item: ResponseItem) -> Self {
        let label = item.label();
        Self { item, label }
    }
}

/// `serialize_with` for a list of items at the IPC/event boundary.
pub(crate) fn serialize_labelled<S: Serializer>(
    items: &[ResponseItem],
    s: S,
) -> Result<S::Ok, S::Error> {
    s.collect_seq(items.iter().cloned().map(LabelledResponse::from))
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
    /// replaces this one. An item with neither key never supersedes anything,
    /// and an FYI never replaces an open "needs you": the question is still
    /// waiting for the user, however much news arrives after it. An item
    /// about an agent is only replaced by another item about an agent — the
    /// same one, or (via the to-do) the agent that took the task over, e.g.
    /// after `respawn_agent` — never by an agent-less note on its to-do.
    fn superseded_by(&self, newer: &ResponseItem) -> bool {
        if self.kind == ResponseKind::NeedsYou && newer.kind == ResponseKind::Fyi {
            return false;
        }
        if self.process_id.is_some() && newer.process_id.is_none() {
            return false;
        }
        (newer.process_id.is_some() && self.process_id == newer.process_id)
            || (newer.todo_id.is_some() && self.todo_id == newer.todo_id)
    }

    /// Whether `newer` takes this item's place: the old supersede rules
    /// between items the assistant files, else the same key.
    fn replaced_by(&self, newer: &ResponseItem) -> bool {
        if is_legacy(&self.key) && is_legacy(&newer.key) {
            self.superseded_by(newer)
        } else {
            self.key == newer.key
        }
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

    /// Every item the UI shows (open and parked), newest first.
    pub fn list(&self) -> Vec<ResponseItem> {
        visible(&self.inner.lock().expect(LOCK_POISONED).items)
    }

    // Every mutation hands the new list to `changed` while still holding the
    // lock, so snapshots are published in the order the changes happened: an
    // add racing a dismiss can never publish its older list last and bring
    // the dismissed item back on screen. `changed` must not block (a
    // broadcast send does not).

    /// File `item` by its key. Same key and fingerprint: nothing changes
    /// (a dismissed item stays dismissed). Otherwise it goes on top as open,
    /// dropping whatever it replaces and, past [`MAX_ITEMS`], the oldest
    /// (dismissed first). Returns what happened and the item as listed.
    pub fn upsert(
        &self,
        mut item: ResponseItem,
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<(Upsert, ResponseItem)> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let existing = inner.items.iter().find(|o| o.key == item.key);
        if let Some(old) = existing.filter(|o| o.fingerprint == item.fingerprint) {
            return Ok((Upsert::Unchanged, old.clone()));
        }
        let mut outcome = if existing.is_some() {
            Upsert::Changed
        } else {
            Upsert::New
        };
        let mut items = inner.items.clone();
        items.retain(|old| !old.replaced_by(&item));
        // An assistant key can stay held by an item it does not replace (an
        // FYI beside an open question): keep keys unique.
        if items.iter().any(|o| o.key == item.key) {
            item.key = legacy_key(item.id, None, None);
            outcome = Upsert::New;
        }
        item.state = ResponseState::Open;
        items.insert(0, item.clone());
        while items.len() > MAX_ITEMS {
            let at = items
                .iter()
                .rposition(|i| i.state == ResponseState::Dismissed)
                .unwrap_or(items.len() - 1);
            items.remove(at);
        }
        commit(&mut inner, items, changed)?;
        Ok((outcome, item))
    }

    /// Park, unpark (`Open`) or dismiss the item with `key`.
    pub fn set_state(
        &self,
        key: &str,
        state: ResponseState,
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<()> {
        self.set_state_where(|i| i.key == key, state, changed)
    }

    /// Tick one item off by id: a soft [`set_state`](Self::set_state).
    pub fn dismiss(&self, id: ResponseId, changed: impl FnOnce(&[ResponseItem])) -> CoreResult<()> {
        self.set_state_where(|i| i.id == id, ResponseState::Dismissed, changed)
    }

    fn set_state_where(
        &self,
        pred: impl Fn(&ResponseItem) -> bool,
        state: ResponseState,
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        let index = inner
            .items
            .iter()
            .position(pred)
            .ok_or(CoreError::ResponseNotFound)?;
        if inner.items[index].state == state {
            return Ok(());
        }
        let mut items = inner.items.clone();
        items[index].state = state;
        commit(&mut inner, items, changed)
    }

    /// Drop the item with `key` for good (its source says it is handled).
    /// No such item is a no-op.
    pub fn remove(&self, key: &str, changed: impl FnOnce(&[ResponseItem])) -> CoreResult<()> {
        self.remove_where(|i| i.key == key, changed)
    }

    /// Drop every item whose key starts with `prefix` and is not in `live`.
    pub fn reconcile(
        &self,
        prefix: &str,
        live: &[String],
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<()> {
        self.remove_where(
            |i| i.key.starts_with(prefix) && !live.contains(&i.key),
            changed,
        )
    }

    /// Take one item off the list by id (a reply; see [`restore`](Self::restore)).
    pub fn take(
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
        let mut items = inner.items.clone();
        let item = items.remove(index);
        commit(&mut inner, items, changed)?;
        Ok(item)
    }

    /// Take every item matching `pred` off the list in one write (and one
    /// announcement). Nothing matching is a no-op that announces nothing.
    pub fn remove_where(
        &self,
        pred: impl Fn(&ResponseItem) -> bool,
        changed: impl FnOnce(&[ResponseItem]),
    ) -> CoreResult<()> {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        if !inner.items.iter().any(&pred) {
            return Ok(());
        }
        let items: Vec<ResponseItem> = inner.items.iter().filter(|i| !pred(i)).cloned().collect();
        commit(&mut inner, items, changed)
    }

    /// Put back an item [`take`](Self::take) took, in its place by age —
    /// unless a newer item about the same agent or to-do arrived meanwhile,
    /// which is the one the user should see.
    pub fn restore(&self, item: ResponseItem, changed: impl FnOnce(&[ResponseItem])) {
        let mut inner = self.inner.lock().expect(LOCK_POISONED);
        // Only a *newer* item can have replaced it: an older one that sits
        // beside it (a question next to the FYI being restored, an agent's
        // item next to a sweep note on its to-do) was never its replacement.
        if inner
            .items
            .iter()
            .any(|i| i.created_at > item.created_at && item.replaced_by(i))
        {
            return;
        }
        let mut item = item;
        // Its key may have been re-used meanwhile (an FYI filed beside it).
        if inner.items.iter().any(|i| i.key == item.key) {
            item.key = legacy_key(item.id, None, None);
        }
        let at = inner
            .items
            .iter()
            .position(|i| i.created_at < item.created_at)
            .unwrap_or(inner.items.len());
        // Kept in memory even if the write fails: this undoes a reply that
        // never got through, and losing the item is worse than a stale file.
        inner.items.insert(at, item);
        // At the cap, the oldest *other* item goes: the restored one is a
        // reply's question the user still has to answer.
        if inner.items.len() > MAX_ITEMS {
            let last = inner.items.len() - 1;
            inner.items.remove(if at == last { last - 1 } else { last });
        }
        let _ = save(inner.path.as_deref(), &inner.items);
        changed(&visible(&inner.items));
    }
}

fn load(path: &std::path::Path) -> Vec<ResponseItem> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut items: Vec<ResponseItem> = serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!("responses file is corrupt, starting fresh: {e}");
        Vec::new()
    });
    for i in items.iter_mut().filter(|i| i.key.is_empty()) {
        i.key = legacy_key(i.id, i.process_id, i.todo_id);
    }
    items
}

/// What the UI gets: everything but dismissed items.
fn visible(items: &[ResponseItem]) -> Vec<ResponseItem> {
    items
        .iter()
        .filter(|i| i.state != ResponseState::Dismissed)
        .cloned()
        .collect()
}

/// Save `items` and only then make them the list and announce them: a
/// failed write leaves memory matching the file, and nothing published.
fn commit(
    inner: &mut Inner,
    items: Vec<ResponseItem>,
    changed: impl FnOnce(&[ResponseItem]),
) -> CoreResult<()> {
    save(inner.path.as_deref(), &items)?;
    inner.items = items;
    changed(&visible(&inner.items));
    Ok(())
}

fn save(path: Option<&std::path::Path>, items: &[ResponseItem]) -> CoreResult<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(items)
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
        let id = ResponseId::new();
        ResponseItem {
            id,
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
            key: legacy_key(id, process_id, todo_id),
            fingerprint: text.to_string(),
            state: ResponseState::Open,
            reason: ResponseReason::Other,
            sender: None,
        }
    }

    /// A non-assistant item, e.g. an agent's question.
    fn keyed(key: &str, fingerprint: &str) -> ResponseItem {
        ResponseItem {
            key: key.to_string(),
            fingerprint: fingerprint.to_string(),
            ..item(None, None, fingerprint)
        }
    }

    /// Add, returning the list `add` announced.
    fn add(store: &ResponseStore, i: ResponseItem) -> Vec<ResponseItem> {
        let mut announced = Vec::new();
        store.upsert(i, |l| announced = l.to_vec()).unwrap();
        announced
    }

    fn texts(items: &[ResponseItem]) -> Vec<&str> {
        items.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn upsert_reports_new_changed_and_unchanged() {
        let store = ResponseStore::new();
        let upsert = |i| store.upsert(i, |_| {}).unwrap().0;
        assert_eq!(upsert(keyed("agent:a", "question:1")), Upsert::New);
        store
            .set_state("agent:a", ResponseState::Parked, |_| {})
            .unwrap();
        // Same fingerprint: nothing written, nothing announced, still parked.
        let (outcome, kept) = store
            .upsert(keyed("agent:a", "question:1"), |_| panic!("no change"))
            .unwrap();
        assert_eq!(outcome, Upsert::Unchanged);
        assert_eq!(kept.state, ResponseState::Parked);
        // New fingerprint: replaced, open again.
        assert_eq!(upsert(keyed("agent:a", "crashed:2")), Upsert::Changed);
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].fingerprint, "crashed:2");
        assert_eq!(list[0].state, ResponseState::Open);
    }

    #[test]
    fn a_dismissed_item_stays_hidden_until_its_fingerprint_changes() {
        let store = ResponseStore::new();
        let id = add(&store, keyed("agent:a", "question:1"))[0].id;
        let mut announced = vec![keyed("x", "x")];
        store.dismiss(id, |l| announced = l.to_vec()).unwrap();
        assert!(announced.is_empty());
        assert!(store.list().is_empty());
        let (outcome, _) = store
            .upsert(keyed("agent:a", "question:1"), |_| panic!("no change"))
            .unwrap();
        assert_eq!(outcome, Upsert::Unchanged);
        assert!(store.list().is_empty());
        add(&store, keyed("agent:a", "question:2"));
        assert_eq!(store.list()[0].state, ResponseState::Open);
    }

    #[test]
    fn a_dismissed_assistant_item_is_not_refiled_by_the_same_text() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        let id = add(&store, item(Some(agent), None, "tests green"))[0].id;
        store.dismiss(id, |_| {}).unwrap();
        add(&store, item(Some(agent), None, "tests green"));
        assert!(store.list().is_empty());
        assert_eq!(
            texts(&add(&store, item(Some(agent), None, "merged"))),
            vec!["merged"]
        );
    }

    #[test]
    fn set_state_parks_unparks_and_fails_on_an_unknown_key() {
        let store = ResponseStore::new();
        add(&store, keyed("agent:a", "1"));
        store
            .set_state("agent:a", ResponseState::Parked, |_| {})
            .unwrap();
        assert_eq!(store.list()[0].state, ResponseState::Parked);
        store
            .set_state("agent:a", ResponseState::Open, |_| {})
            .unwrap();
        assert_eq!(store.list()[0].state, ResponseState::Open);
        assert!(matches!(
            store.set_state("agent:b", ResponseState::Dismissed, |_| {}),
            Err(CoreError::ResponseNotFound)
        ));
    }

    #[test]
    fn remove_and_reconcile_drop_by_key() {
        let store = ResponseStore::new();
        add(&store, keyed("agent:a", "1"));
        add(&store, keyed("msg:slack:c1:t1", "1"));
        add(&store, keyed("msg:teams:c2:t2", "1"));
        add(&store, keyed("msg:slack:c3:t3", "1"));
        store.remove("agent:a", |_| {}).unwrap();
        store
            .reconcile("msg:", &["msg:teams:c2:t2".to_string()], |_| {})
            .unwrap();
        let keys: Vec<String> = store.list().into_iter().map(|i| i.key).collect();
        assert_eq!(keys, vec!["msg:teams:c2:t2"]);
    }

    #[test]
    fn an_fyi_beside_a_question_gets_its_own_key() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        add(
            &store,
            ResponseItem {
                kind: ResponseKind::NeedsYou,
                ..item(Some(agent), None, "merge now?")
            },
        );
        let list = add(&store, item(Some(agent), None, "tests green"));
        assert_eq!(list[1].key, format!("process:{agent}"));
        assert_eq!(list[0].key, format!("item:{}", list[0].id));
    }

    #[test]
    fn an_old_format_file_loads_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("assistant-responses.json");
        let agent = ProcessId::new();
        let (first, id) = (ResponseId::new(), ResponseId::new());
        std::fs::write(
            &file,
            format!(
                r#"[{{"id":"{first}","kind":"needsYou","text":"merge now?","project":null,
                "agent":null,"agentColor":null,"todo":null,"source":null,
                "processId":"{agent}","todoId":null,"createdAt":"2026-01-01T00:00:00Z"}},
                {{"id":"{id}","kind":"fyi","text":"x","project":null,"agent":null,
                "agentColor":null,"todo":null,"source":"Slack","processId":null,
                "todoId":null,"createdAt":"2026-01-01T00:00:00Z"}}]"#
            ),
        )
        .unwrap();
        let store = ResponseStore::new();
        store.set_path(file);
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].key, format!("process:{agent}"));
        assert_eq!(list[1].key, format!("item:{id}"));
        assert_eq!(list[0].fingerprint, "");
        assert_eq!(list[0].state, ResponseState::Open);
        assert_eq!(list[0].reason, ResponseReason::Other);
        assert_eq!(list[0].sender, None);
    }

    #[test]
    fn remove_where_drops_every_match_in_one_announcement() {
        let store = ResponseStore::new();
        add(&store, item(None, None, "keep"));
        add(&store, item(None, None, "drop one"));
        add(&store, item(None, None, "drop two"));
        let mut announcements = 0;
        let mut last = Vec::new();
        store
            .remove_where(
                |i| i.text.starts_with("drop"),
                |l| {
                    announcements += 1;
                    last = l.to_vec();
                },
            )
            .unwrap();
        assert_eq!(announcements, 1);
        assert_eq!(texts(&last), vec!["keep"]);
        assert_eq!(texts(&store.list()), vec!["keep"]);
    }

    #[test]
    fn remove_where_with_no_match_announces_nothing() {
        let store = ResponseStore::new();
        add(&store, item(None, None, "keep"));
        let mut announced = false;
        store.remove_where(|_| false, |_| announced = true).unwrap();
        assert!(!announced);
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
    fn an_fyi_never_replaces_an_open_needs_you() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        let question = ResponseItem {
            kind: ResponseKind::NeedsYou,
            ..item(Some(agent), None, "merge now?")
        };
        add(&store, question.clone());
        let list = add(&store, item(Some(agent), None, "tests green"));
        assert_eq!(texts(&list), vec!["tests green", "merge now?"]);
        // A newer question still replaces both.
        let list = add(
            &store,
            ResponseItem {
                kind: ResponseKind::NeedsYou,
                ..item(Some(agent), None, "rebase first?")
            },
        );
        assert_eq!(texts(&list), vec!["rebase first?"]);
    }

    #[test]
    fn a_note_on_the_to_do_without_an_agent_leaves_the_agents_item() {
        let store = ResponseStore::new();
        let todo = TodoId::new();
        let question = ResponseItem {
            kind: ResponseKind::NeedsYou,
            ..item(Some(ProcessId::new()), Some(todo), "merge now?")
        };
        add(&store, question);
        let list = add(
            &store,
            ResponseItem {
                kind: ResponseKind::NeedsYou,
                ..item(None, Some(todo), "open for a week: drop it?")
            },
        );
        assert_eq!(
            texts(&list),
            vec!["open for a week: drop it?", "merge now?"]
        );
    }

    #[test]
    fn a_restore_at_the_cap_keeps_the_restored_item() {
        let store = ResponseStore::new();
        let mut oldest = item(None, None, "oldest");
        oldest.created_at -= chrono::Duration::minutes(1);
        for n in 0..MAX_ITEMS {
            add(&store, item(None, None, &n.to_string()));
        }
        store.restore(oldest, |_| {});
        let list = store.list();
        assert_eq!(list.len(), MAX_ITEMS);
        assert_eq!(list[MAX_ITEMS - 1].text, "oldest");
        // The oldest other item made room.
        assert_eq!(list[MAX_ITEMS - 2].text, "1");
    }

    #[test]
    fn an_older_question_beside_it_does_not_stop_an_fyi_coming_back() {
        let store = ResponseStore::new();
        let agent = ProcessId::new();
        let mut question = ResponseItem {
            kind: ResponseKind::NeedsYou,
            ..item(Some(agent), None, "merge now?")
        };
        question.created_at -= chrono::Duration::minutes(1);
        add(&store, question);
        let fyi = add(&store, item(Some(agent), None, "tests green"))[0].clone();

        let taken = store.take(fyi.id, |_| {}).unwrap();
        store.restore(taken, |_| {});
        assert_eq!(texts(&store.list()), vec!["tests green", "merge now?"]);
    }

    #[test]
    fn an_older_agent_item_beside_it_does_not_stop_a_sweep_note_coming_back() {
        let store = ResponseStore::new();
        let todo = TodoId::new();
        let mut agents = item(Some(ProcessId::new()), Some(todo), "agent: done?");
        agents.created_at -= chrono::Duration::minutes(1);
        add(&store, agents);
        let note = add(&store, item(None, Some(todo), "open a week"))[0].clone();

        let taken = store.take(note.id, |_| {}).unwrap();
        store.restore(taken, |_| {});
        assert_eq!(texts(&store.list()), vec!["open a week", "agent: done?"]);
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
        let removed = store.take(gone, |l| announced = l.to_vec()).unwrap();
        assert_eq!(removed.text, "gone");
        assert_eq!(texts(&announced), vec!["kept"]);
        assert_eq!(store.list()[0].id, kept);
        assert!(matches!(
            store.take(gone, |_| panic!("nothing changed")),
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

        let taken = store.take(id, |_| {}).unwrap();
        store.restore(taken.clone(), |_| {});
        assert_eq!(texts(&store.list()), vec!["newer", "middle", "older"]);

        let taken = store.take(id, |_| {}).unwrap();
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
    fn a_failed_save_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("data");
        let store = ResponseStore::new();
        store.set_path(sub.join("assistant-responses.json"));
        let kept = add(&store, item(None, None, "kept"))[0].id;
        // A file where the data dir should be: every later write fails.
        std::fs::remove_dir_all(&sub).unwrap();
        std::fs::write(&sub, "").unwrap();

        assert!(store
            .upsert(item(None, None, "lost"), |_| panic!("nothing was saved"))
            .is_err());
        assert!(store.take(kept, |_| panic!("nothing was saved")).is_err());
        assert_eq!(texts(&store.list()), vec!["kept"]);
    }

    #[test]
    fn the_wire_shape_adds_a_label_that_the_stored_shape_lacks() {
        let full = ResponseItem {
            agent_color: Some("#7aa2f7".to_string()),
            todo: Some("Fix the login redirect".to_string()),
            source: Some("Slack".to_string()),
            ..item(Some(ProcessId::new()), Some(TodoId::new()), "x")
        };
        let stored = serde_json::to_value(&full).unwrap();
        assert!(stored.get("label").is_none(), "{stored}");
        let wire = serde_json::to_value(LabelledResponse::from(full.clone())).unwrap();
        assert_eq!(wire["label"], full.label());
        // The wire shape is the stored one plus `label`, field for field.
        let mut without_label = wire.clone();
        without_label.as_object_mut().unwrap().remove("label");
        assert_eq!(without_label, stored);
        assert_eq!(serde_json::from_value::<ResponseItem>(wire).unwrap(), full);

        let json = serde_json::to_value(item(None, None, "x")).unwrap();
        assert_eq!(json["kind"], "fyi");
        assert!(json.get("agentColor").is_some());
        assert!(json.get("createdAt").is_some());
        assert_eq!(
            serde_json::to_value(ResponseKind::NeedsYou).unwrap(),
            "needsYou"
        );
        assert_eq!(json["state"], "open");
        assert_eq!(json["reason"], "other");
        assert!(json.get("fingerprint").is_some());
        assert!(json.get("sender").is_some());
    }
}
