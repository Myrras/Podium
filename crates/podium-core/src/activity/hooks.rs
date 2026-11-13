//! What an agent's hooks report about the work running behind it.
//!
//! Podium adds four command hooks to a Claude Code agent's own `--settings`
//! ([`settings`]). Each hands its stdin JSON to Podium through the
//! filesystem: one file per event, written under a temp name and renamed
//! into the agent's own directory under `<mcp config dir>/hooks/` (0700,
//! files 0600), so the per-start wipe of that dir clears it too. No network,
//! no token. [`Store::counts`] drains those files on every activity poll.
//!
//! Native Unix only: the hook is a POSIX `sh` line. Agents without hooks
//! (Windows, WSL, a user's own `--settings`, a lean sub-agent) are never
//! registered, so their counts stay `None`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::ids::ProcessId;

/// How many sub-agents and background shells an agent has running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub subagents: u32,
    pub shells: u32,
}

/// Each hooked agent's event directory. Global because it is filled while
/// the launch is planned ([`settings`]), where no [`Store`] is in reach; ids
/// are unique, so orchestrators (and tests) never share an entry.
static WATCHED: LazyLock<Mutex<HashMap<ProcessId, PathBuf>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Never panic on the poll thread: the data stays usable after a panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn watched(id: ProcessId) -> Option<PathBuf> {
    lock(&WATCHED).get(&id).cloned()
}

/// The `"hooks"` settings for agent `id`, whose events land under
/// `config_dir/hooks/<id>/`; creates that dir and starts watching it.
/// `None` (no hooks) off Unix, or when the dir cannot be made.
pub(crate) fn settings(id: ProcessId, config_dir: &Path) -> Option<serde_json::Value> {
    if !cfg!(unix) {
        return None;
    }
    let dir = config_dir.join("hooks").join(id.to_string());
    if let Err(e) = private_dir(&dir) {
        tracing::warn!(error = %e, "agent hooks off: cannot create their directory");
        return None;
    }
    let command = hook_command(&dir)?;
    let shell_command = background_shell_command(&dir)?;
    lock(&WATCHED).insert(id, dir);
    let hook = |matcher: Option<&str>, command: &str| {
        let mut entry = serde_json::json!({
            "hooks": [{ "type": "command", "command": command }],
        });
        if let Some(m) = matcher {
            entry["matcher"] = m.into();
        }
        serde_json::json!([entry])
    };
    Some(serde_json::json!({
        "SubagentStart": hook(None, &command),
        "SubagentStop": hook(None, &command),
        "Stop": hook(None, &command),
        "PostToolUse": hook(Some("Bash"), &shell_command),
    }))
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// A POSIX `sh` line saving its stdin as one new `<secs>-<pid>.json` in
/// `dir`: written under a dot-name, then renamed, so a reader never sees half
/// a file and two hooks never interleave. Recreates `dir` if [`Store`]
/// swept it. Prints nothing (hook stdout can be read as a reply) and always
/// exits 0. `None` if the path cannot be quoted.
fn hook_command(dir: &Path) -> Option<String> {
    Some(format!("{}; exit 0", save(dir, "cat")?))
}

/// [`hook_command`] for `PostToolUse`, which fires for every Bash call with
/// its full output: saves only a payload that starts a background shell
/// (carries `"backgroundTaskId"`), so foreground output never reaches disk.
fn background_shell_command(dir: &Path) -> Option<String> {
    let save = save(dir, "printf '%s' \"$p\"")?;
    Some(format!(
        "p=$(cat); case $p in *'\"backgroundTaskId\"'*) {save};; esac; exit 0"
    ))
}

/// Shell commands writing `source`'s output to a new event file in `dir`.
fn save(dir: &Path, source: &str) -> Option<String> {
    let dir = crate::platform::quote_posix(dir.to_str()?).ok()?;
    Some(format!(
        "d={dir}; [ -d \"$d\" ] || mkdir -m 700 -p \"$d\"; t=\"$d/.$$.tmp\"; \
         (umask 077; {source} >\"$t\") && mv \"$t\" \"$d/$(date +%s)-$$.json\""
    ))
}

/// Polls an agent may miss in a row before [`Store`] takes it for gone and
/// removes its state and directory. Running agents are polled every round.
const STALE_ROUNDS: u64 = 60;

/// A pause between two [`Store::counts`] calls longer than this starts a new
/// poll round (the UI polls every agent at once, once per second).
const ROUND_GAP: Duration = Duration::from_millis(500);

/// The counts each agent's hooks last reported. The orchestrator holds one
/// ([`crate::Orchestrator::hooks`]).
#[derive(Debug, Default)]
pub struct Store {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    agents: HashMap<ProcessId, Agent>,
    round: u64,
    last_call: Option<Instant>,
}

/// What one agent has running, by task id.
#[derive(Debug, Default)]
struct Agent {
    subagents: HashSet<String>,
    shells: HashSet<String>,
    /// The poll round it was last asked about.
    round: u64,
}

impl Store {
    /// The counts for agent `id`, or `None` when it runs without Podium's
    /// hooks. Reads (and deletes) the events its hooks wrote since the last
    /// call, in order. Cheap enough for the once-a-second poll.
    ///
    /// Cleanup: an agent missing from [`STALE_ROUNDS`] poll rounds in a row
    /// (stopped or removed) loses its state and directory. Counted in rounds,
    /// not time, so a UI that paused polling sweeps nothing. A later restart
    /// starts from zero; its hook recreates the directory.
    pub fn counts(&self, id: ProcessId) -> Option<Counts> {
        let dir = watched(id)?;
        let mut inner = lock(&self.inner);
        let now = Instant::now();
        if inner
            .last_call
            .is_none_or(|at| now.duration_since(at) > ROUND_GAP)
        {
            inner.round += 1;
            let round = inner.round;
            inner.agents.retain(|other, agent| {
                if round - agent.round <= STALE_ROUNDS {
                    return true;
                }
                // Back after a stop: a new run, whose events are already
                // waiting in its directory.
                if *other == id {
                    *agent = Agent::default();
                    return true;
                }
                if let Some(dir) = watched(*other) {
                    let _ = std::fs::remove_dir_all(dir);
                }
                false
            });
        }
        inner.last_call = Some(now);
        let round = inner.round;
        let agent = inner.agents.entry(id).or_default();
        agent.round = round;
        for event in drain(&dir) {
            agent.apply(event);
        }
        let n = |set: &HashSet<String>| u32::try_from(set.len()).unwrap_or(u32::MAX);
        Some(Counts {
            subagents: n(&agent.subagents),
            shells: n(&agent.shells),
        })
    }
}

/// The events waiting in `dir`, oldest first, each file deleted once read.
/// Files that do not parse are dropped.
// shortcut: ordered by mtime, then name. Sound on APFS (ns timestamps); on a
// coarse-clock filesystem two hooks in one tick may swap. Upgrade with a
// sequence number if that shows.
fn drain(dir: &Path) -> Vec<Event> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && name.ends_with(".json")
        })
        .map(|e| {
            let at = e.metadata().and_then(|m| m.modified());
            (at.unwrap_or(SystemTime::UNIX_EPOCH), e.path())
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(_, path)| {
            let raw = std::fs::read(&path).ok();
            let _ = std::fs::remove_file(&path);
            // Payloads hold prompts and model output: debug only.
            serde_json::from_slice(&raw?)
                .inspect_err(
                    |e| tracing::debug!(path = %path.display(), error = %e, "skipped hook event"),
                )
                .ok()
        })
        .collect()
}

