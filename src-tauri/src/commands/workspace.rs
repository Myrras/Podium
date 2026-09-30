//! Persistent workspace project list, stored as JSON in the app data dir.
//!
//! The workspace is the ordered set of projects shown in the sidebar; it
//! survives restarts (the frontend re-opens every path at startup). Writes
//! are atomic (temp file + rename). A missing or corrupt file is treated as
//! an empty list.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::IpcError;

/// File name inside the app data directory.
const WORKSPACE_FILE: &str = "workspace.json";

/// Serializes load-modify-save cycles so concurrent commands cannot clobber
/// each other's writes.
static WORKSPACE_LOCK: Mutex<()> = Mutex::new(());

fn workspace_path(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| IpcError::new("io", format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(WORKSPACE_FILE))
}

/// One persisted workspace project: its root path plus an optional user-set
/// display-name override. Position in the list is the sidebar order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// User-picked badge colour (`#rrggbb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

/// Accepts both the current object form and the legacy bare-path-string form,
/// so upgrading never drops a workspace (rewritten to the object form on the
/// next save).
#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Path(PathBuf),
    Full(WorkspaceEntry),
}

impl From<RawEntry> for WorkspaceEntry {
    fn from(raw: RawEntry) -> Self {
        match raw {
            RawEntry::Path(path) => WorkspaceEntry {
                path,
                name: None,
                color: None,
            },
            RawEntry::Full(entry) => entry,
        }
    }
}

/// Read the list; a missing or corrupt file is an empty list.
fn load(path: &Path) -> Vec<WorkspaceEntry> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<RawEntry>>(&raw)
        .map(|v| v.into_iter().map(WorkspaceEntry::from).collect())
        .unwrap_or_else(|e| {
            tracing::warn!("workspace.json is corrupt, starting fresh: {e}");
            Vec::new()
        })
}

