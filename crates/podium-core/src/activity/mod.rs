//! Reported activity: what a process itself says it is doing, as opposed to
//! the output-timing guess in [`crate::process::scrollback`]. One provider
//! per source ([`claude_status`], [`foreground`], [`hooks`]); this module
//! merges their answers into the [`OutputActivity`] the UI polls.

pub mod claude_status;
pub mod foreground;
pub mod hooks;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::ids::ProcessId;
use crate::process::scrollback::OutputActivity;

/// What a process reports it is doing. `Waiting` is "needs the user";
/// `Idle` is done or paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportedState {
    Working,
    Waiting,
    Idle,
}

/// The providers' merged answer for one process. The default is "nothing
/// reported", which leaves the UI on its own heuristics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reported {
    pub state: Option<ReportedState>,
    /// What a `Waiting` process is waiting for, when the provider says.
    pub waiting_for: Option<String>,
    /// When `state` began, when the provider knows.
    pub since: Option<SystemTime>,
    pub subagents: u32,
    pub shells: u32,
}

impl Reported {
    /// An agent: Claude Code's own status plus the hook counts.
    pub fn agent(
        status: Option<claude_status::ClaudeStatus>,
        counts: Option<hooks::Counts>,
    ) -> Self {
        let counts = counts.unwrap_or_default();
        let mut reported = Self {
            subagents: counts.subagents,
            shells: counts.shells,
            ..Self::default()
        };
        if let Some(status) = status {
            // A shell Claude reports is running, whether a hook counted it or not.
            reported.shells = reported.shells.max(u32::from(status.shell));
            reported.state = Some(status.state);
            reported.waiting_for = status.waiting_for;
            reported.since = status.since;
        }
        reported
    }

    /// A terminal: whether something runs in its foreground.
    pub fn terminal(busy: Option<bool>) -> Self {
        Self {
            state: busy.map(|busy| match busy {
                true => ReportedState::Working,
                false => ReportedState::Idle,
            }),
            ..Self::default()
        }
    }
}

/// When each process's reported state was first seen, for providers that
/// give no `since`.
pub(crate) type Seen = HashMap<ProcessId, (ReportedState, Instant)>;

/// One running process as polled: its output timing and what was reported.
pub(crate) struct Polled {
    pub id: ProcessId,
    /// `None` while it never printed.
    pub output: Option<OutputActivity>,
    /// When this run started (`now` if unknown).
    pub started: Instant,
    pub reported: Reported,
}

/// Fill each polled process's [`OutputActivity`] with what was reported.
/// `seen` is replaced by this poll's states, so a process that stopped drops
/// out and starts over on its next run.
pub(crate) fn merge(
    polled: Vec<Polled>,
    seen: &mut Seen,
    now: Instant,
    wall: SystemTime,
) -> HashMap<ProcessId, OutputActivity> {
    let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    let prev = std::mem::take(seen);
    let mut out = HashMap::new();
    for p in polled {
        let mut activity = match p.output {
            Some(output) => output,
            // Never printed: listed only once something is reported (a
            // terminal running a silent command), quiet since its start.
            None if p.reported != Reported::default() => OutputActivity {
                quiet_ms: ms(now.saturating_duration_since(p.started)),
                ..OutputActivity::default()
            },
            None => continue,
        };
        activity.subagents = p.reported.subagents;
        activity.shells = p.reported.shells;
        if let Some(state) = p.reported.state {
            // Still the state of the last poll: it began when first seen.
            let first_seen = prev
                .get(&p.id)
                .filter(|(was, _)| *was == state)
                .map_or(now, |(_, at)| *at);
            seen.insert(p.id, (state, first_seen));
            let spent = match p.reported.since {
                // A `since` ahead of our clock reads as "just now".
                Some(since) => wall.duration_since(since).unwrap_or_default(),
                None => now.saturating_duration_since(first_seen),
            };
            activity.state = Some(state);
            activity.waiting_for = p.reported.waiting_for;
            activity.state_ms = Some(ms(spent));
        }
        out.insert(p.id, activity);
    }
    out
}

/// Claude Code's config dir on the native backend: `$CLAUDE_CONFIG_DIR`, else
/// `~/.claude`.
// ponytail: a copy of the private `agent::claude_config_dir`, with the same
// limit (a CLAUDE_CONFIG_DIR set only in the login shell profile is missed).
// Drop this for that one once it is `pub(crate)`.
pub(crate) fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(dir.into());
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".claude"))
}

#[cfg(test)]
mod tests {
    use super::claude_status::ClaudeStatus;
    use super::hooks::Counts;
    use super::*;

    fn printed() -> Option<OutputActivity> {
        Some(OutputActivity {
            quiet_ms: 40,
            burst_ms: 900,
            ..OutputActivity::default()
        })
    }

    fn polled(id: ProcessId, reported: Reported) -> Vec<Polled> {
        vec![Polled {
            id,
            output: printed(),
            started: Instant::now(),
            reported,
        }]
    }

    #[test]
    fn an_agent_merges_claude_status_with_hook_counts() {
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let status = ClaudeStatus {
            state: ReportedState::Waiting,
            shell: false,
            waiting_for: Some("permission".to_string()),
            since: Some(since),
        };
        let counts = Counts {
            subagents: 2,
            shells: 3,
        };
        assert_eq!(
            Reported::agent(Some(status), Some(counts)),
            Reported {
                state: Some(ReportedState::Waiting),
                waiting_for: Some("permission".to_string()),
                since: Some(since),
                subagents: 2,
                shells: 3,
            }
        );
        // Counts alone report no state; nothing at all is the default.
        assert_eq!(
            Reported::agent(None, Some(counts)),
            Reported {
                subagents: 2,
                shells: 3,
                ..Reported::default()
            }
        );
        assert_eq!(Reported::agent(None, None), Reported::default());
    }