/// The fields of a hook payload the counts need.
#[derive(Debug, Deserialize)]
struct Event {
    hook_event_name: String,
    agent_id: Option<String>,
    background_tasks: Option<Vec<Task>>,
    tool_response: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Task {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    status: String,
}

impl Agent {
    fn apply(&mut self, event: Event) {
        match event.hook_event_name.as_str() {
            "SubagentStart" => {
                self.subagents.extend(event.agent_id);
            }
            // Its `background_tasks` still lists itself as running: ignored.
            "SubagentStop" => {
                if let Some(id) = event.agent_id {
                    self.subagents.remove(&id);
                }
            }
            // A foreground Bash has no `backgroundTaskId`.
            "PostToolUse" => {
                let shell = event
                    .tool_response
                    .as_ref()
                    .and_then(|r| r.get("backgroundTaskId"))
                    .and_then(serde_json::Value::as_str);
                self.shells.extend(shell.map(str::to_string));
            }
            // The full list: also catches shells that exited, which fire no
            // event of their own.
            "Stop" => {
                if let Some(tasks) = event.background_tasks {
                    let running = |kind: &str| {
                        tasks
                            .iter()
                            .filter(|t| t.kind == kind && t.status == "running")
                            .map(|t| t.id.clone())
                            .collect()
                    };
                    self.shells = running("shell");
                    self.subagents = running("subagent");
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: serde_json::Value) -> Event {
        serde_json::from_value(json).expect("event")
    }

    fn start(id: &str) -> Event {
        event(serde_json::json!({
            "hook_event_name": "SubagentStart",
            "agent_id": id,
            "agent_type": "general-purpose",
        }))
    }

    fn stop_sub(id: &str) -> Event {
        event(serde_json::json!({
            "hook_event_name": "SubagentStop",
            "agent_id": id,
            "background_tasks": [
                { "id": id, "type": "subagent", "status": "running" },
            ],
        }))
    }

    fn bash(background: Option<&str>) -> Event {
        let mut response = serde_json::json!({ "stdout": "", "stderr": "" });
        if let Some(id) = background {
            response["backgroundTaskId"] = id.into();
        }
        event(serde_json::json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "sleep 25", "run_in_background": background.is_some() },
            "tool_response": response,
        }))
    }

