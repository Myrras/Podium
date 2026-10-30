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

Every `process_resize` is also remembered on the process in core
(`ManagedProcess.term_size`, even while it is not running), and `do_start`
opens the next PTY at that size instead of 80x24: a terminal is only refitted
when its host changes size, so a process (re)started under an open terminal
otherwise stayed 80 columns wide. On Windows each terminal gets xterm's
`windowsPty` hint (`windowsPtyOption`): ConPTY repaints its own screen on
resize, and xterm reflowing on top of that scrambled output. The build number
(Windows 11 → reflow on) comes from the WebView's async UA client hints and is
applied to live terminals when it arrives.

## Activity indicator

The **activity indicator** is derived once a second by `agentActivityStore`'s
monitor. It has two states, "working…" and "needs input".

### Working

"Working…" (pulsing dot) comes from the backend: `process_activity` reports,
per running process, how long ago it last printed and how long that burst of
output has lasted, read off the core's scrollback
(`ScrollbackBuffer::output_activity`, bursts split by a 1s gap). So it covers
agents whose terminal was never opened here.

A process reads as working when it printed within the last 2.5s **and** the
burst has lasted ≥300ms: a lone redraw (Claude Code repainting after the pane
is fitted, a prompt being drawn) is a burst of ~0ms, while a spinner keeps one
growing. The previous signal was the frontend's own stream timestamp, which
did not exist for an unopened terminal and which the attach snapshot and the
resize repaint both bumped. So every open showed "working…" for a few
seconds.

### Needs input

A quiet screen showing a permission/confirmation prompt or a credential
request (`sudo` password, ssh passphrase) reads as **"needs input"** instead.
That half still reads the xterm viewport, so it needs an opened terminal. It
is derived for **every** running process, not just agents, so a terminal
stalled on `sudo` surfaces the same way. Non-agents only show that state
("working…"/"idle" is noise for a plain shell).

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
