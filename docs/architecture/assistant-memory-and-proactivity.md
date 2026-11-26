# Assistant memory and proactivity

How the personal assistant (see [assistant.md](assistant.md)) keeps its memory
and how Podium keeps the user's overview (the Responses list). The assistant
is **silent by default**: it never writes to the chat on its own account.
The watcher lives in `proactivity.rs`; its setting `proactive` sits in the
`assistant` block, defaults **on** and is the Settings → Assistant → Notifications →
"Desktop notifications" switch.

## Memory

**Memory** is a separate `assistant-memory.md` in the same dir: free-form
markdown appended to the identity prompt at spawn under `# Your memory`, so it
survives new conversations. The assistant maintains it itself through the
`update_assistant_memory` MCP tool (full replace — an empty string clears it),
and the user can read, edit or clear it in the panel's memory view. It is read
fresh from disk on every spawn, so hand-editing the file works. A change
reaches a *running* session only on its next spawn.

## Proactivity

One task watches the `EventBus` and, on a slow tick, the agents. It holds only
a weak reference, so it ends with the orchestrator. It files **items** on the
Responses list (see [assistant.md](assistant.md#responses)) and writes nothing
to the chat. Agent items are filed, and the external sweep runs, whenever the
assistant is on; `proactive` gates only desktop notifications (the frontend's
call).

### Items

| Item | Key | Reason | Filed when | Goes when |
| ---- | --- | ------ | ---------- | --------- |
| Agent waiting | `agent:<id>` | `question` | the agent waits on the user (Claude Code's own `waiting` report, else the prompt the UI saw: `process_report_waiting` → `report_agent_waiting` / `agents_awaiting_input`) | it stops waiting (only a `question` item goes; a review filed meanwhile stays); dismissed at once when the user opens, types into or replies to it |
| Agent crashed | `agent:<id>` | `crashed` | exit that was not a stop on request | restarted or removed; dismissed at once when the user opens it |
| Ready to review | `agent:<id>` | `review` | `report_to_assistant`, or the agent completing its to-do (`file_review_response`) | the user types into the agent, or it is removed; dismissed at once when the user opens it |
| Message | `msg:<source>:<conversation>:<thread>` | `message` | the external sweep (below) | replied or aged out |

A second review while the first is still listed (open or parked), e.g.
`complete_todo` then `report_to_assistant`, only updates its text
(`ResponseStore::upsert_review`): id, fingerprint and state stay, so it pings
once. The newer text wins, but a bare "Completed to-do: …" never replaces an
agent's summary. Agent item text is clipped to `MAX_RESPONSE_TEXT_CHARS`, like
the assistant's own items.

**Opening or answering an agent takes its item off at once**, not on the
watcher's next tick (30 s). Every path dismisses (soft), so the item stays
on disk with its fingerprint and the watcher re-filing the same episode is
`Unchanged`, out of sight; a new question (new `since`), crash or review has a
new fingerprint and comes back open:

- opening: the row or front-desk figure click calls `dismissResponse` for an
  `agent:` key (the assistant's own items stay), and `set_viewing` dismisses
  the viewed agent's item (any other way of opening it);
- typing into the agent: `write_stdin` removes a review and dismisses the
  rest. The watcher counts a wait as a new question only when it **began
  after** the typing (`agents_awaiting_input`'s report time, or `now -
  state_ms` of Claude Code's `waiting`); an answered prompt still on screen
  keeps its episode, so its item stays dismissed;
- a reply from the row: `ResponseStore::take` leaves an agent item behind
  dismissed instead of removing it, `restore` opens it again in place (see
  [assistant.md](assistant.md#dismissing-and-replying)).

**Known limit: agent items do not survive a Podium restart.** Process ids are
new on every run, so the first tick's `reconcile_agent_responses` drops every
`agent:` item an earlier run left (a pending review included).

The fingerprint is `question:<since>` / `crashed:<since>` / `review:<ms>` /
the newest message's time, so re-filing the same episode is `Unchanged`:
nothing is published and nothing pings. An agent the user is viewing gets
none of the agent items. There is no silence timer, no escalation and no
triage: the old nudges (silent agent, lingering to-dos, escalation, the
Haiku FORWARD/DROP triage) and the "Start my day" button are gone.

### Notifications

Desktop notifications come from the Responses list only (frontend half:
[Terminal & activity](terminal-and-activity.md#assistant-notifications)). A
new item pings only for reason `crashed` or `review`, plus the existing prompt
ping for a waiting agent (`notifyAgentWaiting`; the `question` item adds none,
so there is one ping per episode). `message` items never ping. A proposal's
`needsYou` item (below) pings. All of it is gated by the "Desktop
notifications" setting (`proactive`).

### Viewing

**Viewing** is what keeps the assistant quiet about the agent the user is
looking at. The frontend's activity monitor reports it (`process_report_viewing`
→ `Orchestrator::set_viewing`, in memory only, at most one process, never a
non-agent). The flag lives on `ManagedProcess.viewing`, so a removed process
drops it by itself, and `ProcessInfo.viewing` carries it to
`list_processes`/`get_process_status`.

Proactivity skips the viewed agent: no item for it. `set_viewing` dismisses
its item at once; on the next tick its waiting episode is recorded as seen
(`Asking::seen`), so leaving the agent never files that question afterwards.

- `add_response` for a `process_id` that is the viewed agent stores nothing
  and answers with a short "skipped, the user is viewing that agent" text (not
  an error; `Orchestrator::add_assistant_response` returns `Option`).
- A `report_to_assistant` from the viewed agent is still delivered, with one
  line appended telling the assistant not to mention it or file an item.
- The identity prompt says the same of `viewing: true`. Reading the agent's
  output stays fine.

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
app-data dir is invisible to it. Such a sweep has no Slack/Teams tools: the
prompt asks it to name the unreachable source in one plain line instead of
JSON, which is an unreadable answer and so a failed sweep (see below). Slack and Teams need no integration: the session inherits the user's own
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
`configure_sweep` (`SweepConfig`) also sets the turn timeout. **Every sweep
spawns a fresh session** (no `--resume`, no state between sweeps) and ends it
afterwards: stdin is closed so the CLI can stop its MCP servers, then it is
killed after `SESSION_EXIT_GRACE` (5 s); a failed or cut-short turn is killed
at once. Each sweep is one turn read straight off its stdout (no output
task, no chat events, no transcript): the `result` event's text is the answer (a
turn cut off at the turn limit has none; the last prose block is only the
fallback for a normal end without `result` text). The hand-off follows the
settings *then*, and `shutdown()` stops the sweep before the assistant.
**One sweep at a time** (`SweepManager::begin`, a busy flag held by
`SweepRun` until the hand-off is done): the watcher runs it on its own task
(weak orchestrator ref) and a tick that finds one still running is skipped.
`Orchestrator::begin_external_sweep` announces the run
(`SweepRun::announce`): it publishes `AssistantEvent::Sweeping { active:
true }` at once and `{ active: false }` when the `SweepRun` drops, so the
end is announced on every path (answer, failure, timeout, shutdown, a
panicking task). Runtime-only, never recorded; the frontend mirrors it as
`assistantStore.sweeping`, and the bot (`useAssistantDesk`, state `reading`)
holds letters while it is set and nothing more urgent is going on. There is
no getter: a webview that loads mid-sweep shows it only from the next sweep.
A turn over `SWEEP_TURN_TIMEOUT` (5 min, injectable) kills the child.
Shutdown wakes a running turn at once (`Notify`),
kills the child and counts as neither a finding nor a failure. Failures are
logged at debug/warn without content.

**A broken check must not look like a quiet one.** A spawn failure, a
session that ends under the turn, a timeout, a turn the CLI ended with an
error or an answer that cannot be read counts as a failed sweep
(`FailureStreak`). After
`FAILURES_BEFORE_NOTICE` (3) in a row the sweep returns
`SweepOutcome::NotWorking(reason)` once, and the watcher hands the assistant
`sweep::not_working_text` via `Orchestrator::assistant_poll` →
`AssistantManager::poll` (a Podium-owned reason string, never CLI output);
further failures are quiet until a sweep works again and fails three more
times. That notice is the only thing the sweep ever puts in the chat. It is
delivered as `PendingKind::Poll`: it leaves the same `Notice` row a nudge
does (`record_before_write`, stored as a `nudge` row), so the panel folds it
into one faint "Podium check" line.

**The sweep is stateless and files Responses items itself.** Every sweep asks
(`assistant::poll_prompt`, which carries the current time) for the *full* set
of messages from the last `MESSAGE_WINDOW_DAYS` (2) days still waiting for the
user — a message that asks the user something with no reply from them — as one
strict JSON object:

```json
{"messages":[{"source":"slack","conversation":"<id>","thread":"<id or the conversation id>",
  "sender":"<name>","place":"Slack, direct message","newest":"<RFC 3339>","text":"<short gist>"}]}
```

`sweep::parse_messages` reads it (from the first `{` to the last `}`, so a
code fence or a line of prose around it is tolerated). A malformed entry
(unknown source, missing field, bad time, an id over 200 chars) is dropped on
its own, logged at debug without content; entries older than the window are
dropped in code too. An answer with no readable `messages` array is a failed
sweep and files and removes **nothing**, so a bad run never wipes the list.
`sender`/`place` are clipped to 40 chars, `text` to 300.

On hand-off (re-checking enabled and the poll settings, and
dropping entries of a source switched off since),
`Orchestrator::sync_message_responses` upserts each as a `needsYou` item, key
`msg:<source>:<conversation>:<thread>`, reason `message`, `sender`,
`source` = place, fingerprint = `newest`, then drops every `msg:` item not in
the set: a replied or aged-out conversation leaves the list. A *dismissed* one
stays until its `newest` is over 2 days old, so a sweep that misses a thread
once cannot bring it back as new when the next one finds it. Same `newest` is
`Unchanged` (nothing written, a dismissed item stays dismissed); a newer one
is `Changed` and back to open. Message items never ping (the frontend skips
reason `message`). With both sources off or a zero interval no sweep runs at
all; the `proactive` setting does not affect it.

## The silence contract

A turn Podium starts for itself (a sub-agent result, a sweep notice) that
brings nothing must say nothing. The assistant's own stream drops a
`Message` carrying `NOTHING_TO_REPORT` **on a line of its own** — anywhere in
the reply — before it is recorded or published. It is not an exact-match
check: models prefix the word with the very summary the prompt forbids. A
reply that quotes the sentinel mid-sentence is a real reply.

The old `ACTION_REQUIRED` marker and `TurnEnd.notify` are gone.
`take_action_required` stays only to strip a stray marker before anything is
recorded or published.

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

## Turning the assistant off

`enabled` in the same `assistant` block (default on;
`Orchestrator::set_assistant_enabled`, IPC `assistant_enabled_get/set`). Off:
the session is killed deliberately (no `exited`), proposals and their
Responses items are dropped, the queue is dropped with its undelivered
messages reported via `queueNotice`, and all sub-agents are stopped quietly.
From then on `assistant_send` and
`assistant_response_reply` fail with `CoreError::AssistantDisabled`
(`assistantDisabled`), nudges and the external sweep are dropped per call
(`assistant_nudge` / `assistant_poll` and the manager's `deliver_own`), and
`report_to_assistant` / `spawn_subagent` over MCP answer plain text ("the
assistant is turned off; nothing was delivered"), so no session is ever
spawned. Transcript, memory and the Responses list stay on disk. Turning it on
spawns nothing until the next send or nudge. The manager mirrors the setting
in an atomic, preset when the settings path is set, so startup honours it too.

## Settings

**Settings → Assistant → Notifications** holds the "Desktop notifications" toggle (`proactive`); **Settings → Assistant** holds the autonomy mode
(the chat header keeps its own autonomy picker — both mirror the one stored
value).
