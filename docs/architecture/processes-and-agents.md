# Processes and agents

- **Processes** are shell command lines run via `$SHELL -lic` (login +
  interactive, so both `.zprofile`/`.bash_profile` and `.zshrc`/`.bashrc`
  PATH/env edits are honoured) in their own PTY + process group. Stop =
  SIGTERM to the group, SIGKILL after a grace period. A user stop is never
  counted as a crash.
- **Supervision** (`process/supervisor.rs`): `RestartPolicy` `never` /
  `on-crash` / `always`; exponential backoff 500ms → 30s (doubling), circuit
  breaker at 5 restarts per rolling 60s, backoff resets after 60s of stable
  running. Manual start fully resets both. Timings injectable
  (`SupervisorConfig`) so tests run in milliseconds.
- **Agent adapters** (`agent/mod.rs`): the `AgentAdapter` trait turns an
  `AgentLaunchCtx` (project root and its backend, prompt, MCP connect info,
  config `extra_args`) into a `LaunchPlan { command, env }`. Availability is
  probed via the login shell (`command -v`) of the backend the agent would
  run on — cached once per backend per run, and `adapters_list` takes the
  project — so nvm/homebrew PATH edits are honoured. Adding an agent CLI = new module + registry entry; no IPC or
  frontend change. Max **8 active agents per project** (recursion guard —
  agents can spawn agents over MCP).
- **Agent session memory** (`agent/session_memory.rs`, setting
  `remember_sessions` in `agents.json`, default off) builds on _Restart
  resumes the session_ (see To-dos below): `session_memory.json` in the app
  data dir, keyed by project root. `Orchestrator::shutdown()` is the only
  place a session is captured: each still-running agent with an `AgentSession`
  is remembered under that session's **current id** — the Podium-assigned
  `--session-id`, or the one its last restart moved it to — with its name, the
  to-dos/scratchpads it owns, its per-spawn `args_override` and its
  `worktree`/`worktree_name` request (`ManagedProcess.spawn_*`, so the
  restored identity prompt matches), in spawn order (`ManagedProcess.seq`, a
  per-run insertion counter a restart keeps) so a restore keeps the sidebar
  order, all in one write. Before that, each id follows a `/clear` (see
  _Restart resumes the session_); read outside the lock and within 200 ms, and
  no process starts on that exit path. An agent without an `AgentSession` is
  **not** remembered: Auggie (no session flags), and an agent whose own
  `extra_args` pick the session (`--resume`, `--continue`, …) — a restore
  would put Podium's `--resume` next to the user's flag.

  The next `open_project` on that root, with the setting on, restores the
  entries one at a time, oldest first: `front` peeks at the oldest, and only
  once its outcome is known — restored, or skipped — does `remove` drop
  exactly that entry (matched on the whole entry, so a fresh capture of the
  same session survives) in one write. A failed save is logged and never
  cancels the restore. An app that exits part-way (a quit, a crash) therefore
  still has the entry in flight and the rest next time, and `remember_all`
  dedupes on the session id, so a leftover cannot turn into a duplicate. Each
  is respawned through the restart path: `spawn_agent_inner` with
  `AgentSpawn.resume_session_id` sets the new process's `AgentSession.id` to
  the remembered id and hands `restart_launch(…, stored = true)` — the
  `--resume` line, no prompt — to `do_start`, so a resume that crashes within
  `RESUME_FAILURE_WINDOW` gets `resume_failed` and its next start is fresh,
  exactly as for a restart. Before that, an entry is skipped when it is older
  than 14 days (`session_memory::MAX_AGE`, for a project that kept failing to
  open), is not a hyphenated UUID (the file is user-writable and the id lands
  unquoted on the shell line), or `AgentAdapter::session_exists` (disk, or the
  WSL distro's login shell — `Orchestrator::session_stored`, shared with
  `start_process`) no longer finds it: a pruned session would only come back
  as a fresh agent without context or prompt.

  A `worktree` request whose named worktree already exists under
  `.podium/worktrees/` (`worktree::exists`) is dropped, and so is the "suggest
  worktree" offer (`AgentSpawn.suggest_worktree = false`), so the agent
  neither makes a `-2` copy nor asks about one. `exists` matches the slug
  only: a worktree `create` de-duplicated to `<slug>-2`, or another agent's
  under the same slug, is taken as is. An unnamed request cannot be matched
  and is passed on. After the spawn the agent is re-assigned the
  to-dos/scratchpads that still exist, are not done/archived and no live agent
  claimed (re-assigned, not handed over: a composed prompt would land in the
  resumed conversation as a new message).

  `shutdown()` sets `Inner.shutting_down` first, under the lock. From then on
  `spawn_agent_inner` (before it inserts a row) and `do_start` refuse with
  `CoreError::ShuttingDown` under that same lock, so nothing — a restore in
  flight, a supervised restart — launches after the stop sweep; a start
  refused after the insert removes its row again, so no never-started row is
  left. The restore loop stops at the flag. A spawn refused with
  `ShuttingDown`, `ProjectNotFound` or `AgentLimitReached` stops it too,
  leaving that entry and the rest on the list for the next open. With the
  setting off, opening a project drops its list (`clear`), and turning the
  setting off clears every root. The MCP server is started synchronously in
  `setup` so a restored agent never spawns before the MCP connect info (its
  `--mcp-config`, `-n` and identity) is set.

**Windows ConPTY.** Process-group stop is Unix-only; Windows terminates the ConPTY child directly. The PTY reader answers ConPTY's startup `ESC[6n` cursor query itself and strips it from the output, since ConPTY holds the child until it is answered and xterm.js only exists once a terminal is opened — see `CursorHandshake` in `process/pty.rs`.
