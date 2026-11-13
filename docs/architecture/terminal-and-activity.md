# Terminal and activity (frontend)

How the frontend shows terminals and agent activity: the xterm.js registry in
`src/lib/terminalRegistry.ts`, the activity monitor in `agentActivityStore`,
the sub-agent list, assistant notifications, the assistant composer and the
window chrome.

## Terminal registry

xterm.js instances live **outside React** in `src/lib/terminalRegistry.ts`
(keyed by processId, reparentable host div). This survives StrictMode
double-mounts and keeps scrollback across tab switches. The registry owns
attach/re-attach (including the `lagged` recovery) and re-themes live
terminals on theme change.

## Activity

`agentActivityStore`'s monitor derives each running process's state once a
second. The core reports what a process says about itself; the store falls
back to guessing when it says nothing.

### Reported state (core)

`podium-core/src/activity/` gathers one `Reported` per running process, and
`process_activity` sends it merged into `OutputActivity` (`state`,
`waitingFor`, `stateMs`, `subagents`, `shells`, next to `quietMs`/`burstMs`).
`state` is `working`, `waiting` or `idle`, or null. Providers:

| Provider                | Covers                       | Source                                                                                                                         |
| ----------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `claude_status`         | Claude Code agents, native   | Claude Code's own `<claude config dir>/sessions/<pid>.json` (`status`: busy, shell, waiting, idle), found by the `podium-<tag>` session name. Undocumented Claude Code internals |
| `hooks`                 | sub-agent and shell counts   | event files from Podium's hooks, drained on each poll (see [Agent hooks](processes-and-agents.md#agent-hooks))                |
| `foreground` (Unix)     | terminals, default shell only | working while the PTY's foreground process group is not the shell; none for a terminal started with its own command           |

`stateMs` is time in the current state: Claude Code's own timestamp when it
records one, else since the poll that first saw the state. Windows, WSL and
Auggie agents report nothing.

### Fallback (store)

With `state` null the store keeps the old heuristics: working when the PTY
printed within 2.5 s and the burst lasted at least 300 ms (a lone redraw is
~0 ms, a spinner keeps growing), and **needs input** when a quiet screen
shows a permission prompt or credential request (`detectInputPrompt`, needs an
opened terminal). A reported state wins over both; a reported `waiting` is the
needs-input episode. Exception for non-agent processes: the core can only
report `working` or `idle` for a terminal, so a reported `working` with an
input prompt on screen (`sudo`, `[y/N]`) reads `waiting` and counts as a
prompt episode (one notification). A reported `idle` stays idle. Background shells fall back to the screen read
(`detectBackgroundShells`) when the core counts none.

### Store contract

`ProcessActivity` (`activity`, `reported`, `waitingFor`, `sinceMs`,
`subagents`, `shells`) is kept per process in `details`.
`useProcessActivity(id)` reads one slot, so a component re-renders only when
its process changes; an untracked process reads `UNTRACKED_ACTIVITY` (idle,
not reported). `useAgentActivity` is the narrow read. Sidebar and worktree headers select
only their working/needs counts (`activityRollup` through zustand's
`useShallow`), so a header re-renders only when its counts change.

### Status pill

`pillState` (`src/lib/processStatus.ts`) maps lifecycle plus activity to one
pill, shown on each sidebar row and in the terminal pane header, never
hidden: Working (Running for a terminal or service), Needs you, Idle, Up,
Stopping, Crashed, Exited, Not started. A running terminal or service reads Up
until it reports a state. The pill carries the elapsed time and counters for
running sub-agents and background shells; on row hover the buttons replace the
elapsed time. Collapsed projects and sections roll up counts of working and
needs-you (`activityRollup.ts`).

Rows: selection is a raised block without a hue. An agent row shows an
identity square in its session colour instead of a tinted row; a to-do shows
its owner's square, hollow when that agent has stopped.

### Sub-agents in the monitor