    #[test]
    fn a_shell_claude_reports_counts_as_at_least_one() {
        let shell_only = || ClaudeStatus {
            state: ReportedState::Idle,
            shell: true,
            waiting_for: None,
            since: None,
        };
        assert_eq!(Reported::agent(Some(shell_only()), None).shells, 1);
        let counted = Counts {
            subagents: 0,
            shells: 4,
        };
        assert_eq!(Reported::agent(Some(shell_only()), Some(counted)).shells, 4);
    }

    #[test]
    fn a_terminal_is_working_while_its_foreground_is_busy() {
        assert_eq!(
            Reported::terminal(Some(true)).state,
            Some(ReportedState::Working)
        );
        assert_eq!(
            Reported::terminal(Some(false)).state,
            Some(ReportedState::Idle)
        );
        assert_eq!(Reported::terminal(None), Reported::default());
    }

    #[test]
    fn nothing_reported_leaves_the_output_timing_as_it_was() {
        let id = ProcessId::new();
        let mut seen = Seen::new();
        let now = Instant::now();
        let out = merge(
            polled(id, Reported::default()),
            &mut seen,
            now,
            SystemTime::now(),
        );
        assert_eq!(out.get(&id), printed().as_ref());
        assert!(seen.is_empty());

        // A process that never printed and reports nothing stays unlisted.
        let silent = vec![Polled {
            id,
            output: None,
            started: now,
            reported: Reported::default(),
        }];
        assert!(merge(silent, &mut seen, now, SystemTime::now()).is_empty());
    }

    #[test]
    fn a_silent_process_is_listed_once_it_reports_a_state() {
        let id = ProcessId::new();
        let now = Instant::now();
        let silent = vec![Polled {
            id,
            output: None,
            started: now - Duration::from_secs(3),
            reported: Reported::terminal(Some(true)),
        }];
        let out = merge(silent, &mut Seen::new(), now, SystemTime::now());
        let activity = &out[&id];
        assert_eq!(activity.state, Some(ReportedState::Working));
        assert_eq!((activity.quiet_ms, activity.burst_ms), (3_000, 0));
    }

    #[test]
    fn state_ms_counts_from_when_the_state_was_first_seen() {
        let id = ProcessId::new();
        let mut seen = Seen::new();
        let t0 = Instant::now();
        let wall = SystemTime::now();
        let poll = |seen: &mut Seen, busy, at| {
            merge(polled(id, Reported::terminal(Some(busy))), seen, at, wall)[&id].state_ms
        };

        assert_eq!(poll(&mut seen, true, t0), Some(0));
        assert_eq!(
            poll(&mut seen, true, t0 + Duration::from_secs(2)),
            Some(2_000)
        );
        // A new state starts counting again.
        assert_eq!(poll(&mut seen, false, t0 + Duration::from_secs(3)), Some(0));
        assert_eq!(
            poll(&mut seen, false, t0 + Duration::from_secs(4)),
            Some(1_000)
        );

        // A poll without the process (it stopped) forgets it.
        merge(Vec::new(), &mut seen, t0, wall);
        assert_eq!(poll(&mut seen, false, t0 + Duration::from_secs(9)), Some(0));
    }

    #[test]
    fn a_provider_since_wins_over_the_remembered_change() {
        let id = ProcessId::new();
        let mut seen = Seen::new();
        let now = Instant::now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(5_000);
        let reported = |since| Reported {
            state: Some(ReportedState::Working),
            since: Some(since),
            ..Reported::default()
        };

        let past = reported(wall - Duration::from_secs(90));
        let out = merge(polled(id, past), &mut seen, now, wall);
        assert_eq!(out[&id].state_ms, Some(90_000));

        // Clock skew: a start in the future is no time spent, not a panic.
        let future = reported(wall + Duration::from_secs(5));
        let out = merge(polled(id, future), &mut seen, now, wall);
        assert_eq!(out[&id].state_ms, Some(0));
    }

    #[test]
    fn the_wire_is_camel_case_with_nulls_when_nothing_is_reported() {
        let id = ProcessId::new();
        let reported = Reported {
            waiting_for: Some("permission".to_string()),
            ..Reported::agent(
                Some(ClaudeStatus {
                    state: ReportedState::Waiting,
                    shell: true,
                    waiting_for: None,
                    since: None,
                }),
                None,
            )
        };
        let out = merge(
            polled(id, reported),
            &mut Seen::new(),
            Instant::now(),
            SystemTime::now(),
        );
        assert_eq!(
            serde_json::to_value(&out[&id]).unwrap(),
            serde_json::json!({
                "quietMs": 40,
                "burstMs": 900,
                "state": "waiting",
                "waitingFor": "permission",
                "stateMs": 0,
                "subagents": 0,
                "shells": 1,
            })
        );
        assert_eq!(
            serde_json::to_value(printed().unwrap()).unwrap(),
            serde_json::json!({
                "quietMs": 40,
                "burstMs": 900,
                "state": null,
                "waitingFor": null,
                "stateMs": null,
                "subagents": 0,
                "shells": 0,
            })
        );
    }
}
