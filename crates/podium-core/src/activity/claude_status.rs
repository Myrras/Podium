//! Claude Code's own record of what a session is doing.
//!
//! While a session runs, Claude Code keeps `<config_dir>/sessions/<cli
//! pid>.json` with its `status`. The CLI's pid is not always the PTY child's
//! (a login shell that did not exec stays the parent), so the file is found
//! by the `-n` name Podium gives the session, with the PTY pid as fallback.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::ReportedState;

/// What identifies one agent's Claude Code session on disk.
#[derive(Debug, Clone, Copy)]
pub struct Probe<'a> {
    /// Claude Code's config dir (`$CLAUDE_CONFIG_DIR`, else `~/.claude`).
    pub config_dir: &'a Path,
    /// The `podium-<tag>` name Podium stamps on the session
    /// ([`crate::agent::peer_session_name`]).
    pub session_name: &'a str,
    /// The agent's PTY child, while it has one.
    pub pty_pid: Option<u32>,
    /// The directory the agent runs in.
    pub cwd: &'a Path,
    /// When Podium started this run (wall clock).
    pub started: SystemTime,
}

/// What Claude Code says the session is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeStatus {
    pub state: ReportedState,
    /// Only a background shell is running.
    pub shell: bool,
    /// What a waiting session is waiting for.
    pub waiting_for: Option<String>,
    /// When `state` began, when Claude Code records it.
    pub since: Option<SystemTime>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Live {
    pid: Option<u32>,
    started_at: u64,
    cwd: Option<PathBuf>,
    name: Option<String>,
    status: String,
    status_updated_at: Option<u64>,
    waiting_for: Option<String>,
}