    fn stop(tasks: serde_json::Value) -> Event {
        event(serde_json::json!({
            "hook_event_name": "Stop",
            "background_tasks": tasks,
        }))
    }

    fn counts(agent: &Agent) -> (usize, usize) {
        (agent.subagents.len(), agent.shells.len())
    }

    #[test]
    fn each_event_moves_the_counts() {
        let mut a = Agent::default();
        a.apply(start("sub-1"));
        a.apply(start("sub-2"));
        assert_eq!(counts(&a), (2, 0));
        a.apply(stop_sub("sub-1"));
        assert_eq!(counts(&a), (1, 0), "its own running entry is ignored");
        a.apply(bash(None));
        assert_eq!(counts(&a), (1, 0), "a foreground Bash is no shell");
        a.apply(bash(Some("shell-1")));
        assert_eq!(counts(&a), (1, 1));
        // An unknown event, or a stop of an agent never seen, changes nothing.
        a.apply(event(
            serde_json::json!({ "hook_event_name": "Notification" }),
        ));
        a.apply(stop_sub("sub-9"));
        assert_eq!(counts(&a), (1, 1));
    }

    #[test]
    fn stop_replaces_both_sets_with_what_still_runs() {
        let mut a = Agent::default();
        a.apply(start("sub-1"));
        a.apply(start("sub-2"));
        a.apply(bash(Some("shell-1")));
        a.apply(bash(Some("shell-2")));
        a.apply(stop(serde_json::json!([
            { "id": "shell-2", "type": "shell", "status": "running", "command": "sleep 25" },
            { "id": "shell-1", "type": "shell", "status": "completed" },
            { "id": "sub-2", "type": "subagent", "status": "running", "agent_type": "general-purpose" },
            { "id": "mon-1", "type": "monitor", "status": "running" },
        ])));
        assert_eq!(a.shells, HashSet::from(["shell-2".to_string()]));
        assert_eq!(a.subagents, HashSet::from(["sub-2".to_string()]));
        // An empty list clears; a Stop without one leaves the counts alone.
        a.apply(event(serde_json::json!({ "hook_event_name": "Stop" })));
        assert_eq!(counts(&a), (1, 1));
        a.apply(stop(serde_json::json!([])));
        assert_eq!(counts(&a), (0, 0));
    }

    #[test]
    fn a_realistic_turn_ends_at_zero() {
        let mut a = Agent::default();
        // A turn starts a background shell and a sub-agent; the shell exits
        // unseen, the sub-agent's stop comes after the main Stop.
        a.apply(bash(Some("shell-1")));
        a.apply(start("sub-1"));
        assert_eq!(counts(&a), (1, 1));
        a.apply(stop(serde_json::json!([
            { "id": "sub-1", "type": "subagent", "status": "running" },
        ])));
        assert_eq!(counts(&a), (1, 0));
        a.apply(stop_sub("sub-1"));
        assert_eq!(counts(&a), (0, 0));
    }

    #[test]
    fn the_settings_hook_four_events_with_quoted_commands() {
        let root = tempfile::tempdir().expect("tempdir");
        let config = root.path().join("it's mine");
        let id = ProcessId::new();
        let hooks = settings(id, &config);
        if cfg!(not(unix)) {
            assert!(hooks.is_none());
            return;
        }
        let hooks = hooks.expect("hooks on unix");
        let command = hooks["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command")
            .to_string();
        let dir = config.join("hooks").join(id.to_string());
        let quoted = crate::platform::quote_posix(dir.to_str().unwrap()).unwrap();
        assert!(command.starts_with(&format!("d={quoted}; ")), "{command}");
        assert!(command.ends_with("exit 0"), "{command}");
        for name in ["SubagentStart", "SubagentStop", "Stop"] {
            assert_eq!(
                hooks[name],
                serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]),
                "{name}"
            );
        }
        // Bash gets its own command, saving background starts only.
        let shell = hooks["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command");
        assert_eq!(
            hooks["PostToolUse"],
            serde_json::json!([{
                "matcher": "Bash",
                "hooks": [{ "type": "command", "command": shell }],
            }])
        );
        assert!(
            shell.starts_with("p=$(cat); case $p in *'\"backgroundTaskId\"'*) "),
            "{shell}"
        );
        assert!(shell.contains(&format!("d={quoted}; ")), "{shell}");
        assert!(shell.ends_with(";; esac; exit 0"), "{shell}");
        assert_eq!(hooks.as_object().unwrap().len(), 4);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn an_agent_without_hooks_has_no_counts() {
        assert_eq!(Store::default().counts(ProcessId::new()), None);
    }

