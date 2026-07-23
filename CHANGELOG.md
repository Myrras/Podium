# Changelog

All notable **functional** (user-facing) changes to Podium are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. (Automated
changelog generation from Conventional Commits via git-cliff is planned but not
yet set up.)

## [1.2.2] - 2026-07-23

### Added

- **Assigned to-dos are colour-coded by session.** Each agent session gets a
  subtle colour when it starts, and every to-do it owns is tinted to match — so
  you can see at a glance which session is on what. An assigned to-do also hides
  its "start agent" button and drops out of multi-select (one session owns it),
  and an agent trying to claim a to-do already owned by another live session is
  now refused over MCP.
- **Copy-id button on to-dos and scratchpads.** The to-do and scratchpad detail
  panes now have a button that copies the item's id to the clipboard — handy for
  referencing it in a prompt or an MCP call.
- **Delete archived scratchpads.** Archived scratchpads can now be permanently
  deleted from the Archive view, the same way archived to-dos already could.

### Fixed

- **Grouped to-dos no longer borrow a random to-do's name for the session.**
  Starting one agent on several selected to-dos left the session unnamed so the
  agent names itself after reading them all, instead of taking whichever to-do
  happened to be first.
- **Selecting a to-do or scratchpad now highlights it.** Opening a to-do or
  scratchpad in the work area highlights its sidebar row, matching how a focused
  agent or terminal is shown.

## [1.2.1] - 2026-07-23

### Changed

- **Agents no longer show start/stop/restart controls** — in the sidebar row
  and the terminal pane header. An agent session is a one-shot conversation, not
  a restartable service; services and terminals keep their controls.
- **To-do titles are editable** in the detail pane header — via the edit button
  or a double-click (Enter commits, Escape cancels).
- **To-do descriptions autosave.** The Save button is gone; edits save after a
  typing pause and flush on blur. External (agent) edits still sync in and no
  longer clobber what you're typing.

### Fixed

- **The "needs input" notification no longer fires repeatedly.** Switching away
  from an agent whose prompt you'd already seen used to ping every time; it now
  fires once per prompt — viewing the agent acknowledges it, and only a new
  prompt pings again.
- **Adapter availability is probed once per app start** instead of every 60s, so
  the slow per-adapter login-shell check no longer recurs during a session. A
  newly installed CLI is picked up on the next launch.

## [1.2.0] - 2026-07-15

### Added

- **Scratchpads: shared, freeform notes per project.** Each project now has a
  Scratchpads section (below Terminals) with a WYSIWYG markdown editor
  (headings, lists, checklists, tables, code blocks, links), a live "On this
  page" table of contents, free-text tags, and archive/restore. Scratchpads are
  shared with agents over MCP — an agent can read and write them just like you
  can — and you can spawn an agent directly on one.
- **Windows support.** Podium now builds and runs on Windows: the PTY layer
  runs on ConPTY, terminals default to PowerShell, and the build workflow ships
  an NSIS installer alongside the macOS `.dmg`.
- **Agents name their own sessions.** A new `rename_session` MCP tool lets a
  running agent rename its session to reflect what it's working on, and a
  generically named agent is now told (in-context) to do so right after your
  first message — so sessions in the sidebar become recognisable on their own.

### Fixed

- **Spawned processes now see PATH edits from `.zshrc`/`.bashrc`.** Services,
  terminals, and agents run via a login *and* interactive shell (`$SHELL
  -lic`), so tools set up in interactive-only rc files (nvm, Homebrew shellenv,
  Docker/Colima/OrbStack) resolve — an agent no longer fails to find `docker`
  when a normal terminal tab finds it fine.
- **Agent prompts with apostrophes are no longer truncated on Windows.**
  Spawning an agent from a prompt containing a `'` (e.g. a to-do starting
  "You're…") used to cut the prompt off at the quote; agent command lines now
  use OS-correct argument quoting.
- **Restoring a project no longer duplicates it.** A race at startup (two
  concurrent restores of the same folder) could mint two entries for one
  project; opening a folder is now atomic. A workspace entry whose folder is
  permanently gone can be cleared via a "Remove from workspace" action on its
  error toast.
