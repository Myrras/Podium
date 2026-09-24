//! Finding an interactive agent's Claude Code session id after the fact.
//!
//! Podium's agents run as real interactive PTYs (the user's terminal view),
//! not the piped `stream-json` mode `assistant.rs` parses for its own
//! session id. Claude Code itself already writes every session — headless
//! or interactive — to `~/.claude/projects/<sanitized-cwd>/<session-id>.jsonl`
//! (verified empirically: the filename stem is the exact id `--resume`
//! takes), so this module reads that directory instead of the PTY stream.

use std::path::Path;
use std::time::SystemTime;

/// Claude Code's own scheme for naming a cwd's project directory: every path
/// separator becomes `-` (empirically verified against real
/// `~/.claude/projects/` entries).
fn sanitize_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy().replace(['/', '\\'], "-")
}

/// The newest `.jsonl` session file under `projects_root`'s directory for
/// `cwd` whose modification time is at or after `after`, if any. `after`
/// should be the agent's own spawn time, so a session file left over from an
/// unrelated earlier run in the same directory is never picked up.
fn find_latest_session_id_in(
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

/// Same as [`find_latest_session_id_in`], reading Claude Code's real
/// `~/.claude/projects` directory. `None` if the home directory can't be
/// resolved (mirrors every other `platform::` lookup's failure mode).
pub fn find_latest_session_id(cwd: &Path, after: SystemTime) -> Option<String> {
    let projects_root = crate::platform::claude_projects_root()?;
    find_latest_session_id_in(&projects_root, cwd, after)
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