    #[test]
    fn counts_drain_files_in_order_and_skip_junk() {
        let root = tempfile::tempdir().expect("tempdir");
        let id = ProcessId::new();
        let dir = root.path().join(id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        lock(&WATCHED).insert(id, dir.clone());
        let store = Store::default();
        assert_eq!(store.counts(id), Some(Counts::default()));

        let base = SystemTime::now();
        let write = |name: &str, body: &str, secs: u64| {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base + Duration::from_secs(secs)).unwrap();
        };
        // Named against their order: the mtime decides.
        write(
            "1-9.json",
            r#"{"hook_event_name":"SubagentStop","agent_id":"sub-1"}"#,
            2,
        );
        write(
            "2-8.json",
            r#"{"hook_event_name":"SubagentStart","agent_id":"sub-1"}"#,
            1,
        );
        write("3-7.json", "not json", 3);
        write(
            ".4-6.tmp",
            r#"{"hook_event_name":"SubagentStart","agent_id":"sub-2"}"#,
            4,
        );
        assert_eq!(store.counts(id), Some(Counts::default()));
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            left,
            vec![std::ffi::OsString::from(".4-6.tmp")],
            "a half-written file waits"
        );
    }

    #[test]
    fn a_missing_agent_is_swept_after_its_stale_rounds() {
        let root = tempfile::tempdir().expect("tempdir");
        let (gone, live) = (ProcessId::new(), ProcessId::new());
        for id in [gone, live] {
            let dir = root.path().join(id.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            lock(&WATCHED).insert(id, dir);
        }
        let store = Store::default();
        let poll = |ids: &[ProcessId]| {
            // Each call after a long pause is its own round.
            lock(&store.inner).last_call = None;
            for id in ids {
                store.counts(*id);
            }
        };
        poll(&[gone, live]);
        for _ in 0..STALE_ROUNDS {
            poll(&[live]);
        }
        assert!(root.path().join(gone.to_string()).exists());
        poll(&[live]);
        assert!(!root.path().join(gone.to_string()).exists());
        assert!(!lock(&store.inner).agents.contains_key(&gone));
        assert!(root.path().join(live.to_string()).exists());
    }

    /// Runs hook `command` through `sh` with `payload` on stdin.
    #[cfg(unix)]
    fn run_hook(command: &str, payload: &serde_json::Value) {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .args(["-c", command])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "a hook must print nothing");
    }

    #[cfg(unix)]
    #[test]
    fn the_hook_commands_hand_their_stdin_to_counts() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let id = ProcessId::new();
        let hooks = settings(id, root.path()).expect("hooks");
        let command = |event: &str| {
            hooks[event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let dir = root.path().join("hooks").join(id.to_string());
        let files = || std::fs::read_dir(&dir).unwrap().flatten().count();
        let bash = |background: bool| {
            let mut response = serde_json::json!({ "stdout": "fictitious output\n" });
            if background {
                response["backgroundTaskId"] = "shell-1".into();
            }
            serde_json::json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": "sleep 25", "run_in_background": background },
                "tool_response": response,
            })
        };

        // A foreground Bash call's output never reaches the disk.
        run_hook(&command("PostToolUse"), &bash(false));
        assert_eq!(files(), 0);

        run_hook(&command("PostToolUse"), &bash(true));
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .expect("event file");
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        let store = Store::default();
        assert_eq!(
            store.counts(id),
            Some(Counts {
                subagents: 0,
                shells: 1
            })
        );
        assert_eq!(files(), 0, "drained");

        let start = serde_json::json!({
            "hook_event_name": "SubagentStart",
            "agent_id": "sub-1",
            "agent_type": "general-purpose",
        });
        run_hook(&command("SubagentStart"), &start);
        assert_eq!(
            store.counts(id),
            Some(Counts {
                subagents: 1,
                shells: 1
            })
        );
    }
}
