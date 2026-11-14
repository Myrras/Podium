# Floor mode

An alternative to the sidebar and work area: a map with a room per project and
a desk per agent. Frontend only; no Rust or IPC change. UI in
`src/components/floor/`, pure logic in `src/lib/floor/`.

## Mode and entry

`layoutStore.mode` is `"project" | "floor" | null`, persisted in `podium.layout`.
`null` = not chosen: `ModeChooser` (a `Modal`) shows and the app renders Project
mode. Closing it without a choice keeps `null` (asked again next start). The
choice is changed in Settings > General > Appearance > View (`setMode`).
`App` renders `Floor` inside `<main>` when `mode === "floor"`; every
subscription stays mounted in App. `Floor` calls `floorStore.reset()` on unmount.

## Stores

- `layoutStore` (persisted): `floorPositions` (`project.root` → `[x, y]`),
  `setFloorPosition`, `setFloorPositions` (several, one write),
  `resetFloorPositions`; `forgetProject(root)` drops the
  position too.
- `floorStore` (memory): `camera {x,y,z}`, `animate`, `viewport`, `focus`
  (`"all"` | project id | `null` after a manual pan or zoom), `popover`,
  `terminalId`, `hoverDesk`, `agentModal`, `scratchpadAgentModal`, `fitted`
  (FloorMap fitted all once) and `moved` (`setCamera` ran since the last
  reset). The first fit skips when `moved` is set; it checks the flag, not the
  camera object, because StrictMode's simulated unmount runs `reset()`.
  `setCamera` defaults to `focus: null, animate: false`; `openTerminal` closes
  the popover.
- The pinned desk is `processStore.activeProcessId`; the open modal is
  `floorStore.terminalId`.

## Library (`src/lib/floor/`)

- `types.ts`: `RoomModel`, `DeskState` (`working | needs | idle | ended | crashed`),
  `Camera`, `Rect`, `Size`, popover and modal request types.
- `geometry.ts`: pure numbers, no DOM: `roomSize` (desk rows of fixed `DESK_H`),
  `snap` and `hits` (40 px grid, one cell apart), `layoutRooms` (saved spots
  first; overlapping rooms are pushed right or down, the one that did not grow
  moves; spots are kept when a room shrinks), `bounds`, `frame`, `zoomAt`, `toScreen`; zoom
  0.3 to 2, far below 0.4.
- `rooms.ts`: `deskState`, `useRooms`/`getRooms` (never write), `roomOf`,
  `useFloorData` (loads to-dos, scratchpads and worktrees once per added project and
  enabled section, forgetting removed projects, and writes back changed room
  positions in one `setFloorPositions`; only `FloorMap` calls it).
  `getRooms` caches by input (projects, processes, positions, desk states), so
  the layout runs once per change for every caller; an unchanged room keeps its
  object so `memo(Room)` skips it. `useRooms` is `useSyncExternalStore` over it.
  `grown` tracks which rooms grew at the latest size change, for `layoutRooms`.
- `camera.ts`: `jumpTo("all" | projectId)` (animated), `zoomBy`, `showAgent`.
- `startAgent.ts`: `startAgentIn` for the nameplate +.
- `menus.ts`: right-click items (`processMenu`, `todoMenu`, `scratchpadMenu`) for
  `ContextMenu`; used by desks, wall notes, notebooks, rack lights and the to-do
  and scratchpad popover rows. Items that stop or remove something set `confirm`
  on `MenuItem` (`ContextMenu.tsx`) and ask inside the menu. `openProcess` (pin
  and open the modal) and `todoAgentAction` (Open the agent / Start agent) are
  the one way to do those; desks, Next and the to-do popover footer use them.
  The project menu shares worktree removal (`src/lib/worktrees.ts`) and the
  rename rule (`src/lib/projectName.ts`) with the sidebar.
- `terminalCommands.ts` (`useTerminalCommands`) and `terminals.ts` `runCommand`:
  justfile recipes and saved commands, run in a new terminal.
- `waypoints.ts`: edge markers for rooms that need you and are out of view.
- `sprites.ts` + `Bot.tsx`: pixel art as SVG data URLs, 8 bot heads; only a
  working bot moves. `botVariant.ts` gives each agent in a room its own head (`RoomModel.variants`, set in `buildRooms`). Idle
  and ended bots are grey; the desk adds a floating Zzz for idle only.

## Components

`Floor` → `FloorHeader`, then `GitBoard` (when `openBoard` is set) or `FloorMap`
+ `FloorWaypoints`, then `FloorStatusBar`, `FloorPopovers`, `FloorTerminalModal`,
`FloorAgentModals`. `FloorMap` → `Room` → `RoomNameplate`, `RoomWall`, `Desk`.