/// Atomically persist the list: write a temp file, then rename it over the
/// old one so a crash mid-write can never leave a torn file.
fn save(path: &Path, entries: &[WorkspaceEntry]) -> Result<(), IpcError> {
    let dir = path
        .parent()
        .ok_or_else(|| IpcError::new("io", "workspace path has no parent directory"))?;
    std::fs::create_dir_all(dir).map_err(IpcError::from)?;
    let json = serde_json::to_string_pretty(entries)
        .map_err(|e| IpcError::new("io", format!("failed to encode workspace: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(IpcError::from)?;
    std::fs::rename(&tmp, path).map_err(IpcError::from)?;
    Ok(())
}

/// Append `path` unless already present. Order is stable: existing entries
/// keep their position (and any name override) so the sidebar does not
/// shuffle on reopen.
fn add_entry(entries: &mut Vec<WorkspaceEntry>, path: &Path) {
    if !entries.iter().any(|e| e.path == path) {
        entries.push(WorkspaceEntry {
            path: path.to_path_buf(),
            name: None,
            color: None,
        });
    }
}

/// Drop `path` if present (no-op when absent).
fn remove_entry(entries: &mut Vec<WorkspaceEntry>, path: &Path) {
    entries.retain(|e| e.path != path);
}

/// If an entry exists for `legacy` and it differs from `new`, either rename
/// it in place to `new` (preserving its sidebar position and name override),
/// or — when `new` is already present too (both spellings were opened at
/// some point) — drop the stale `legacy` one and keep the modern entry as-is.
/// See `podium_core::project::legacy_canonical_root`: on Windows, switching
/// project-root canonicalization from `std::fs::canonicalize` to
/// `dunce::canonicalize` left existing workspace entries under their old
/// `\\?\`-prefixed path, which this reconciles back onto the new one.
fn migrate_legacy_entry(entries: &mut Vec<WorkspaceEntry>, legacy: &Path, new: &Path) {
    if legacy == new {
        return;
    }
    let Some(pos) = entries.iter().position(|e| e.path == legacy) else {
        return;
    };
    if entries.iter().any(|e| e.path == new) {
        entries.remove(pos);
    } else {
        entries[pos].path = new.to_path_buf();
    }
}

/// Set (or clear) the display-name override for `path`; a blank name clears
/// it. Creates the entry if it is somehow absent.
fn set_entry_name(entries: &mut Vec<WorkspaceEntry>, path: &Path, name: Option<String>) {
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if let Some(entry) = entries.iter_mut().find(|e| e.path == path) {
        entry.name = name;
    } else {
        entries.push(WorkspaceEntry {
            path: path.to_path_buf(),
            name,
            color: None,
        });
    }
}

/// Set (or clear) the badge colour for `path`. Creates the entry if absent.
fn set_entry_color(entries: &mut Vec<WorkspaceEntry>, path: &Path, color: Option<String>) {
    if let Some(entry) = entries.iter_mut().find(|e| e.path == path) {
        entry.color = color;
    } else {
        entries.push(WorkspaceEntry {
            path: path.to_path_buf(),
            name: None,
            color,
        });
    }
}

/// Reorder entries to match `ordered`; any entry not listed keeps its
/// relative position at the end.
fn reorder_entries(entries: &mut Vec<WorkspaceEntry>, ordered: &[PathBuf]) {
    let mut out: Vec<WorkspaceEntry> = Vec::with_capacity(entries.len());
    for path in ordered {
        if let Some(pos) = entries.iter().position(|e| &e.path == path) {
            out.push(entries.remove(pos));
        }
    }
    out.append(entries);
    *entries = out;
}

/// Add `path` to the workspace list (deduped, order-stable).
pub fn add(app: &AppHandle, path: &Path) -> Result<(), IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    add_entry(&mut entries, path);
    save(&file, &entries)
}

/// Remove `path` from the workspace list (no error if absent).
pub fn remove(app: &AppHandle, path: &Path) -> Result<(), IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    remove_entry(&mut entries, path);
    save(&file, &entries)
}

/// Reconcile a project's old, pre-`dunce` workspace entry (if any) onto its
/// current canonical path. Call before `add` so a fresh `project_open` for an
/// already-known folder doesn't leave a stale duplicate sidebar entry behind.
pub fn migrate_legacy_path(app: &AppHandle, legacy: &Path, new: &Path) -> Result<(), IpcError> {
    if legacy == new {
        return Ok(());
    }
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    migrate_legacy_entry(&mut entries, legacy, new);
    save(&file, &entries)
}

/// Persist a project's display-name override (a blank name clears it).
pub fn set_name(app: &AppHandle, path: &Path, name: Option<String>) -> Result<(), IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    set_entry_name(&mut entries, path, name);
    save(&file, &entries)
}

/// The persisted display-name override for `path`, if any.
pub fn name_for(app: &AppHandle, path: &Path) -> Result<Option<String>, IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    Ok(load(&file)
        .into_iter()
        .find(|e| e.path == path)
        .and_then(|e| e.name))
}

/// Persist a project's badge colour (`None` clears it).
pub fn set_color(app: &AppHandle, path: &Path, color: Option<String>) -> Result<(), IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    set_entry_color(&mut entries, path, color);
    save(&file, &entries)
}

/// The persisted badge colour for `path`, if any.
pub fn color_for(app: &AppHandle, path: &Path) -> Result<Option<String>, IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    Ok(load(&file)
        .into_iter()
        .find(|e| e.path == path)
        .and_then(|e| e.color))
}

