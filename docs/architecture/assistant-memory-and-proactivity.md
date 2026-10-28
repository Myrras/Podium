# Assistant memory and proactivity

How the personal assistant (see [assistant.md](assistant.md)) keeps its memory
and how Podium speaks first. Proactivity lives in `proactivity.rs`; its setting
`proactive` sits in the `assistant` block and defaults **on**.

## Memory

**Memory** is a separate `assistant-memory.md` in the same dir: free-form
markdown appended to the identity prompt at spawn under `# Your memory`, so it
survives new conversations. The assistant maintains it itself through the
`update_assistant_memory` MCP tool (full replace — an empty string clears it),
and the user can read, edit or clear it in the panel's memory view. It is read
fresh from disk on every spawn, so hand-editing the file works. A change
reaches a *running* session only on its next spawn.

## Proactivity

**Proactivity** lets Podium speak first. One task watches the `EventBus` and,
on a slow tick, the process/to-do state. The watcher holds only a weak
reference, so it ends with the orchestrator. Switching the setting off
silences all of it.

### Triggers

- An agent that exited (finished or crashed).
- An agent the UI reported an input prompt on (`process_report_waiting` →
  `Orchestrator::report_agent_waiting` / `agents_awaiting_input`).
- A running agent that has printed nothing for 5 minutes, so it is probably
  waiting on a question (`Orchestrator::idle_agents`, measured from the
  scrollback's last append, or from the process start when it never printed).
- A 6-hourly sweep for to-dos open longer than a week.

The prompt report and the silence timer share one dedupe record, and a
reported prompt wins the wording, so a blocked agent is announced once — at
the next tick rather than five minutes later. The silence timer stays as the
backstop for agents whose terminal has never been opened, where there is no
screen to read.

The **day-start summary** used to be a fourth trigger here. See
[Day start](#day-start).

### Escalation

That record then **escalates once, and stops**: an agent still blocked
`escalate_after` later (default 10 minutes) is raised a second time. The
wording asks the assistant to say which answer it would pick and why —
something the user can wave through in a word — and, once that answer is
agreed, to call `respawn_agent` (nothing can type into an agent's terminal).
That is the cap, `MAX_RAISES = 2` per question: raising it every
`escalate_after` for as long as the agent lived once produced a "no change"
reply every ten minutes for hours.

The record is kept **for as long as the agent lives**, not only while it looks
stuck: an agent that prints one line and falls silent again is waiting on the
same unanswered question, and re-reporting it from scratch is exactly the
repetition this prevents.

A **new question cycle** starts when the user types into the agent's terminal
after the last raise. `Orchestrator::write_stdin` stamps
`ManagedProcess.last_input` (`last_user_input`), and a tick that finds it
newer than `Waiting.last` drops the record, so the next stall is raised from
scratch. A write counts as the user typing unless it is a short escape
sequence (at most 16 bytes starting with ESC — xterm.js's own focus reports
and cursor/device-attribute replies, which would otherwise end the question
on every window focus change). A lone arrow key is skipped too. Otherwise
only the agent exiting (or vanishing) clears the record.

### Viewing

**Viewing** is what keeps the assistant quiet about the agent the user is
looking at. The frontend's activity monitor reports it (`process_report_viewing`
→ `Orchestrator::set_viewing`, in memory only, at most one process, never a
non-agent). The flag lives on `ManagedProcess.viewing`, so a removed process
drops it by itself, and `ProcessInfo.viewing` carries it to
`list_processes`/`get_process_status`.

Proactivity skips the viewed agent: no finished/crashed nudge, and no
idle/waiting nudge. Its question is recorded as already raised instead
(`Watcher::mark_seen`: one raise toward the cap, escalation clock restarted
every tick it stays in view), so leaving the agent never fires a nudge at
once.

- `add_response` for a `process_id` that is the viewed agent stores nothing
  and answers with a short "skipped, the user is viewing that agent" text (not
  an error; `Orchestrator::add_assistant_response` returns `Option`).
- A `report_to_assistant` from the viewed agent is still delivered, with one
  line appended telling the assistant not to mention it or file an item.
- The identity prompt says the same of `viewing: true`. Reading the agent's
  output stays fine.

### Nudges

**Triage** (`triage.rs`, `TriageManager`): the watcher no longer hands a
nudge straight to the assistant. `Watcher::nudge` pushes it
(`Orchestrator::push_triage`, a `TriageItem { text, context, wants_output, process_id, needs_running }`) into a
debounce buffer; the push into an empty buffer starts the one worker
(`triage::run_worker`, weak orchestrator ref), which waits out
`ProactivityConfig::triage_window` (60 s, injectable), takes the whole
batch and classifies it in **one** Haiku call. Nudges arriving meanwhile
wait for the next window, so one triage runs at a time; the worker retires
under the buffer's lock (`take_batch`) when it finds it empty, after
shutdown, or with proactivity off (that batch is dropped, and the next push
starts a fresh worker at once), so no push is stranded. A drop guard
releases the `worker` flag if the task dies mid-batch (panic, abort), and
the buffer keeps the newest 50 nudges, so a dead worker can neither
silence proactivity nor grow the buffer. The
call is a **one-shot** headless child per batch, spawned like the sweep
(login shell, `--cd` in WSL mode, `spawn_gated`, `hide_console_window`,
stderr dropped): `exec claude -p --input-format stream-json --output-format
stream-json --verbose --model haiku --tools '' --strict-mcp-config
--disable-slash-commands --safe-mode --no-session-persistence
--system-prompt <TRIAGE_IDENTITY>` — no built-in tools, no MCP server at
all (strict with no `--mcp-config`), no skills/commands, no CLAUDE.md,
hooks or plugins (`--safe-mode`; auth works as normal, unlike `--bare`,
which needs an API key), and its own short system prompt instead of Claude
Code's default. Measured on CLI 2.1.311: ~0.6k input tokens to start,
against ~5.3k without `--safe-mode`. One turn is written, stdin closed,
and every prose block of the turn read (newline-joined) until the `result`; the child is killed
after. The prompt (`triage_prompt`) lists the last 10 forwarded nudges as
context (memory only, so a repeat can be told from news) and the batch
numbered from 1, all through `quote_line`; a "finished" nudge also carries
the agent's ANSI-stripped last 30 output lines (`context`, cut to the last
1500 characters with `truncate_tail` — the PR URL or question is at the
end — never sent to the assistant), since the triage has no tool to read them.
The watcher only sets `wants_output` on that item; `run_worker` reads the
output (`proactivity::last_output`) right after `take_batch`, before
classifying — not at exit time — so the batch has waited a window and the PTY
reader has stored the final chunk, the watcher's event loop never sleeps per
exited agent, and nothing is read for a batch dropped with proactivity off. The identity is
the Responses rules cut down to two answers: FORWARD a crash/failure, a
blocked or still-blocked agent, finished work with something for the user,
lingering to-dos not already forwarded identically, anything unsure; DROP a
routine finish with nothing left for the user, a "no change"/"still
waiting" repeat, an identical lingering sweep; the escalation of a
still-blocked agent (it asks for a pick and `respawn_agent`) is always
forwarded, even when it resembles an earlier note. The reply must be one
`<n>: FORWARD|DROP` line per nudge (`parse_verdicts`: case and whitespace
aside, a bare code fence ignored; every number exactly once) — **anything
else, a turn error, a timeout (`TRIAGE_TIMEOUT`, 90 s, `TriageConfig`) or a
failed spawn forwards the whole batch**, so a nudge is never lost to
triage, and each such fallback — an unparseable reply included
(`try_parse_verdicts` returns `None`) — is logged at `warn` (batch size and a
short reason, never output or reply text) so a CLI that rejects a flag is not a
silent no-op; a full buffer evicting its oldest nudges is a `warn` too.
The forwarded nudges go to the assistant together through
`Orchestrator::assistant_nudge_all` (`AssistantManager::nudge_all`): texts
already waiting in the queue and repeats within the batch are dropped, the
rest is one nudge joined with a blank line, delivered as one turn when the
assistant is idle and queued as that one joined item while a turn runs. The
dedupe is partial: texts already waiting as a single nudge and repeats within
the batch are dropped, but a batch already queued as one joined item is not
matched by a later identical single nudge. Dropped ones are logged at debug
as a count, never their text. At delivery each is checked again (`is_stale`), since the batch waited
at least a window: a nudge about an agent the user is now viewing is dropped,
and so is a stuck-agent one (`needs_running`) whose agent is gone or no
longer running; a to-do sweep nudge has no `process_id` and is unaffected.
The
proactivity switch is checked at push, before the call and before
delivery, so off means nothing runs. Shutdown (`TriageManager::shutdown`)
wakes the worker mid-window or mid-call, which kills the child and drops
the buffer; `Orchestrator::shutdown` calls it before killing the assistant,
so a worker cannot start a session afterwards. Binary and cwd follow `configure_assistant`; `configure_triage`
sets the timeout. Only the watcher's own nudges are triaged: an agent's
`report_to_assistant`, sub-agent results and questions (`nudge_lasting`),
proposal outcomes and user turns go straight to the assistant.

Each forwarded nudge becomes a `[Podium] …` turn (consecutive waiting ones
merged) plus a `notice` event, so the chat says why the assistant is speaking
unprompted. Timings are injectable
(`ProactivityConfig`) so tests run in milliseconds.

A nudge **starts the session when there is none** (`ensure_session`, the same
path `send` uses). Otherwise the user only learned an agent had finished
whenever they next happened to type, which is how a weekend's worth of
`[Podium]` lines came to arrive in one pile. This reverses the original
phase-4 rule; the reasoning behind that rule ("a fresh session would not know
what the nudge is about") no longer holds now that every launch starts blank
and `assistant-memory.md` carries the context. A spawn that fails falls back
to queuing, so a nudge is never lost.

Because these are event-driven — Podium already sees an exit or a silent
agent for free — **a quiet day costs no turns at all**, and with triage a day
of routine finishes costs Haiku calls, not assistant turns.

### Unblocking a stuck agent

**Unblocking a stuck agent** is `respawn_agent` (`Orchestrator::respawn_agent`):
it stops the agent and spawns a fresh one carrying the answer in its prompt.
Nothing can type into a running agent's terminal, so a session blocked on a
question is otherwise dead weight. A plain `stop_process` + `spawn_agent` pair
would lose the one thing the caller cannot reassemble — the **to-dos the old
session owned**, which are what carry the task over.

Those (and the adapter) are inherited. The replacement is deliberately *not*
given the old name, so the two sessions are told apart in the sidebar. An
agent that owns no to-do has nothing to inherit, so `task` must be supplied as
well or the call is refused rather than burning a session on an answer to a
question nobody asked it. It is classified **destructive**, so
`confirm-destructive` (the default) shows the user a card first.

## External sweep

Alongside the event-driven lane runs a slow one: the **external sweep**, every
`poll_interval_minutes` (default 10; `0` is off), checking the enabled
sources for anything needing the user. It runs **on Haiku in a session of
its own** (`sweep.rs`, `SweepManager`), never in the assistant's: a second
long-lived headless child, `exec claude -p --input-format stream-json
--output-format stream-json --verbose --model haiku` (`SWEEP_MODEL`)
`--setting-sources user --permission-mode bypassPermissions
--disallowedTools Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task,
SendMessage,AskUserQuestion,WebFetch,WebSearch,Read,Grep,Glob,LS,mcp__podium,
mcp__podium__<tool>…
--settings '{"permissions":{"deny":["Read(//{app_data_dir}/mcp/**)"]}}'`
plus its own short read-only identity (`--append-system-prompt`; the whole
answer goes in the final message, since only the turn's closing `result` text is
read), spawned the way the assistant is (login shell, `--cd` in WSL mode,
`spawn_gated`, `hide_console_window`, stderr dropped). It is passed **no
Podium `--mcp-config`** and runs in an empty directory of its own,
`{app_data_dir}/sweep-workspace` (created when missing; `set_launch` derives
it and the denied `mcp` dir from the assistant's cwd, the app data dir).
The sweep runs in its own cwd (`sweep-workspace` under the app data dir), so
Slack/Teams MCP servers must be user-scope; a local-scope server added from the
app-data dir is invisible to it. Such a sweep has no Slack/Teams tools and still
answers normally: it stays silent (`NOTHING_TO_REPORT`) or reports "no access"
as a finding. It is not a failure, so the not-working notice never appears (the
streak counts only spawn failures, a session ending mid-turn, timeouts and
`is_error` results). The identity prompt asks it to name an unreachable source
in one line instead of staying silent. Slack and Teams need no integration: the session inherits the user's own
user-scope MCP servers (`--strict-mcp-config` deliberately not passed, or
they would be hidden), so this is `poll_prompt` wording plus a timer. That
also loads the user's own `podium mcp-bridge` server (the agents' route:
spawn/stop/remove agents, ungated), whose input here is untrusted
Slack/Teams text — so the whole `podium` server is denied: the
server-level rule `mcp__podium` (the CLI's form for every tool of a server;
not run against a real CLI here) and, as a second layer generated from the
router like a lookup's, `mcp__podium__<tool>` for each tool. A bridge
registered under another name is not covered. The sweep reads only through
its Slack/Teams MCP servers, so every local file reader (`Read`, `Grep`,
`Glob`, `LS`) is denied too, with `WebFetch`/`WebSearch` so an injected
message cannot carry data out; the `--settings` deny of `Read` on the MCP
config dir (token files, caller keys) stays as a second layer, and
`--setting-sources user` keeps project and local settings and `CLAUDE.md`
files out. The open surface is the user's own other MCP servers: **like a
lookup sub-agent, the sweep can still call them (Slack send, Teams
post, …) without a prompt, and Podium cannot enforce its read-only wording
on those; Haiku reading untrusted messages is also more injectable than the
main assistant was.** Binary and cwd follow `configure_assistant`;
`configure_sweep` (`SweepConfig`) also sets the turn timeout. It is spawned
lazily on the first sweep and **kept alive between sweeps**, so "since you
last looked" holds within a run; a session that died is respawned with
`--resume <its session id>` (in memory only; dropped when a resumed spawn
never comes up, whether the turn's write or read failed, or it hung before
saying anything and timed out) and killed on
shutdown. Each sweep is one turn read straight off its stdout (no output
task, no events, no transcript): the `result` event's text is the answer (an empty one, or a turn cut off at
the turn limit, is quiet; the last prose block is only the fallback for a
normal end without `result` text). The hand-off words the notice from the
sources enabled *then*, and `shutdown()` stops the sweep before the assistant.
**One sweep at a time** (`SweepManager::begin`, a busy flag held by
`SweepRun` until the hand-off is done): the watcher runs it on its own task
(weak orchestrator ref) and a tick that finds one still running is skipped.
A turn over `SWEEP_TURN_TIMEOUT` (5 min, injectable) kills the child; the
next sweep respawns it. Shutdown wakes a running turn at once (`Notify`),
kills the child and counts as neither a finding nor a failure. Failures are
logged at debug/warn without content.

**A broken check must not look like a quiet one.** A spawn failure, a
session that ends under the turn, a timeout or a turn the CLI ended with an
error counts as a failed sweep (`FailureStreak`). After
`FAILURES_BEFORE_NOTICE` (3) in a row the sweep returns
`SweepOutcome::NotWorking(reason)` once, and the watcher hands the assistant
`sweep::not_working_text` through the same `assistant_poll` path as a
finding (a Podium-owned reason string, never CLI output); further failures
are quiet until a sweep works again and fails three more times.

A reply carrying `NOTHING_TO_REPORT` on a line of its own
(`assistant::says_nothing_to_report`, the same matcher as below), an empty
one, an error or a timeout reaches **nobody**: the assistant gets no turn,
the panel no row or event. A real finding is handed to the assistant as
`[Podium] The routine check of <sources> found: <findings via quote_line,
capped at 4000 chars>. Tell the user what needs them, following your
Responses rules.` (`sweep::finding_text`), via `Orchestrator::assistant_poll`
→ `AssistantManager::poll` — re-checking the proactivity switch and the
poll settings first, since they may have changed during the sweep. With
both sources off or proactivity off no sweep runs at all. That turn is
delivered as `PendingKind::Poll`: unlike a proposal-outcome `Notice` it
leaves the same `Notice` row a nudge does (`record_before_write`, stored as a
`nudge` row), so the panel folds the turn and its actions into one faint
"Podium check" line rather than mixing them with the user's own. Unlike
nudges, polls are not merged or deduped in the queue.

## The silence contract

**The silence contract**: a sweep that found nothing must say nothing. The
prompt instructs the sweep session to answer with exactly `NOTHING_TO_REPORT`,
and the sweep drops such a reply before the assistant sees anything. The
assistant's own stream applies the same rule to any nudge's answer: a `Message`
carrying that sentinel **on a line of its own** — anywhere in the
reply — is dropped before it is recorded or published: no chat row, no
transcript row.

It is deliberately not an exact-whole-message match: models keep prefixing the
word with the very summary the prompt forbids ("Same messages as last
check.\n\nNOTHING_TO_REPORT"), and matching exactly let all of that through as
a notification — the noise this contract exists to prevent. A reply that
quotes the sentinel *mid-sentence* while reporting something real is still a
real reply.

## Desktop notifications

**Desktop notifications come from the Responses list only.** A chat reply, a
proposal card and a `[Podium]` nudge never ping; a **new Responses item**
does (see [Terminal & activity](terminal-and-activity.md#assistant-notifications) for the frontend
half). So what the assistant may file *is* the notification policy, and the
identity prompt spells it out.

| Item | Filed only for |
| ---- | -------------- |
| `needsYou` | An agent blocked on a question only the user can answer (one item per question, replaced when the question changes). An external message (Slack/Teams/mail) that directly asks the user something. Never a manual step the user already knows about (merging a PR, `just install-signed`, testing a build). |
| `fyi` | Finished work the user asked for that has something to look at (a result, a PR/MR URL, a posted review, the answer). An agent that crashed or failed. An external heads-up that asks nothing. An agent the assistant stopped or respawned on its own. |

Never filed: "no change"/"still waiting" reminders or a repeat of an open
item, a routine check that found nothing, lookup or sub-agent results it used
itself, an agent that finished with nothing left for the user, or anything
about the viewed agent. A nudge that brings nothing new about an agent already
reported gets no new chat reply either. Unit tests pin the key phrases.

The old `ACTION_REQUIRED` marker and `TurnEnd.notify` (with the
`turn_from_user` bookkeeping that fed it) are gone. `take_action_required`
stays, only to strip a stray marker an assistant writes out of habit before
anything is recorded or published. A reply that is *only* the marker is
dropped like a swallowed one.

## Proposals file their own item

**Proposals file their own item.** When the autonomy gate parks a proposal,
`AssistantManager::propose` adds a `needsYou` Responses item — "Confirm or
reject: `<tool> <summary>`", `source` `"proposal"`, no process/to-do — and
keeps its id on the `PendingProposal`. Resolving (`resolve_proposal`),
invalidating (`invalidate_proposals_for_project`) or dropping it
(`new_conversation`) takes the item off again.

Proposals are memory-only but the list is on disk, so `new_conversation`
(every launch runs one) also sweeps every item whose source is `proposal`,
which is how one a restart orphaned goes away. The assistant must not file one
itself.

A pending proposal is also dropped when **its project closes**
(`invalidate_proposals_for_project`, keyed off the call's `project_id`
argument): confirming it later could only fail, so the card resolves itself
with the reason and the assistant is told.

## Day start

The **day-start summary** is the panel's **"Start my day" button**
(`assistant_day_start`, shown in the empty state). It used to be a proactivity
trigger, but a nudge never spawns the CLI, so as a nudge it only surfaced
whenever the user next happened to type — which is exactly the complaint that
moved it.

The button sends `DAY_START_PROMPT` through the ordinary `assistant_send` path
instead: it spawns the session, shows as a user turn and respects the queue
setting. It asks for open work across all projects *prioritised with a
reason*, and to ask the user rather than guess when the to-dos and agents do
not settle the order. There is deliberately no once-a-day lock — pressing it
is explicit, so refusing would only annoy.

## Turning the assistant off

`enabled` in the same `assistant` block (default on;
`Orchestrator::set_assistant_enabled`, IPC `assistant_enabled_get/set`). Off:
the session is killed deliberately (no `exited`), proposals and their
Responses items are dropped, the queue is dropped with its undelivered
messages reported via `queueNotice`, and all sub-agents are stopped quietly.
From then on `assistant_send`, `assistant_day_start` and
`assistant_response_reply` fail with `CoreError::AssistantDisabled`
(`assistantDisabled`), nudges and the external sweep are dropped per call
(`assistant_nudge` / `assistant_poll` and the manager's `deliver_own`), and
`report_to_assistant` / `spawn_subagent` over MCP answer plain text ("the
assistant is turned off; nothing was delivered"), so no session is ever
spawned. Transcript, memory and the Responses list stay on disk. Turning it on
spawns nothing until the next send or nudge. The manager mirrors the setting
in an atomic, preset when the settings path is set, so startup honours it too.

## Settings

**Settings → Assistant** holds the proactivity toggle plus the autonomy mode
(the chat header keeps its own autonomy picker — both mirror the one stored
value).
