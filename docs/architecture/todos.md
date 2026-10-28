# To-dos

Each project has a shared to-do list (`todo.rs`), visible to the user (sidebar)
and every agent (MCP).

## Storage

Persisted in one `todos.json` in the app data dir, **keyed by project root
path**, not project id, so lists survive restarts. Atomic writes (temp file +
rename) keep the project folder clean and the file uncorrupted. Every mutation
emits `TodosChanged`.

## Links and archive

To-dos can carry pinned issue/PR **links** (agents add them via
`add_todo_link`, shown at the top of the detail pane).

They can also be **archived**. Listing auto-archives any done to-do left over
from an earlier day, and a to-do can be archived/unarchived manually. Archived
items drop out of the active list and show in the Archive modal.

## Assignment and colours

An **assigned** to-do carries its owning agent's session colour. Each agent
gets a subtle colour at spawn from an 8-hue palette, avoiding colours other
live agents hold. It rides on `AssignedAgent`, and is also surfaced on
`ProcessInfo.color`. The to-do row **and the agent's own sidebar row** are
tinted the same colour so the two can be matched at a glance. The to-do's
spawn button is hidden and it drops out of multi-select, because one session
owns it.

An agent that **exits** keeps its to-do/scratchpad links (only removing the
process drops them) but loses its colour. `spec.color` is cleared in the exit
handler and kept as `last_color`, and the to-do row is tinted neutral grey. The
next start (`do_start`) takes `last_color` back if no live agent holds it, else
picks one the way a spawn does (`pick_session_color`).

A link to an exited agent no longer counts as **owning**.
`AssignedAgent.active` (`ManagedProcess::is_active`, also in the `list_todos`
MCP output) is `false`, the row gets its spawn button back and can join a
multi-select again, and spawning on it (or `assign_todo`) hands it to the new
agent.

## Exited agents are removed automatically

The setting is `remove_exited_agents_after_minutes` in `agents.json`
(`agent/cleanup.rs`). On disk it is `removeExitedAgentsAfterMinutes`, on
`AgentSettingsDto` `removeExitedAfterMinutes`. Default 5, missing = 5, `0` =
never, at most 1440, refused above that in core.

A sweep task (`cleanup::spawn`, started in `setup` next to the proactivity
watcher, weak orchestrator ref, every 15 s) reads the setting fresh each tick
and calls `Orchestrator::remove_exited_agents`. That picks, under one `inner`
lock, every `Agent`-kind process whose status is `Exited` with `at` (wall
clock, so sleep counts; a clock that went back reads as not due) at least that
long ago and no supervised restart pending. It never picks one of the
assistant's sub-agents (`inner.subagents`), which the sub-agent hub expires
itself after its linger, keeping the result for the assistant.

Each pick is taken out with `take_process`, the same path `remove_process`
uses (links cleared, restart cancelled), then `ProcessRemoved` plus
`TodosChanged`/`ScratchpadsChanged` are published per affected project.
Terminals and services are never touched. A restart puts the agent back to
running, and its next exit writes a new `at`, so the clock runs from the latest
exit.

The exit nudge is published with the exit, long before the removal. A Responses
item keeps the names it resolved at add time, so a removed process id there
breaks nothing (a later `get_process_output` or `respawn_agent` on it answers
`ProcessNotFound`). The frontend's `applyRemoved` already disposes the terminal
and clears the focus, so an open agent's work area falls back to the empty
state. A removed agent can no longer be restarted from the sidebar, so its
conversation is not resumable from Podium after that.
`AgentCleanupConfig { tick, minute }` is injectable (tests use millisecond
minutes).

## Restart resumes the session

Claude Code agents launch with `--session-id <process id>`. The adapter's
`LaunchPlan.restart` (`RestartCommands`, the launch line in parts) spells the
two commands a manual start of an exited agent can run instead, for whichever
session id is current (`AgentSession` on the process; `spec.command` is never
swapped).

`restart_launch` picks per start:

- `resume(<current id>)` (no initial prompt) when `AgentAdapter::session_exists`
  finds the CLI's transcript.
- Else `fresh(<new UUID>)`: the original launch under a *new* `--session-id`,
  since the CLI refuses an id it already used and `--resume` fails without a
  transcript. The new id becomes current, so the agent stays resumable.

The chosen launch (`Relaunch`) is handed to `do_start`, which makes its id
current under the same lock only once the PTY spawned. A concurrent start that
loses with `ProcessAlreadyRunning` therefore cannot overwrite it. A resumed run
that crashes within `RESUME_FAILURE_WINDOW` (5 s) marks the id `resume_failed`
in the exit handler, so the next start is fresh instead of crash-looping on
`--resume`.

The transcript is `projects/*/<id>.jsonl` under `$CLAUDE_CONFIG_DIR` or
`~/.claude`. It is read from disk natively, and on a WSL project checked inside
the distro through its login shell (`run_shell_ok_on`, off the async runtime),
since that is where the CLI runs.

## `/clear` is followed

After `/clear` the CLI goes on under a new session id it picks itself, so
`AgentSession.id` would still name the pre-clear conversation.
`AgentAdapter::live_session` reads Claude Code's live registry,
`<config>/sessions/<pid>.json`, for the PTY child (`LiveProcess`: pid, spawn
time, cwd). It trusts the record only when:

- its own `pid` matches,
- its `startedAt` is no earlier than the spawn (less 5 s of slack; the spawn
  time is `ManagedProcess.started_wall`, a wall-clock stamp, since an `Instant`
  does not count a laptop's sleep), and
- its `cwd`, when present, is the agent's.

So a stale record from a reused pid is ignored.

The PTY child is the CLI only when the login shell execs it: a lone command,
which zsh and bash do (verified against a running Podium). That is not the case
behind a wrapper `command_override`, a shell function or fish, nor on native
Windows (`cmd.exe` is the child) or WSL (Podium holds `wsl.exe`'s pid). There
`live_session` finds nothing (or answers `None` outright) and the agent keeps
the pre-clear id.

`read_live_session` takes the new id only when it is a valid UUID other than
the current one, and only once Claude Code has stored the new conversation
(`session_exists`; one not yet written to would only make the next start
fresh). There is no newest-file guess.

### Locking and timing

The read happens outside the lock. `live_probe` collects pid, spawn time, cwd,
adapter, backend and the current id under it, and `apply_live_session` moves
the session under it again only if the same run (pid) is still on the same id,
clearing `resumed`/`resume_failed`.

It is asked while the CLI is still there:

- in `stop_process` before the signal (a clean exit removes the record),
- in the exit handler on the wait thread, after the resume-failure check (a
  crash or kill leaves the record behind), and
- in the shutdown capture.

Stop and capture read off the runtime, all agents at once, giving up after
`LIVE_SESSION_TIMEOUT` (200 ms): a read over 200 ms is skipped (logged at
debug) and the id kept. The exit handler has no such deadline, so a hung file
system holds back only that agent's `Exited`. An agent that exits cleanly on
its own (`/exit`) therefore resumes its pre-clear conversation.

### Opting out

A session flag the user put in `extra_args` (`--session-id`, `--resume`,
`--continue`, `--fork-session`, `--from-pr`, `=`-forms included) disables all
of it. The same `AgentSession` is what _Agent session memory_ (see
[Processes and agents](processes-and-agents.md#agent-session-memory)) carries across an app
restart.

## Open versus multi-select

Sidebar rows distinguish being **open** in the work area (a subtle accent
highlight, like agents/terminals) from being in a **Cmd/Ctrl+click
multi-select** (a bolder accent fill). Those rows are queued for the next
agent.