/// Reorder the persisted workspace to match `ordered` (the new sidebar order).
pub fn reorder(app: &AppHandle, ordered: &[PathBuf]) -> Result<(), IpcError> {
    let file = workspace_path(app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    reorder_entries(&mut entries, ordered);
    save(&file, &entries)
}

/// Return the persisted workspace project paths, in sidebar order.
#[tauri::command]
pub fn workspace_list(app: AppHandle) -> Result<Vec<String>, IpcError> {
    let file = workspace_path(&app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    Ok(load(&file)
        .into_iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect())
}

/// Remove one entry by path; returns the updated list of paths. The frontend
/// uses this to prune entries whose folder no longer exists at startup.
#[tauri::command]
pub fn workspace_remove(app: AppHandle, path: String) -> Result<Vec<String>, IpcError> {
    let file = workspace_path(&app)?;
    let _guard = WORKSPACE_LOCK.lock().expect("workspace lock poisoned");
    let mut entries = load(&file);
    remove_entry(&mut entries, Path::new(&path));
    save(&file, &entries)?;
    Ok(entries
        .into_iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(entries: &[WorkspaceEntry]) -> Vec<PathBuf> {
        entries.iter().map(|e| e.path.clone()).collect()
    }

    #[test]
    fn add_appends_and_dedupes() {
        let mut entries = Vec::new();
        add_entry(&mut entries, Path::new("/a"));
        add_entry(&mut entries, Path::new("/b"));
        add_entry(&mut entries, Path::new("/a"));
        assert_eq!(
            paths(&entries),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn add_keeps_order_and_name_stable_on_reopen() {
        let mut entries = vec![
            WorkspaceEntry {
                path: PathBuf::from("/a"),
                name: Some("Alpha".into()),
                color: None,
            },
            WorkspaceEntry {
                path: PathBuf::from("/b"),
                name: None,
                color: None,
            },
        ];
        add_entry(&mut entries, Path::new("/a"));
        assert_eq!(
            paths(&entries),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(entries[0].name.as_deref(), Some("Alpha"));
    }

    #[test]
    fn migrate_legacy_entry_renames_in_place_when_new_is_absent() {
        let mut entries = vec![WorkspaceEntry {
            path: PathBuf::from(r"\\?\C:\Meridian"),
            name: Some("Meridian".into()),
            color: None,
        }];
        migrate_legacy_entry(
            &mut entries,
            Path::new(r"\\?\C:\Meridian"),
            Path::new(r"C:\Meridian"),
        );
        assert_eq!(paths(&entries), vec![PathBuf::from(r"C:\Meridian")]);
        assert_eq!(
            entries[0].name.as_deref(),
            Some("Meridian"),
            "position and name override survive the rename"
        );
    }

    #[test]
    fn migrate_legacy_entry_drops_the_legacy_duplicate_when_new_already_exists() {
        let mut entries = vec![
            WorkspaceEntry {
                path: PathBuf::from(r"\\?\C:\Meridian"),
                name: None,
                color: None,
            },
            WorkspaceEntry {
                path: PathBuf::from(r"C:\Meridian"),
                name: Some("Meridian".into()),
                color: None,
            },
        ];
        migrate_legacy_entry(
            &mut entries,
            Path::new(r"\\?\C:\Meridian"),
            Path::new(r"C:\Meridian"),
        );
        assert_eq!(paths(&entries), vec![PathBuf::from(r"C:\Meridian")]);
        assert_eq!(entries[0].name.as_deref(), Some("Meridian"));
    }

    #[test]
    fn migrate_legacy_entry_is_a_no_op_when_legacy_is_absent() {
        let mut entries = vec![WorkspaceEntry {
            path: PathBuf::from(r"C:\Other"),
            name: None,
            color: None,
        }];
        migrate_legacy_entry(
            &mut entries,
            Path::new(r"\\?\C:\Meridian"),
            Path::new(r"C:\Meridian"),
        );
        assert_eq!(paths(&entries), vec![PathBuf::from(r"C:\Other")]);
    }

    #[test]
    fn remove_drops_entry_and_ignores_absent() {
        let mut entries = vec![
            WorkspaceEntry {
                path: PathBuf::from("/a"),
                name: None,
                color: None,
            },
            WorkspaceEntry {
                path: PathBuf::from("/b"),
                name: None,
                color: None,
            },
        ];
        remove_entry(&mut entries, Path::new("/a"));
        assert_eq!(paths(&entries), vec![PathBuf::from("/b")]);
        remove_entry(&mut entries, Path::new("/missing"));
        assert_eq!(paths(&entries), vec![PathBuf::from("/b")]);
    }

    #[test]
    fn set_name_sets_trims_and_clears() {
        let mut entries = vec![WorkspaceEntry {
            path: PathBuf::from("/a"),
            name: None,
            color: None,
        }];
        set_entry_name(&mut entries, Path::new("/a"), Some("  Cool  ".into()));
        assert_eq!(entries[0].name.as_deref(), Some("Cool"), "trimmed");
        set_entry_name(&mut entries, Path::new("/a"), Some("   ".into()));
        assert_eq!(entries[0].name, None, "blank clears");
        // Setting a name for an unknown path creates the entry.
        set_entry_name(&mut entries, Path::new("/new"), Some("New".into()));
        assert_eq!(entries[1].path, PathBuf::from("/new"));
        assert_eq!(entries[1].name.as_deref(), Some("New"));
    }

    #[test]
    fn set_color_keeps_name_and_round_trips_through_json() {
        let mut entries = vec![WorkspaceEntry {
            path: PathBuf::from("/a"),
            name: Some("Alpha".into()),
            color: None,
        }];
        set_entry_color(&mut entries, Path::new("/a"), Some("#3e63dd".into()));
        assert_eq!(entries[0].color.as_deref(), Some("#3e63dd"));
        assert_eq!(entries[0].name.as_deref(), Some("Alpha"));

        let json = serde_json::to_string(&entries).unwrap();
        let back: Vec<WorkspaceEntry> = serde_json::from_str::<Vec<RawEntry>>(&json)
            .unwrap()
            .into_iter()
            .map(WorkspaceEntry::from)
            .collect();
        assert_eq!(back, entries);

        set_entry_color(&mut entries, Path::new("/a"), None);
        assert_eq!(entries[0].color, None);
        assert!(!serde_json::to_string(&entries).unwrap().contains("color"));
    }

    #[test]
    fn reorder_matches_order_and_appends_missing() {
        let mut entries = vec![
            WorkspaceEntry {
                path: PathBuf::from("/a"),
                name: None,
                color: None,
            },
            WorkspaceEntry {
                path: PathBuf::from("/b"),
                name: Some("Bee".into()),
                color: None,
            },
            WorkspaceEntry {
                path: PathBuf::from("/c"),
                name: None,
                color: None,
            },
        ];
        // Mention c, a plus an unknown; b is appended at the end.
        reorder_entries(
            &mut entries,
            &[
                PathBuf::from("/c"),
                PathBuf::from("/a"),
                PathBuf::from("/unknown"),
            ],
        );
        assert_eq!(
            paths(&entries),
            vec![
                PathBuf::from("/c"),
                PathBuf::from("/a"),
                PathBuf::from("/b"),
            ]
        );
        // Name overrides survive reordering.
        assert_eq!(entries[2].name.as_deref(), Some("Bee"));
    }

    #[test]
    fn load_migrates_legacy_bare_path_form() {
        // Old files were a bare JSON array of path strings; parse into entries.
        let legacy = r#"["/a", "/b"]"#;
        let raw: Vec<RawEntry> = serde_json::from_str(legacy).unwrap();
        let entries: Vec<WorkspaceEntry> = raw.into_iter().map(WorkspaceEntry::from).collect();
        assert_eq!(
            paths(&entries),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert!(entries.iter().all(|e| e.name.is_none()));
    }

    #[test]
    fn load_parses_new_object_form() {
        let modern = r#"[{"path": "/a", "name": "Alpha"}, {"path": "/b"}]"#;
        let raw: Vec<RawEntry> = serde_json::from_str(modern).unwrap();
        let entries: Vec<WorkspaceEntry> = raw.into_iter().map(WorkspaceEntry::from).collect();
        assert_eq!(entries[0].name.as_deref(), Some("Alpha"));
        assert_eq!(entries[1].name, None);
    }
}