- The camera never goes through React state: `FloorMap` subscribes to the store
  and writes the world layer's `transform`, `--z` and `data-far` (z < 0.4;
  labels hide with `:global([data-far])`). Only transform and opacity animate;
  `--z` is registered with `@property` so it animates with the layer, and
  nameplates and desk marks counter-scale by `1 / --z`. `FloorWaypoints` does
  the same: React renders the set of markers, a store subscription writes their
  positions, and they transition only when `animate` is set.
- Pan starts from a pointerdown whose target is outside
  `button, [role="button"], a, input, textarea, select, [data-no-pan]`, after
  4 px. Every clickable thing on the map is a `<button>` or has `data-no-pan`.
- A room moves by dragging its nameplate (snapped to the grid, a colliding
  spot is dropped) or with the arrow keys on it. A drag start (past the click
  slop) closes the open popover. A cancelled drag
  (`pointercancel`) snaps back and saves nothing.
- Popovers anchor on `data-fixture="<projectId>:<todos|scratchpads|commands|services|menu>"`;
  `FloorPopovers` looks it up on each render and closes when it is gone, so
  fixtures stay mounted while open. One popover at a time. `Popover` places
  itself on open, on window resize, through a `ResizeObserver` on anchor and
  popover, and one frame after each move of its optional `follow`
  subscription; an animated move is followed with rAF for `MOTION.slow`
  (`--dur-4`), then the loop stops. `Popover` and `ContextMenu` know nothing
  of the floor: the fixture popovers, `Desk` and `RoomWall` pass
  `followCamera` (`lib/floor/follow.ts`, fires on each `floorStore` camera
  change and when a room moves or resizes, a push or re-layout, so the popover
  re-places to its fixture). Both clamp to the window with `clampToViewport` (`lib/viewport.ts`).
- A right-click menu closes on Esc, an outside mousedown or wheel, a window
  resize and each `follow` move (pan, zoom or jump), so it never stays at a
  stale spot. `useContextMenu(follow?)` defaults to the enclosing popover's
  `follow`, so the to-do and scratchpad rows in a popover close theirs too.
- A section hidden in `ProjectInfo.sections` hides its fixture (Commands follows
  `terminals`). Services show only when the project has any.
- To-do and scratchpad popovers fill the `Popover` header and footer through
  `PopoverSlot`. The Commands popover (`CommandsFixturePopover`) lists commands
  with a search field (arrow keys, Enter) and has Edit commands in the header.
- Stopped terminals: `Desk` greys the art and labels it "stopped". The terminal
  modal shows the state and Close terminal (after a confirmation); the nameplate's terminal icon calls
  `newTerminal`.

## Esc and terminal rules

- No `keydown` listener on `window` or `document`. Esc "show all" is an
  `onKeyDown` on the map viewport and one on the header root; both do nothing
  while a popover or the terminal modal is open.
- A popover closes on Esc only while focus is inside it (it takes focus on open
  and gives it back to the anchor).
- Close blurs first: `Popover` (Esc, ×, outside mousedown, a vanished anchor)
  and `FloorTerminalModal` (× and the overlay) blur a focused element inside
  them (`blurWithin`) before they close, so a field's own onBlur commit saves
  it. React delivers no blur for an element its commit removes, so fields keep
  no unmount commit of their own. An Esc a field uses to cancel its edit calls
  `preventDefault`, so it neither closes nor commits. Only real autosave
  flushes (to-do description, scratchpad content) still run on unmount.
- `FloorTerminalModal` is a portal overlay, not `Modal`: Esc must reach the
  terminal (Claude Code uses it) and the card must not scale (xterm mouse
  mapping). It closes with its button or a mousedown on the overlay. Esc in the
  rename field only cancels the edit. Tab is trapped in the card; a focused
  xterm sends Tab and Esc to the PTY.
- The modal has one header bar: `TerminalPane`'s, filled through its optional
  `leading`, `trailing`, `onRename` and `hideActions` props (the modal draws no
  bar of its own). Stop is an icon at the far left.
- Agent actions map to existing commands: Resume = `startProcess`, Stop
  = `stopProcess`. Stop (`stopIfRunning`, shared by the menus and the modal)
  re-reads the status when confirmed and stops only a running process. Close
  agent/terminal (`closeProcess`) reads the current process when it runs, calls
  `processStop` while it is active, keeps the process and its modal (toast)
  when the stop fails, and otherwise calls `removeProcess` and then hides the
  modal. × hides
  the modal and keeps the process running; `Next: <name>` opens the next
  waiting agent. Stop, Close terminal and Close agent ask first in a confirm
  dialog inside the card (Esc only cancels it).

## Not built

Multi-select as assistant context, Open in Warp,
section toggles in Floor mode, a Stopping light, keyboard travel between rooms,
answer buttons in the modal.
