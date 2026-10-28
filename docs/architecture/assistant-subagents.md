# Assistant sub-agents

Sub-agents (`subagent.rs`, `Orchestrator::spawn_subagent`) are work the
assistant hands off so its turn ends at once. Each is an ordinary interactive
PTY Claude Code agent process in the assistant's own scope.

## Scope

The scope (`AssistantScope`, `Orchestrator::set_assistant_workspace`, called
from `setup`) is a Podium-internal project rooted at
`{app_data_dir}/assistant-workspace`. It is deliberately kept **out of
`Inner.projects`**, so every project call (close, rename, sections, to-dos,
scratchpads, worktrees, Git board, `spawn_agent`, `add_process`, session
memory) answers `ProjectNotFound` for it, and `open_project` refuses that
folder (root compared canonical). It runs on the assistant's backend
(`platform::wsl()`). `spawn_subagent` is refused until the scope is set and
while there is no MCP server (the result only comes back over MCP).

## Kinds and launch

`subagent::launch_args` runs after the user's default args so the restrictions
win. `args` is `--model`/`--effort` only. Both flags, the `--effort` levels and
the CLI's `Task`→`Agent` alias were checked against Claude Code 2.1.301.

Both kinds load **only the user's own settings** (`--setting-sources user`):
no project or local settings files (hooks, permissions), no project/local
`CLAUDE.md` or `.claude/rules` (the CLI's walk up the parent dirs included) and
no settings-based approval of a project `.mcp.json`.

| Kind | Permissions | Podium tools | Disallowed |
| ---- | ----------- | ------------ | ---------- |
| `lookup` | `--permission-mode bypassPermissions` (it runs in the background, a prompt for an MCP read would only block it) | The **lookup route** (see [Built-in MCP server](mcp-server.md#routes)) | `--disallowedTools` Bash, Edit, MultiEdit, Write, NotebookEdit, SendMessage, Agent, Task, AskUserQuestion, plus `mcp__podium__<tool>` for every tool not on the lookup route |
| `full` | The normal permission prompts | The agents' route | `--disallowedTools SendMessage,Agent,Task,AskUserQuestion` |

For `lookup`, the `mcp__podium__<tool>` list is generated from the router
(`PodiumTools::all_tool_names` minus `unprompted_podium_tools`). It is a second
layer under the route, so a tool added later is refused until classified
read-only. Its `--settings` also deny `Read(//{app_data_dir}/mcp/**)` (the
CLI's absolute-path rule spelling, `agent::absolute_path_rule`; Read rules
cover Grep/Glob, and the shell is denied): the token files and every caller
key. A lookup **can still call the user's own other MCP servers** (Slack,
Teams, a ticket system) **without a prompt**, as the assistant itself can. It
is told to change nothing, but Podium cannot enforce that for servers that are
not its own.

For `full`, the CLI's own question dialog would sit in a terminal nobody
watches, so questions go through `ask_assistant` (see
[Questions](#questions)).

Both kinds' `--settings` allow `ask_assistant` and `report_to_assistant`
unprompted. A sub-agent never gets the go-ahead push rules or
`crossSessionInbound`. It has its own identity prompt and no session
(`AgentSession`): never restarted, never remembered by session memory. The
prompt says:

- No project.
- Do the task, then call `report_to_assistant` with two lines and whether
  follow-up is needed.
- Ask via `ask_assistant` only when truly blocked on a decision that changes
  the outcome and cannot be reversed cheaply, then end the turn and wait.
  Otherwise pick the sensible default and state it in the report.
- No peer-message note.

> **A `--settings` in the user's own default args** (Settings → Agents)
> replaces Podium's for sub-agents too (`has_flag`, as for agents). Then a
> lookup shows Claude Code's one-time bypass-mode warning in its terminal (no
> `skipDangerousModePermissionPrompt`) and has no deny rule on the MCP dir, and
> a full one asks before its report (no allow rules). The flags in
> `launch_args` still apply.

## Shared workspace

All sub-agents run in the one `{app_data_dir}/assistant-workspace`. Before
every start (`Hub::scrub_workspace` → `subagent::scrub_workspace`) Podium
removes what an earlier sub-agent may have left at its root that would shape
the next session: `.claude/`, `.mcp.json`, `CLAUDE.md`, `CLAUDE.local.md` (a
symlink is removed, never followed).

Only a sub-agent can have put them there. `--setting-sources user` already
ignores all but `.mcp.json` (approved anyway when the user's settings carry
`enableAllProjectMcpServers`), so the scrub is the second layer. One folder for
all of them keeps the folder trust (see [Folder trust](#folder-trust)) to one
entry.

## Cap and phases

`SubagentBook` is pure bookkeeping. At most `MAX_RUNNING_SUBAGENTS` = 4 run.
Further ones are `queued` (a `notStarted` row, never refused) and start
oldest-first as slots free. A newcomer never jumps the queue. Admission
happens under the same lock as the row insert.

| Phase | Meaning |
| ----- | ------- |
| `queued` | Waiting for a slot |
| `running` | Holds a slot |
| `waiting` | Asked a question, still holds its slot |
| `done` | Ended: reported |
| `failed` | Ended: exit without a report |
| `stopped` | Ended: exit Podium caused |

An ended phase frees the slot and sticks (a stop after a report stays `done`).

Ended rows linger 5 min (`SubagentConfig::linger`, held by the book) and are
then removed with their process. A driver task (`run_subagent_driver`, started
lazily on the first spawn, weak reference) acts on exits handed over by the
exit handler and sweeps on a 10 s tick (also reconciles a missed exit),
publishing the snapshot without them by itself.

The linger counts on **either clock** (`SubagentBook::expired`): `Instant`
stops during a macOS sleep, so a row ended before the lid closed would
otherwise stay up to five awake minutes after the wake. The wall clock alone
could be set back. Each `SubagentInfo` carries `expiresAt` (`endedAt` +
linger), and the frontend lets a row go at that moment on its own (see
[Terminal and activity](terminal-and-activity.md#list)), since a snapshot can reach
a webview late. Timings and the adapter are injectable (`SubagentConfig`).

## Ending

`report_to_assistant` from a sub-agent (`report_subagent_result`) moves it to
`done`:

- The result is kept on the row up to 16 KiB, a memory cap only.
  `list_subagents` / `assistant_subagents_list` return it whole.
- A `finished_nudge` carries the result up to 4000 chars via `quote_line`. Only
  when clipped it adds a pointer to `list_subagents` / `get_process_output`.
- The session is stopped after a 5 s grace so the tool call can finish.
- A second report changes nothing.

An exit without a report is `failed` (a `failed_nudge` with the reason). An
exit Podium caused is `stopped`. A queued one handed a slot by `fill_slots` is
started outside the lock. A stop landing in between ends it
(`subagent_unstarted`), and `do_start` re-checks the phase under the lock and
refuses an ended sub-agent, so no untracked process is left.

### Who is told

The *user's* stop (`assistant_subagent_stop`, and IPC `process_stop`, which
calls `Orchestrator::user_stop_process` and so routes a sub-agent through
`stop_subagent`) nudges the assistant ("stopped by the user, no summary
needed"). The assistant's own `stop_process` (gated destructive) is quiet, as
is a new conversation that clears them. A queued one stopped is cancelled and
never runs.

These nudges go through `quote_line` (name, task and result are agent-written:
whitespace runs collapse to one space and the text is JSON-quoted, so nothing
can start a line that reads like Podium speaking; the other agent nudges use it
too). They are **not** behind the proactivity switch, because the result is
what the user asked for.

## `lasting` nudges

Sub-agent nudges go out via `AssistantManager::nudge_lasting`.
`new_conversation(keep_lasting)` hands a waiting `lasting` item to the new
conversation instead of reporting it undelivered (when the sub-agents were
kept).

`assistant_new_conversation { stopSubagents }`: with stop,
`stop_all_subagents` first marks each ended `stopped` under the lock (so a
racing report finds it ended and nudges nothing), then signals the processes.
It runs once more after the old session is gone, for a spawn that raced the
reset. The `setup` call passes `false` (nothing runs yet).

> Known limit: a `lasting` nudge already written into the turn the reset kills
> is not re-sent (the old session got it, the new one never hears of it).

## Questions

`ask_assistant` → `Orchestrator::ask_subagent_question`. `answer_subagent` →
`answer_subagent`.

### Asking

A running sub-agent that asks becomes `waiting` (`SubagentBook::ask`: the
question on the row, capped at `MAX_QUESTION_CHARS` = 2000, a second ask
replaces it; refused for a queued or ended one). `waiting`:

- holds its slot (`SubagentPhase::is_active`, counted by `running()`),
- never expires (it is not ended),
- counts as live for the quit guard, the keep/stop dialog and
  `isSubagentLive`,
- is skipped by proactivity's stuck checks (the assistant already has the
  question).

The tool answers at once that the answer arrives as the next message, so the
sub-agent ends its turn.

### The assistant's nudge

The assistant gets a `lasting` nudge (`subagent::question_nudge`, the question
via `quote_line`) worded **answer-first**. Answer it yourself with
`answer_subagent` whenever the task, context, memory or to-dos settle it, and
mention it only in the usual short report. Relay only a decision that is
genuinely the user's (preference, risk, money, people, anything it cannot
infer) as a needsYou `add_response` keyed by the process id with a recommended
answer. That item is what pings the desktop, so a question does not by
default. The identity prompt says the same (pinned by tests).

### Answering

`answer_subagent` is refused unless the sub-agent is `waiting`. It types the
answer in through the PTY:

- Control characters dropped (no escape can steer the terminal).
- `\r\n`/`\r` → `\n`.
- A multi-line answer wrapped in bracketed paste (`ESC[200~ … ESC[201~`, which
  Claude Code enables; a bare `\n` would insert a line break in its own input
  box).
- Then `\r` after `ANSWER_ENTER_DELAY` (150 ms), verified on Claude Code 2.1.303
  on a PTY (both lines land in one message).

Then `running`, question cleared.

**The user answering in the terminal** instead: `write_stdin` carrying a `\r`
to a waiting sub-agent resumes it, and so does any later tool call with its
key. A stale `waiting` never blocks anything: a report, a stop or an exit ends
it as from `running`.

## Folder trust

Podium pre-accepts Claude Code's folder-trust dialog for the workspace
(`agent::folder_trust`), so no sub-agent sits on it.

### What Claude Code records

Claude Code records an accepted dialog in its global config file,
`$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`
(`.claude-custom-oauth.json` with `CLAUDE_CODE_CUSTOM_OAUTH_URL`; read from
Podium's own environment, like `claude_config_dir`), as
`projects["<abs path>"].hasTrustDialogAccepted = true`. It is keyed on the
session's physical cwd (`/` separators on Windows) and checked up the parent
dirs to a git root (Claude Code 2.1.301; verified by starting the CLI on a PTY
with a temp `CLAUDE_CONFIG_DIR`).

### When Podium writes it

`src-tauri` hands the path in (`Orchestrator::set_claude_config`, before
`set_assistant_workspace`; unset, as in every test, nothing is touched).
`set_assistant_workspace` writes it once and `Hub::scrub_workspace` checks
again before every start, for a key that went missing.

### How it writes

`ensure_trusted` is a read-modify-write of that one key for the canonical
workspace path:

1. A lock-free read first (key already true → nothing written).
2. Under the CLI's own lock (the `<file>.lock` directory proper-lockfile makes;
   tried 5 × 40 ms, never taken over), a re-read.
3. The key set (`projects` and the entry created when missing).
4. An atomic replace (temp file beside the real one, a symlinked config keeps
   its link, with its permissions, then a rename).

serde_json's `preserve_order` + `float_roundtrip` and the CLI's own layout
(two-space indent, the original's trailing newline or none) write every other
key back byte for byte.

### When it does nothing

The file is never created: a missing file means the CLI never ran with that
config (or Podium resolved another one), and the CLI restores a missing config
from its backups, which a file holding only Podium's key would pre-empt. A
missing file, malformed JSON, an unexpected shape, a held lock or a failed
write leaves the file alone and logs a warn without content. A start never
fails on it. **WSL-backend mode** skips it (the CLI's config is inside the
distro). There, and whenever the write did not happen, the fallback detection
below still applies.

### Fallback detection

For `trust_window` (30 s) after a start Podium watches its output
(`shows_trust_prompt`: header, "Yes, I trust this folder" and "No, exit", no
permission prompt, session not yet started) and reports it like a prompt seen
on screen (`report_agent_waiting`), so proactivity raises it at once. Podium
never answers it: the dialog focuses "No, exit" and ignores an early keypress,
an Enter would end the session. The stuck-sub-agent nudge says the user
answers it once, from the sub-agent list.

## Proactivity

A sub-agent exit is not announced as an agent that "finished" (`is_subagent`
early return). A stuck one (prompt, silence, escalation) gets its own wording
(`stuck_subagent_text`): no project, no `respawn_agent` (refused for
sub-agents). Stop it and start a fresh `spawn_subagent` carrying the answer.
The user can answer in its terminal. `processes_with_subagents` feeds the
dedupe so stuck reports don't restart from scratch.

## Visibility

`ProcessInfo.subagent` (`SubagentMeta { kind, task, phase, endedAt? }`) is set
by `process_info_in`/`process_info`.

- `process_list` / `list_processes` never return them (`list_subagents` /
  `assistant_subagents_list` do, oldest first, as `SubagentInfo`).
- `process:added/updated/status/removed` events still fire for them with the
  internal project id.
- `process_attach`/`process_write`/`get_process_*` work on their ids.
- `start_process`/`restart_process`/`remove_process`/`respawn_agent` are
  refused (Podium starts a queued one and removes ended ones itself).
- Every change publishes a full `subagentsChanged` snapshot (never recorded).
- The quit guard (`has_active_agents_or_terminals`) counts queued and running
  ones, and `shutdown()` stops them.
- `verify_sender` answers `{ fleet: true, kind: "subagent", session }` for one:
  no project, and no process id an agent could act on.