The monitor also covers the assistant's sub-agents, which are never in
`processStore`. `agentActivityStore` adds the running ones from
`assistantStore.subagents` (through `subagentProcess`) to its input, so one
stuck on a prompt is reported at once, **once its terminal has been opened**
(the prompt is read off the xterm viewport). A sub-agent whose terminal was
never opened is caught by the backend's 5-minute silence timer instead. Only
its folder-trust dialog the backend spots on its own (see
[Assistant sub-agents](assistant-subagents.md#fallback-detection)).

## Sub-agents (frontend)

### List

`SubagentsSection` (rendered by `AssistantPane` above the chat log, hidden
when empty; collapsible) lists `assistantStore.subagents`. It is loaded in
`App.tsx` at startup (the activity monitor and quit warning need it with the
panel closed), and kept current by `subagentsChanged` snapshots plus
`applySubagentStatus` from `process:status` in between. The store also drops
an ended row at its `expiresAt` by itself (`pruneExpiredSubagents`, one timer
re-armed on every list change), and `App.tsx` re-pulls the list on window
focus / `visibilitychange`, so the panel never waits on a snapshot a hidden
webview was slow to run.

Per row:

- name/task, kind, a live elapsed clock and phase (the result or error is only
  the phase label's tooltip, never a line of the row)
- stop (`assistant_subagent_stop`, its click does not propagate) while
  queued/running

The row's name/task area itself opens a started one (`role="button"`,
Enter/Space, a focus outline; a queued one is not clickable). There is no
separate open button.

A `waiting` row reads "Needs input" (warning colour, the question as the phase
tooltip), counts in the header summary (`N needs input`) and stays live (stop,
open, ticking clock, `isSubagentLive`, `TerminalPane`'s stop, the quit
warning's "needs input" tag).

### Opening

Open = `layoutStore.openSubagent` (`openSubagentInWorkArea`). It fills the work
area with a `TerminalPane` on a synthesized process, exclusive with the
focused process, to-do, scratchpad and board (a sub-agent is never in the
process store, so it cannot use that focus). The pane shows an "Assistant
sub-agent · kind" label and a stop button, and its stop goes through the
sub-agent path. `CloseWarningModal` lists live sub-agents (queued marked)
beside processes.

### Reset

**Settings → Assistant**'s reset: with live sub-agents the button shows an
inline Keep running / Stop them / Cancel choice before calling
`assistant_new_conversation { stopSubagents }`. The choice is withdrawn when
the live count drops to 0 while it is open, and the next Reset just resets.

## Assistant notifications

`notifyAgentWaiting` (`src/lib/notify.ts`) has a sibling, `notifyAssistant`,
for the same toast+OS-notification+sound treatment on the personal assistant.
It fires from one place only: `assistantStore`'s handling of a
`responsesChanged` snapshot (`notifyNewResponses`). A chat reply, a turn end,
a `proposal` card and a nudge never pings. The Responses list is the only
thing that reaches the desktop (a proposal gets there through the `needsYou`
item the backend files for it).

A module-level `Set` holds the id of every item this app run has seen, filled
from every snapshot and from `loadResponses` (also called once from `App` at
launch, so a closed pane still has a baseline; nothing pings before that first
load completes). Only an id not in the set is news, so what the list held at
launch never pings and a reply that gets restored (same id) never pings twice.

The ping is skipped when `isAssistantVisible()` (focused window **and**
`useLayoutStore.getState().assistantOpen`), and per item when its `processId`
is `viewedAgentId()`. Several new items in one snapshot make one notification:
title "Needs you" when any is `needsYou`, else "FYI"; body the newest item's
text plus "(+N more)".

## Viewing

`agentActivityStore.ts`: `viewedAgentId()` is the one definition. It holds
when the window is focused, the active process is an agent, and no to-do,
scratchpad or Git board pane covers the work area (`isViewingAgent`, the
"needs input" gate, shares its base and so also treats an open scratchpad as
covering the terminal).

The once-a-second monitor calls `processReportViewing`
(→ `process_report_viewing`) only when the value changed, retries a failed
report on the next tick, and also reports on window `focus`/`blur`. Stopping
the monitor reports "none".

## Composer history

The composer (`AssistantPane.tsx`) supports shell-style history recall.
Arrow-up/down walk the sent `user` rows already in `messages` (no separate
persisted history — they come back for free via transcript `hydrate()`).

Recall only fires when the caret sits on the composer's first line (arrow-up)
or last line (arrow-down, only while already recalling). This is checked by
literal `\n`s around the caret (`isCaretOnFirstLine`/`isCaretOnLastLine`), not
visual wrapping, since a plain textarea gives no other way to ask. So a
multi-line draft still lets the arrows move the cursor normally.

Walking past the newest entry restores whatever was being typed before the
first arrow-up. Recalling never sends, and editing a recalled message before
sending appends a new entry rather than rewriting history.

## Selection context

**Selection context** (`src/lib/assistantContext.ts` +
`src/lib/selection.ts`) puts whatever the user has selected into the composer
as dismissible chips, sent along as ordinary `text` attachments. The backend
already folds those into the turn, so nothing in Rust knows about selections.
Two sources: the Cmd/Ctrl+click multi-select and selected text. User-picked attachments are sent before
context items, so the explicit one wins when both would overflow the backend's
five-per-message cap.

### Multi-select

The **Cmd/Ctrl+click multi-select** covers to-dos, scratchpads and now
process/agent rows, all of them, in pick order. Merely **opening** an item is
deliberately not context: that is a "show me this" gesture, and attaching it
would send context nobody asked for.

The multi-select lives in `selection.ts` rather than in each subsection's
`useState`, because two readers need it at once: the subsection's "start an
agent on these" bar and the composer's chips. A selected item whose row is
gone (project closed, to-do deleted) simply stops resolving and drops out, so
no chip ever names a bare id.

### Selected text

Text comes from the document's own `selectionchange` plus xterm's
`onSelectionChange` (a terminal renders to a canvas, so its selection never
reaches the DOM). A selection whose anchor — or the focused element — sits
inside the pane (`[data-assistant-pane]`) is ignored. That is the user reading
or editing the conversation, and clicking into the composer collapses the
document selection, which would otherwise discard the very text they are about
to ask about. `selection.ts` is store-only, importing nothing from `src/state`,
so the terminal registry can feed it without closing a cycle.

## Window chrome

`App.tsx`'s `<header>` is the title bar on Windows and macOS (`data-tauri-drag-region` makes the strip draggable). Windows turns the native decorations off (`"decorations": false`) and draws its own minimize/maximize/close in `WindowControls.tsx` at the top-right (`@tauri-apps/api/window`, polling `isMaximized()` on `onResized` to track Aero Snap). macOS keeps its decorations with `"titleBarStyle": "Overlay"` and `"hiddenTitle": true`, so the native traffic lights (and native fullscreen) are drawn over the strip's top-left corner; `.titlebarMac` leaves 78px of room for them and the brand is centered. macOS also keeps its native *menu bar* (⌘, opens Settings — see the `setup` hook's `#[cfg(target_os = "macos")]` block). Linux keeps its native title bar for now. `src-tauri/tauri.windows.conf.json` / `tauri.macos.conf.json` are platform overrides merged over `tauri.conf.json` as RFC 7396 JSON Merge Patch — a JSON *array* replaces wholesale, so each file repeats the whole `windows[0]` object. The window-state plugin is registered without `StateFlags::DECORATIONS`, otherwise its restore (which runs after `setup`) overrides the platform config's decorations.