/// The status of the session `probe` describes, or `None` when Claude Code
/// has recorded none. Reads the disk: call it outside any lock.
pub fn read(probe: &Probe) -> Option<ClaudeStatus> {
    let dir = probe.config_dir.join("sessions");
    let earliest = probe
        .started
        .checked_sub(crate::agent::LIVE_SESSION_SLACK)
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis());
    let trusted = |live: &Live| {
        u128::from(live.started_at) >= earliest
            && live.cwd.as_deref().is_none_or(|cwd| cwd == probe.cwd)
    };
    let parse = |path: &Path| -> Option<Live> {
        let live: Live = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        trusted(&live).then_some(live)
    };

    // The tag is fixed-length and random, so a prefix match only adds the
    // suffix Claude appends on a name clash.
    let by_name = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| parse(&path))
        .filter(|live| {
            live.name
                .as_deref()
                .is_some_and(|name| name.starts_with(probe.session_name))
        })
        .max_by_key(|live| live.started_at);
    let live = by_name.or_else(|| {
        let pid = probe.pty_pid?;
        parse(&dir.join(format!("{pid}.json"))).filter(|live| live.pid == Some(pid))
    })?;

    let (state, shell) = match live.status.as_str() {
        "busy" => (ReportedState::Working, false),
        "shell" => (ReportedState::Working, true),
        "waiting" => (ReportedState::Waiting, false),
        "idle" => (ReportedState::Idle, false),
        _ => return None,
    };
    Some(ClaudeStatus {
        state,
        shell,
        waiting_for: live.waiting_for.filter(|_| state == ReportedState::Waiting),
        since: live
            .status_updated_at
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NAME: &str = "podium-ab12cd34";
    const PID: u32 = 4242;

    /// Podium started the run at 1_000_000 ms past the epoch.
    fn started() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_000)
    }

    fn write(dir: &Path, file: &str, body: &serde_json::Value) {
        let sessions = dir.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(sessions.join(file), body.to_string()).unwrap();
    }

    fn session(pid: u32, name: &str, status: &str) -> serde_json::Value {
        json!({
            "pid": pid,
            "sessionId": "00000000-0000-0000-0000-000000000001",
            "cwd": "/fictitious/project",
            "startedAt": 1_000_500u64,
            "name": name,
            "status": status,
            "statusUpdatedAt": 1_002_000u64,
        })
    }

    fn read_in(dir: &Path, pty_pid: Option<u32>) -> Option<ClaudeStatus> {
        read(&Probe {
            config_dir: dir,
            session_name: NAME,
            pty_pid,
            cwd: Path::new("/fictitious/project"),
            started: started(),
        })
    }

    #[test]
    fn each_status_maps_to_a_state() {
        let dir = tempfile::tempdir().unwrap();
        let since = Some(UNIX_EPOCH + Duration::from_millis(1_002_000));
        let cases = [
            ("busy", ReportedState::Working, false),
            ("shell", ReportedState::Working, true),
            ("idle", ReportedState::Idle, false),
        ];
        for (status, state, shell) in cases {
            write(dir.path(), "1.json", &session(1, NAME, status));
            assert_eq!(
                read_in(dir.path(), None),
                Some(ClaudeStatus {
                    state,
                    shell,
                    waiting_for: None,
                    since,
                }),
                "{status}"
            );
        }

        let mut waiting = session(1, NAME, "waiting");
        waiting["waitingFor"] = json!("input needed");
        write(dir.path(), "1.json", &waiting);
        assert_eq!(
            read_in(dir.path(), None),
            Some(ClaudeStatus {
                state: ReportedState::Waiting,
                shell: false,
                waiting_for: Some("input needed".to_string()),
                since,
            })
        );

        write(dir.path(), "1.json", &session(1, NAME, "compacting"));
        assert_eq!(read_in(dir.path(), None), None);
    }

    #[test]
    fn the_name_matches_exactly_or_with_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1.json", &session(1, "podium-zz99zz99", "busy"));
        assert_eq!(read_in(dir.path(), None), None);

        write(
            dir.path(),
            "2.json",
            &session(2, &format!("{NAME}-2"), "idle"),
        );
        assert_eq!(
            read_in(dir.path(), None).unwrap().state,
            ReportedState::Idle
        );

        write(dir.path(), "3.json", &session(3, NAME, "busy"));
        let mut later = session(4, NAME, "waiting");
        later["startedAt"] = json!(1_000_900u64);
        write(dir.path(), "4.json", &later);
        // Two matches: the latest start wins.
        assert_eq!(
            read_in(dir.path(), None).unwrap().state,
            ReportedState::Waiting
        );
    }

    #[test]
    fn a_stale_or_foreign_record_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let mut stale = session(1, NAME, "busy");
        stale["startedAt"] = json!(994_000u64);
        write(dir.path(), "1.json", &stale);
        assert_eq!(read_in(dir.path(), None), None);

        let mut elsewhere = session(1, NAME, "busy");
        elsewhere["cwd"] = json!("/fictitious/other");
        write(dir.path(), "1.json", &elsewhere);
        assert_eq!(read_in(dir.path(), None), None);

        // Within the slack, and no cwd at all, is trusted.
        let mut early = session(1, NAME, "busy");
        early["startedAt"] = json!(996_000u64);
        early.as_object_mut().unwrap().remove("cwd");
        write(dir.path(), "1.json", &early);
        assert!(read_in(dir.path(), None).is_some());
    }

    #[test]
    fn a_malformed_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "1.json", &session(1, NAME, "busy"));
        fs::write(dir.path().join("sessions/2.json"), "{not json").unwrap();
        fs::write(dir.path().join("sessions/3.key"), "fictitious").unwrap();
        assert_eq!(
            read_in(dir.path(), None).unwrap().state,
            ReportedState::Working
        );
    }

    #[test]
    fn without_a_name_match_the_pty_pid_file_is_used() {
        let dir = tempfile::tempdir().unwrap();
        // The user passed their own `-n`.
        let own = session(PID, "my-own-name", "idle");
        write(dir.path(), &format!("{PID}.json"), &own);
        assert_eq!(read_in(dir.path(), None), None);
        assert_eq!(
            read_in(dir.path(), Some(PID)).unwrap().state,
            ReportedState::Idle
        );

        // The file must say it is that pid.
        write(dir.path(), &format!("{PID}.json"), &session(7, "x", "idle"));
        assert_eq!(read_in(dir.path(), Some(PID)), None);
    }

    #[test]
    fn a_missing_directory_reports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_in(dir.path(), Some(PID)), None);
    }
}