- **Restoring a done to-do from the archive keeps it active** instead of being
  swept straight back into the archive.
- **Agent terminal height no longer drifts** after tab switches or resizes —
  full-screen TUIs like Claude Code stay stable, with no flicker on an
  unchanged panel.

## [1.1.4] - 2026-07-15

### Changed

- **To-dos an agent is working on are now clickable in the sidebar.** Clicking a
  to-do listed under an agent's row opens it in the work area (matching the
  already-clickable scratchpad titles), so you can jump straight to a to-do an
  agent has self-assigned.

## [1.1.3] - 2026-07-14

### Added

- **Standalone agents are now named after their prompt.** An agent spawned
  without an explicit name (and not tied to a to-do) takes a short label from
  the first line of its launch prompt instead of the generic adapter name, so
  sessions are recognisable in the sidebar right away. The agent can still
  rename itself later.

### Fixed

- **Agent terminals no longer clip their bottom line.** The grid now sheds any
  rows that don't actually fit the panel (a fractional cell height made the
  fit over-count and push the last line under the edge), and the terminal keeps
  a deeper bottom gutter — so Claude Code's status line stays fully visible.
- **The "agent needs input" notification pings once, not repeatedly.** A
  waiting agent alerts a single time while you're not looking at it, and only
  re-arms once you view that agent again — a live prompt's redraw no longer
  fires a fresh ping every couple of seconds, and the window you're already
  watching never pings.

## [1.1.2] - 2026-07-14

### Fixed

- **Agent terminals no longer clip the last column on the right.** Text now
  keeps a right-hand gutter matching the left, so the final characters of each
  line stay fully visible instead of running under the panel edge (or behind a
  macOS overlay scrollbar).

## [1.1.1] - 2026-07-14

### Added

- **Auggie can now be registered as an MCP client in one click.** Settings →
  MCP shows an Auggie card alongside Claude Code, so agents running in Auggie
  can use Podium's MCP tools (list processes, read output, spawn sibling
  agents). Press Run — or copy the shown `auggie mcp add …` command — to
  register Podium; the card reflects whether Auggie's CLI is installed and
  whether Podium is already registered.

## [1.1.0] - 2026-07-13

### Added

- **Notifications now play a sound.** When an agent needs your input, Podium
  plays a sound alongside the OS notification and toast — a built-in beep by
  default, or a custom audio file you pick. A new Settings → General →
  Notifications section adds a Play sound toggle, a Choose…/Reset custom-sound
  picker, and a Test button.

## [1.0.0] - 2026-07-07

### Added

- **Open a project and run its dev servers, terminals, and AI agents in one
  cockpit.** Point Podium at a project folder and it reads `podium.yml`,
  starting each configured service, ad-hoc terminal, and coding agent in its
  own PTY with a real terminal view, lifecycle supervision, and a sidebar to
  switch between them. Opened projects persist as a reorderable, renamable
  workspace and reopen on the next launch.
- **Real terminals that survive tab switches.** Each process gets an xterm.js
  view with full scrollback that is retained when you move between tabs, plus
  input, resize, and a live "working…"/idle activity indicator for agents.
- **Supervised processes.** Services can auto-start and auto-restart
  (`never` / `on-crash` / `always`) with exponential backoff and a circuit
  breaker; a manual stop is never treated as a crash and kills the whole
  process group so children can't outlive it.
- **AI coding agents, including agents that spawn agents.** Launch Claude Code
  or Auggie from the new-agent modal (up to 8 per project). A built-in MCP
  server lets agents inspect and orchestrate the processes Podium manages —
  listing/starting/stopping them, reading output, and spawning more agents.
- **Shared per-project to-dos.** Each project has a to-do list visible in the
  sidebar and to every agent over MCP, with a detail view for descriptions and
  a comment thread. To-dos can carry pinned issue/PR links, be assigned to a
  running agent, and be archived (done items auto-archive from earlier days).
- **Connect external MCP clients once.** A stdio bridge lets tools like Claude
  Code talk to Podium's MCP server through a stable config even though the
  port and token rotate every launch; the Settings → MCP tab offers one-click
  install and status.
- **Dark/light themes, settings, and toasts**, with a motion system that
  respects reduced-motion preferences.
