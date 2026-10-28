# Personal assistant

The personal assistant (`assistant.rs`) is a workspace-wide chat panel backed
by **one long-lived headless Claude Code child**. This file covers its
process, identity, autonomy, persistence, message queue, attachments and
Responses list.

## Process and stream

The child is a plain piped process (`$SHELL -lic 'exec claude -p
--input-format stream-json --output-format stream-json --verbose
--permission-mode bypassPermissions --disallowedTools …'`), not a PTY, not in
the sidebar process list. `--permission-mode bypassPermissions` is required
because `-p`/stream-json mode has no TTY for an interactive permission
prompt. Without it the CLI would stall asking approval for every Podium MCP
tool it can never answer. Deny rules still take precedence over it (see
[Autonomy](#autonomy)), so this does not loosen what the assistant can touch.

It is spawned lazily on the first message and killed on shutdown. A session
that died is replaced on the next turn. User turns are written to its stdin
as one JSON line each. Its stdout stream is parsed by `parse_stream_line`
into `AssistantEvent`s, published on the `EventBus` and forwarded to the
webview as `assistant:event`.

| Event | Meaning |
| --- | --- |
| `started` | the session announced itself |
| `message` | assistant prose |
| `tool` | an action run |
| `toolFailed` | an action that failed |
| `notice` | a Podium-originated note |
| `proposal` | a gated call parked for the user |
| `proposalResolved` | the user's Confirm/Reject was applied |
| `turnEnd` | the turn finished |
| `exited` | the session ended |
| `queueChanged` | the message queue snapshot changed |
| `queueNotice` | a queue note, e.g. a dropped or undelivered message |
| `queueItemDelivered` | a waiting message's turn came up |
| `responsesChanged` | the Responses list changed |
| `subagentsChanged` | the sub-agent list changed |

Unparseable lines (a chatty shell profile) are skipped, stderr is dropped,
and the binary + cwd are injectable (`AssistantConfig`) so integration tests
drive a fake `claude` script.

## Identity prompt

The assistant never edits code itself: `Edit`/`Write`/`NotebookEdit`/`Bash`
are refused at the CLI regardless of permission mode, and implementation work
goes out to a spawned agent (its `--append-system-prompt` identity says so).
That identity also leans hard towards **acting rather than checking in**. A
request that plainly implies dev work is spawned straight away instead of
proposed first, and a **to-do is only for work that runs long or is meant for
later**. A small request that can be done now goes into the agent's prompt
with no to-do at all. Asking is reserved for what it genuinely cannot tell;
everything else takes the sensible default and reports what it did.

It is also told to stay a **pure orchestrator** that delegates almost
everything. Anything beyond a reply from what it already knows (a lookup, a
check, shell/file work belonging to no single project) goes to
`spawn_subagent`, then it answers in one line and **ends its turn**. The
result comes back as a `[Podium]` message. Project code work still goes to
`spawn_agent`, routing (to-dos, messages, proposals) stays its own, and it
never uses the CLI's own `Agent` tool (`Agent` and its old name `Task` are in
its `--disallowedTools`, beside `Edit,Write,NotebookEdit,Bash`). See
[Assistant sub-agents](assistant-subagents.md).

It is told to keep agent updates to warnings and done-ness, how to stop
agents safely (only the project in question; check the output for follow-up
work and complete the to-do first), and how to pass on a push go-ahead. That
go-ahead is a message to an existing Claude Code agent, which accepts it once
`verify_sender` says the sender is the assistant, pushes, opens the PR/MR and
reports back with its URL. If the agent refuses, never reports a push, or is
not Claude Code, the go-ahead goes to a fresh `claude-code` agent (no
`--allowedTools`).

All of that is wording, not a gate — pinned by unit tests so a later prompt
edit cannot quietly drop it.

The notification policy the prompt spells out (what it may file as a
Responses item) is described in
[Assistant memory and proactivity](assistant-memory-and-proactivity.md#desktop-notifications).

## MCP endpoint

It reaches Podium over its **own MCP endpoint** (`/mcp-assistant`), wired in
via a 0600 `assistant.json` next to the per-agent configs — the whole tool
surface, workspace-wide. `--strict-mcp-config` is deliberately *not* passed,
so the user's own MCP servers (mail, Slack, …) stay available.

## Autonomy

`AutonomyMode` is stored in `agents.json` under `assistant`, default
`confirm-destructive`. It is enforced **server-side**, not by prompt wording:
`PodiumTools::call_tool` asks `assistant::requires_proposal` whether this tool
needs the user, and a gated call never runs. Instead it is parked as a
`ProposalInfo` — carrying a closure that replays that very dispatch later —
and the tool answers that a proposal is awaiting the user.

| Mode | Gates |
| --- | --- |
| `full` | nothing |
| `confirm-destructive` | the stopping/archiving/removing tools and `move_board_item` |
| `propose-all` | everything that is not read-only (an *unknown* tool counts as state-changing, so a tool added later cannot slip through) |

Proposals live in memory (never expiring, but lost on restart) and surface as
`assistant:event` `proposal` / `proposalResolved`. The user's Confirm/Reject
in the chat calls `assistant_resolve_proposal`, which runs the stored action
and tells the assistant the outcome as a `[Podium]` turn so it can carry on.

### Blocking SendMessage under propose-all

The gate only sees Podium's own MCP tools, so the CLI's own `SendMessage` —
the way a push go-ahead is relayed — is refused outright under `propose-all`
instead: that mode adds it to the session's `--disallowedTools`. A CLI flag
cannot change in a running session, so moving into or out of `propose-all`
(`Orchestrator::set_assistant_autonomy` →
`AssistantManager::set_send_message_blocked`) replaces the session, resuming
the conversation. That happens at once when it is idle (dropped, respawned by
the next turn), or at the running turn's `TurnEnd` — never mid-turn — via the
output task, which carries a `SpawnCtx` to start its own replacement before
the next waiting item goes out. If that spawn fails, the waiting items are
reported undelivered and `exited` is published, as for any exit nobody asked
for.

So the block applies from the next turn boundary: a turn already running when
`propose-all` is chosen can still `SendMessage` until it ends. The mode
loaded at startup is applied with `preset_send_message_blocked`. The prompt,
which never knows the mode, says: if `SendMessage` is unavailable, use the
fresh-agent route (a `spawn_agent`, which gets the card).

## Stream handling and the chat panel

A failed tool call is never silent: `tool_result` blocks flagged `is_error`
become a `toolFailed` event, stored under its own `TranscriptRole::ToolFailed`
(rows from before that replay as `Notice`). Lines carrying a string
`parent_tool_use_id` are the calls of a CLI-internal `Agent`-tool subagent
(not one of Podium's [sub-agents](assistant-subagents.md), which are
processes of their own) and yield no events — the parent's `Agent` row stands
for them.

A `result` with `is_error` reports its `result` text, not the `success`
subtype. A usage limit reads `<CLI text>: resumes at HH:MM` (the old `<text
with "limit">|<epoch seconds>` form, local time; `Fri HH:MM` when not today;
anything else stays verbatim) or the CLI's own wording. The CLI's synthetic
text block carrying that old form is dropped, the `result` line reporting it.
The store always prefixes it, so the panel shows `The turn failed: Claude AI
usage limit reached: resumes at HH:MM`.

The panel folds each run of consecutive `tool` / `toolFailed` / `nudge` rows
into one collapsed `<details>` line (render-only, the transcript is
unchanged). A `nudge` starts a new run unless the current one already holds
one, so a user turn's actions are never relabelled as Podium's, and a run
with a nudge reads fainter as "Podium check ·".

The panel only follows the tail while scrolled to the bottom (opening a run
keeps a pinned reader pinned, and sending scrolls down directly), else shows
a `New activity ↓` button. That button appears only when a row other than an
action/nudge was appended, and is cleared when the transcript shrinks.

## Persistence

One `assistant.json` in the app data dir (atomic writes, the same kind of
store as `todos.json`) holds Claude's own session id plus the chat rows the
panel replays — user turns, assistant prose, actions run and actions that
failed, capped at the most recent 500. On the next spawn that id comes back
as `--resume`, so the conversation continues *within* a run instead of
meeting a stranger. A session that dies without ever announcing itself makes
Podium drop the stored id, so a pruned session cannot wedge every later turn.

**An app launch, however, deliberately starts blank**: `src-tauri`'s `setup`
calls `assistant_new_conversation` before the webview can ask for the
transcript (`block_on`, not a spawned task — a task would race that call and
replay a conversation the panel is about to drop). A transcript from days ago
is therefore never dumped into a fresh panel, and any nudge backlog from an
earlier run goes with it. That makes `assistant-memory.md` the *only*
continuity between days, which is why the identity prompt now insists on
keeping it current.

The panel's **new-conversation** button (`assistant_new_conversation`) kills
the session and drops transcript, session id, pending proposals and the
message queue. That kill is deliberate, so it is not reported as `exited`
(any messages the queue was still holding are instead reported undelivered as
the first row of the now-fresh transcript).

## Message queue

Only one turn is ever in flight. `AssistantManager` pairs the session with an
`in_flight` flag and one `VecDeque<PendingInput>`, all under the *same* lock,
so a queued item can never slip past a turn that starts between two lock
acquisitions. The proactivity/`report_to_assistant` nudge drain (`nudge`) and
a proposal outcome (`notify`) always route through one `deliver_or_queue`:
idle with nothing already ahead of it → written immediately, exactly as its
caller already expects. Otherwise it waits, FIFO. The user's own turn
(`send`) goes through the same path, so a message typed mid-turn waits
instead of landing mid-answer.

### Send now

A waiting message can still be pushed into the running turn on demand —
`assistant_queue_send_now` / `send_pending_now`, the **send now** action on
the waiting row — which writes it straight into the live session, interleaved
output and all. That replaced a `queue_messages` setting deciding the same
thing for every message in advance: the choice only ever matters for one
message at a time, when you are looking at it. A nudge or a proposal-outcome
notice has no such escape hatch — their ordering is Podium's business.

### Draining

`TurnEnd` (`drain_next`) delivers exactly one waiting *user* item as its own
turn — never merged — and a `TurnEnd` carrying an error still advances the
queue. **Consecutive nudges are the exception**: they go out as one turn, and
a nudge whose text is already waiting is not queued a second time. Both exist
because Podium's own status lines used to arrive one per turn. The 6-hourly
sweep repeats itself verbatim, so a day without chatting stacked the same
line several times over, and the user's own next message then queued up
behind the whole backlog — they watched Podium narrate yesterday before it
answered today. Read together those lines are a summary of where things
stand, which is what they are for. User messages stay unmerged, since those
are separate thoughts.

### Editing, caps and interrupt

Only a *user* message is editable/removable (`assistant_queue_edit` /
`assistant_queue_remove`, `kind: "user"` only). A nudge or a proposal-outcome
notice is Podium's own text, delivered verbatim. The queue caps at the newest
20, dropping the oldest with a `queueNotice`.

`assistant_interrupt` kills the running child — there is no documented stdin
control message for interrupting `--input-format stream-json` mode — and
respawns it (picking the conversation back up via `--resume`), immediately
delivering whatever is waiting rather than for a `TurnEnd` that would never
come. With nothing running it is a no-op.

The queue is **not persisted**, matching `proposals`: a session ending for
any reason (crash, `new_conversation`, shutdown) drops it and reports which
messages were never delivered via `queueNotice`.

### Frontend sync

The frontend gets the whole snapshot on every change (`queueChanged`), plus
`queueItemDelivered` when a *waiting* message's turn comes up — the very
first, immediately-delivered message needs no such event, since the sender
already shows it itself. `assistant_send` (and `assistant_day_start`)
therefore **return whether the message was delivered rather than queued**,
and the panel adds its chat bubble only on `true`.

The panel cannot work that out on its own: a turn Podium started for itself —
the external-sources poll, a nudge — leaves a `nudge` row
(`PendingKind::Poll` / `Nudge`) but publishes nothing that starts or ends the
panel's `busy` state or announces the turn as the user's, so `busy` reads
idle while the session is busy. Guessing from `busy` meant a bubble for a
message the backend had in fact queued, and `queueChanged` then added a
waiting row for the same text: the user's message on screen twice, with the
poll's unrelated answer landing between them. For the same reason
`drain_next` publishes the shrunken `queueChanged` *before*
`queueItemDelivered`, so no render has the message in both lists.

In the panel the busy row and the waiting rows sit together in one **sticky**
block pinned to the bottom of the transcript (`.tail`), waiting rows *below*
the "thinking" indicator: what is about to happen should not scroll out of
sight while you read back. It renders only when something is running or
waiting, so an idle panel looks exactly as it did.

## Attachments

Attachments ride along with a turn (`assistant_send`'s `attachments`,
`podium_core::Attachment`). They come in three forms:

- An **image** always travels as its own inline `{"type":"image","source":
  {"type":"base64",…}}` content block. The CLI's stdin schema for
  `--input-format stream-json` is not documented, so this was verified
  empirically against the real CLI (an inline PNG it described correctly),
  not assumed.
- Anything else is handed over as an **absolute path** for the assistant's
  own `Read` tool (its cwd is the app data dir, so a relative path is
  refused).
- Something that only exists on the clipboard travels as **inline text**.

### Resolving and limits

`resolve_attachments` reads an image-looking path from disk and promotes it
to an inline image. It refuses individually (a `queueNotice` naming the file,
never its content) anything past 5 attachments per message, an image over
5 MB, inline text over 100 kB, or a relative path — the rest of the message
still goes.

### Image compression

An accepted image is then **shrunk** (`compress_image`) to at most 1568px on
its long edge — the size Claude's vision pipeline downscales to anyway, so
the extra pixels only cost upload and memory. It runs on what the cap already
accepted, so the 5 MB limit still judges what *arrived*. A JPEG stays a JPEG
(re-encoding a photo as PNG would grow it), anything else becomes PNG, and an
image already within the limit travels byte-for-byte.

**A GIF is never re-encoded** — this crate decodes a single frame, so
re-encoding an animated one would silently drop the animation; a GIF is only
ever subject to the plain size cap. An image that fails to decode is sent
as-is rather than refused: the CLI may understand a format the `image` crate
does not, and losing the user's attachment to our own decoder is the worse
failure.

### Turn content, queueing and the composer

`turn_content` folds path/text attachments into the turn's own text block,
and a turn with no attachments serialises byte-for-byte as it did before they
existed. Attachments survive queueing: `PendingInput` carries them, and
`QueuedMessage` reports only their names (`AttachmentInfo`), never the bytes
the frontend already holds.

The composer takes them three ways — drop on the panel, paste into the
composer, or the paperclip picker (`@tauri-apps/plugin-dialog`) — and shows a
removable chip per attachment. Drop/paste give bytes but no path, so a binary
file arriving that way is reported by name rather than vanishing. The picker
gives a path, which is why a large PDF should go through it.

## Transcript ordering

A **user turn's transcript row is appended before the turn is written** to
the child's stdin (`record_before_write`). The stdout task records the reply
on its own task, so appending afterwards let a fast reply land *above* the
message it answers, and a restart replayed the conversation out of order. A
nudge's or sweep's `nudge` row is recorded and published there too, since the
panel groups a turn's tool rows by the nudge row ahead of them. It is
idempotent, since a failed write re-queues the same item.

## Responses

Responses (`responses.rs`) are the short "needs you" / "FYI" points the
assistant files via the `add_response` MCP tool, only for what the user
actually needs (the identity prompt's rules, pinned by unit tests — see
[Desktop notifications come from the Responses list
only](assistant-memory-and-proactivity.md#desktop-notifications)). Podium itself files one per
parked proposal. The detail stays in its chat reply.

### Storage and context

One workspace-wide list, newest first, persisted in
`assistant-responses.json` (atomic writes, like `todos.json`) so items
survive a restart *and* the blank conversation every launch starts with —
they are the user's to clear. Agent, project and to-do names (and the agent's
colour) are resolved when the item is added, so an item about an exited agent
still reads right. A live agent's own project wins over a mismatched
`project_id`, and a context that is already gone (agent exited, project
closed, to-do removed) is dropped from the item rather than refusing it.
Agent and project names are capped at 40 chars, the to-do at 60, the source
at 40.

### Superseding

A newer item with the same `processId` or `todoId` replaces the open one,
with these exceptions:

- An FYI never replaces an open "needs you" (both stay).
- An item about an agent is only replaced by another agent's item (the same
  agent, or one on the same to-do, as after `respawn_agent`), never by an
  agent-less one such as the sweep's.
- An item with neither key never replaces anything.

The nudges Podium sends name the agent's process id (and the lingering-to-do
sweep each to-do's and project's id) so the assistant can pass them as the
supersede key.

### The add_response tool

`add_response` is disabled in the agent route's tool router, so it is neither
listed nor runnable there (the list is the assistant's to curate). That is a
guard against a confused agent, not a security boundary: both routes share
one bearer token, so an agent that deliberately posts to `/mcp-assistant`
gets through, exactly as it could reach every other assistant-route tool. The
OS user stays the trust boundary.

It is classified read-only, so the gate never parks it as a proposal, and
capped (newest 100 items, 500-char text) since the assistant writes it from
content it read. `add_response` refuses a `process_id` that names a live
terminal or service rather than an agent.

### Snapshots

Every change publishes a full `responsesChanged` snapshot, never recorded in
the transcript. Nor is the `add_response` call itself: `parse_stream_line`
drops the exact `mcp__podium__add_response` tool event, so neither the
transcript nor the chat shows an action row for it (the list already shows
its result). Snapshots are published under the store's lock, so they arrive
in the order the changes happened, and a write that fails leaves the list
(and the published snapshot) unchanged.

### Dismissing and replying

The user either ticks an item (`assistant_response_dismiss`, it is just
removed) or replies (`assistant_response_reply`). The reply is an ordinary
user turn through `send`'s path (`AssistantManager::send_reply`, so it queues
like any message) carrying the answered `ResponseItem` itself. The session
sees it as `[Reply to your Responses item — "<label>": "<item text>"]` + the
reply (label and text JSON-quoted, since an agent name or to-do text is
agent-controlled and must not be able to close the bracket). The transcript
stores it as a `reply` row (`"<label>\n<reply>"`), and a waiting reply
exposes `replyTo` (the label) on `QueuedMessage` and on
`queueItemDelivered`. Editing a waiting reply changes only its text.

The item is taken off the list before the send (so a double submit cannot
send twice) and put back if the send fails — or if a queued reply never goes
out: dropped by the queue cap, withdrawn by the user
(`assistant_queue_remove`), or lost with the session (new conversation or
crash, both also named in the undelivered notice; at app shutdown
`AssistantManager::shutdown` restores them itself before killing the child,
since nothing after it is sure to run). That is why the `ResponseStore` lives
on the `AssistantStore`: the queue code needs it. `restore` skips an item a
*newer* one has since replaced (an older item beside it never counts) and
keeps the 100-item cap by dropping the oldest other item, never the restored
one.

### Labels and the panel

The label is the non-empty parts of agent · project · to-do · source, or
`Responses item`. It is computed in Rust (`ResponseItem::label`) and added as
`label` only at the wire boundary (`LabelledResponse`, used by
`assistant_responses_list`, the `responsesChanged` event and `add_response`'s
answer) — never written to `assistant-responses.json` — so the frontend never
re-derives it. (The row shows the same parts with the agent styled apart,
dropping blank ones as `label()` does; its `nowrap` collapses whitespace
inside a part.)

The panel's `loadResponses` drops its result when a snapshot landed while it
was in flight. A half-typed reply is filed under its item's id. When that
item is replaced, the draft moves to the newest item that replaced it (the
backend's rule mirrored in `supersedes`), so two items side by side never
share one. The reply box takes focus only when its Reply button is clicked.
