# Assistant memory and proactivity

  **Memory** is a separate `assistant-memory.md` in the same dir: free-form
  markdown appended to the identity prompt at spawn under `# Your memory`, so
  it survives new conversations. The assistant maintains it itself through the
  `update_assistant_memory` MCP tool (full replace — an empty string clears
  it), and the user can read, edit or clear it in the panel's memory view. It
  is read fresh from disk on every spawn, so hand-editing the file works; a
  change reaches a *running* session only on its next spawn.

  **Proactivity** (`proactivity.rs`, setting `proactive` in the same
  `assistant` block, default **on**) lets Podium speak first. One task watches
  the `EventBus` and, on a slow tick, the process/to-do state: an agent that
  exited (finished or crashed), an agent the UI reported an input prompt on
  (`process_report_waiting` → `Orchestrator::report_agent_waiting` /
  `agents_awaiting_input`), a running agent that has printed nothing for
  5 minutes (so it is probably waiting on a question — `Orchestrator::idle_agents`,
  measured from the scrollback's last append, or from the process start when it
  never printed), and a 6-hourly sweep for to-dos open longer than a week.
  The prompt report and the silence timer share one dedupe record, and a
  reported prompt wins the wording, so a blocked agent is announced once —
  at the next tick rather than five minutes later. The silence timer stays
  as the backstop for agents whose terminal has never been opened, where
  there is no screen to read.

  That record then **escalates once, and stops**: an agent still blocked
  `escalate_after` later (default 10 minutes) is raised a second time, with
  wording that asks the assistant to say which answer it would pick and why —
  something the user can wave through in a word — and, once that answer is
  agreed, to call `respawn_agent` (nothing can type into an agent's
  terminal). That is the cap, `MAX_RAISES = 2` per question: raising it every
  `escalate_after` for as long as the agent lived once produced a "no change"
  reply every ten minutes for hours. The record is kept **for as long as the
  agent lives**, not only while it looks stuck: an agent that prints one line
  and falls silent again is waiting on the same unanswered question, and
  re-reporting it from scratch is exactly the repetition this prevents. A
  **new question cycle** starts when the user types into the agent's terminal
  after the last raise: `Orchestrator::write_stdin` stamps
  `ManagedProcess.last_input` (`last_user_input`), and a tick that finds it
  newer than `Waiting.last` drops the record, so the next stall is raised
  from scratch. A write counts as the user typing unless it is a short
  escape sequence (at most 16 bytes starting with ESC — xterm.js's own focus
  reports and cursor/device-attribute replies, which would otherwise end the
  question on every window focus change); a lone arrow key is skipped too.
  Otherwise only the agent exiting (or vanishing) clears the record.

  **Viewing** is what keeps the assistant quiet about the agent the user is
  looking at. The frontend's activity monitor reports it (`process_report_viewing`
  → `Orchestrator::set_viewing`, in memory only, at most one process, never a
  non-agent; the flag lives on `ManagedProcess.viewing`, so a removed process
  drops it by itself) and `ProcessInfo.viewing` carries it to
  `list_processes`/`get_process_status`. Proactivity skips the viewed agent:
  no finished/crashed nudge, and no idle/waiting nudge — its question is
  recorded as already raised instead (`Watcher::mark_seen`: one raise toward
  the cap, escalation clock restarted every tick it stays in view), so
  leaving the agent never fires a nudge at once. `add_response` for a
  `process_id` that is the viewed agent stores nothing and answers with a
  short "skipped, the user is viewing that agent" text (not an error;
  `Orchestrator::add_assistant_response` returns `Option`). A
  `report_to_assistant` from the viewed agent is still delivered, with one
  line appended telling the assistant not to mention it or file an item. The
  identity prompt says the same of `viewing: true`; reading the agent's
  output stays fine.

  Each raise becomes one `[Podium] …` turn plus a `notice` event, so the chat says why the assistant is speaking
  unprompted. Timings are injectable (`ProactivityConfig`) so tests run in
  milliseconds. A nudge **starts the session when there is none**
  (`ensure_session`, the same path `send` uses) — otherwise the user only
  learned an agent had finished whenever they next happened to type, which is
  how a weekend's worth of `[Podium]` lines came to arrive in one pile. This
  reverses the original phase-4 rule; the reasoning behind that rule ("a fresh
  session would not know what the nudge is about") no longer holds now that
  every launch starts blank and `assistant-memory.md` carries the context. A
  spawn that fails falls back to queuing, so a nudge is never lost. Switching
  the setting off silences all of it.

  **Unblocking a stuck agent** is `respawn_agent`
  (`Orchestrator::respawn_agent`): it stops the agent and spawns a fresh one
  carrying the answer in its prompt. Nothing can type into a running agent's
  terminal, so a session blocked on a question is otherwise dead weight, and
  a plain `stop_process` + `spawn_agent` pair would lose the one thing the
  caller cannot reassemble — the **to-dos the old session owned**, which are
  what carry the task over. Those (and the adapter) are inherited; the
  replacement is deliberately *not* given the old name, so the two sessions
  are told apart in the sidebar. An agent that owns no to-do has nothing to
  inherit, so `task` must be supplied as well or the call is refused rather
  than burning a session on an answer to a question nobody asked it. It is
  classified **destructive**, so `confirm-destructive` (the default) shows
  the user a card first.

  Because these are event-driven — Podium already sees an exit or a silent
  agent for free — **a quiet day costs no turns at all**.

  Alongside that runs a slow lane: the **external sweep**, every
  `poll_interval_minutes` (default 10; `0` is off), asking the assistant to
  check the enabled sources for anything needing the user. Slack and Teams
  need no integration — the assistant already inherits the user's own
  user-scope MCP servers — so this is `poll_prompt` wording plus a timer, and
  it is skipped entirely with both sources off or proactivity off. It is
  delivered as `PendingKind::Poll`: unlike a proposal-outcome `Notice` it
  leaves the same `Notice` row a nudge does (`record_before_write`, stored as a
  `nudge` row), so the panel folds the sweep and its actions into one faint
  "Podium check" line rather than mixing them with the user's own. The old
  concern, a row every 10 minutes piling up, is gone because the panel folds
  consecutive Podium rows into a single line. Unlike nudges, polls are not
  merged or deduped in the queue.

  **The silence contract**: a sweep that found nothing must say nothing. The
  prompt instructs the assistant to answer with exactly `NOTHING_TO_REPORT`,
  and a `Message` carrying that sentinel **on a line of its own** — anywhere
  in the reply — is dropped before it is recorded or published: no chat row,
  no transcript row. It is deliberately not an exact-whole-message match:
  models keep prefixing the word with the very summary the prompt forbids
  ("Same messages as last check.\n\nNOTHING_TO_REPORT"), and matching exactly
  let all of that through as a notification — the noise this contract exists
  to prevent. A reply that quotes the sentinel *mid-sentence* while reporting
  something real is still a real reply.

  **Desktop notifications come from the Responses list only.** A chat
  reply, a proposal card and a `[Podium]` nudge never ping; a **new
  Responses item** does (see _Terminal & activity_ for the frontend half).
  So what the assistant may file *is* the notification policy, and the
  identity prompt spells it out. `needsYou` only for an agent blocked on a
  question only the user can answer (one item per question, replaced when
  the question changes) and an external message (Slack/Teams/mail) that
  directly asks the user something — never a manual step the user already
  knows about (merging a PR, `just install-signed`, testing a build).
  `fyi` only for finished work the user asked for that has something to look
  at (a result, a PR/MR URL, a posted review, the answer), an agent that
  crashed or failed, an external heads-up that asks nothing, and an agent
  the assistant stopped or respawned on its own. Never: "no change"/"still
  waiting" reminders or a repeat of an open item, a routine check that found
  nothing, lookup or sub-agent results it used itself, an agent that
  finished with nothing left for the user, or anything about the viewed
  agent. A nudge that brings nothing new about an agent already reported
  gets no new chat reply either. Unit tests pin the key phrases.
  The old `ACTION_REQUIRED` marker and `TurnEnd.notify` (with the
  `turn_from_user` bookkeeping that fed it) are gone. `take_action_required`
  stays, only to strip a stray marker an assistant writes out of habit
  before anything is recorded or published; a reply that is *only* the
  marker is dropped like a swallowed one.

  **Proposals file their own item.** When the autonomy gate parks a proposal,
  `AssistantManager::propose` adds a `needsYou` Responses item — "Confirm or
  reject: `<tool> <summary>`", `source` `"proposal"`, no process/to-do — and
  keeps its id on the `PendingProposal`. Resolving (`resolve_proposal`),
  invalidating (`invalidate_proposals_for_project`) or dropping it
  (`new_conversation`) takes the item off again. Proposals are memory-only
  but the list is on disk, so `new_conversation` (every launch runs one)
  also sweeps every item whose source is `proposal`, which is how one a
  restart orphaned goes away. The assistant must not file one itself.

  The **day-start summary** used to be a fourth trigger here and is now the
  panel's **"Start my day" button** (`assistant_day_start`, shown in the
  empty state). A nudge never spawns the CLI, so as a nudge it only surfaced
  whenever the user next happened to type — which is exactly the complaint
  that moved it. The button sends `DAY_START_PROMPT` through the ordinary
  `assistant_send` path instead: it spawns the session, shows as a user turn
  and respects the queue setting. It asks for open work across all projects
  *prioritised with a reason*, and to ask the user rather than guess when the
  to-dos and agents do not settle the order. There is deliberately no
  once-a-day lock — pressing it is explicit, so refusing would only annoy.
  The watcher holds
  only a weak reference, so it ends with the orchestrator. **Settings →
  Assistant** holds this toggle plus the autonomy mode (the chat header keeps
  its own autonomy picker — both mirror the one stored value).

  A pending proposal is also dropped when **its project closes**
  (`invalidate_proposals_for_project`, keyed off the call's `project_id`
  argument): confirming it later could only fail, so the card resolves itself
  with the reason and the assistant is told.
