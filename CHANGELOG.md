# Changelog

All notable **functional** (user-facing) changes to Podium are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Internal refactors, tooling, and chores are intentionally omitted. Every pull
request adds its entry under `## [Unreleased]`; the version number is assigned
when it merges.

## [Unreleased]

## [2.4.0] - 2026-10-08

### Changed

- **Restarting a stopped agent picks up its own conversation.** A Claude Code
  agent you stop and start again resumes the session it was in, instead of
  starting over with its original prompt. If Claude Code no longer has that
  conversation, or resuming it fails straight away, the agent starts a fresh
  one rather than failing to start.
- **A stopped agent gives up its session colour.** Its row and its to-dos turn
  neutral grey while it is stopped. Restarting it gives its colour back, or a
  new one if another agent took it in the meantime.
- **A stopped agent's to-dos can be picked up again.** They stay linked to it,
  so a restart carries on where it left off, but they get their "start an
  agent" button back and can be multi-selected again — starting a new agent
  on one hands it to that agent. Restarting the stopped agent afterwards
  resumes its own conversation, so it may still think it is on that to-do.

## [2.3.0] - 2026-10-08

### Added

- **Give each project its own badge colour.** Click a project's initials
  badge in the sidebar to pick a colour, so projects are easy to tell apart;
  the initials switch to black or white, whichever reads better. Right-click
  the badge (or press Delete while it has focus) to go back to the theme
  accent. The colour is remembered across restarts.

## [2.2.0] - 2026-10-08

### Added

- **Fold a project's sidebar sections away.** Click a subsection title
  (To-dos, Agents, Processes, Terminals, Worktrees, Scratchpads) to collapse
  it to one line with a count; a collapsed section still flags a row that
  needs your input. A new button on the project header collapses or expands
  every section of that project at once. What you folded is remembered per
  project across restarts, and adding something to a collapsed section opens
  it again.

## [2.1.4] - 2026-10-08

### Fixed

- **Messages waiting to be sent stay visible.** A long conversation with the
  assistant could squeeze the "thinking" indicator and the queued messages at
  the bottom of the panel down to nothing, hiding what was about to be sent.
  They now keep their space however long the transcript gets.

## [2.1.3] - 2026-10-08

### Changed

- **Every merge ships a release.** Each change that lands on `main` now becomes
  a numbered version right away, with downloadable macOS and Windows installers
  attached to its GitHub Release — no more waiting for a release to be cut by
  hand.

## [2.1.2] - 2026-09-30

### Changed

- **The assistant only orchestrates, and keeps it short.** Shell and file work
  and the lookups you ask for now go to a spawned agent instead of the
  assistant's own tool calls; the one thing it still does itself is send a
  message you ask it to send. Agent updates are limited to real warnings and
  whether agents are done. Before stopping a finished agent it checks for
  follow-up work you typed and completes its to-do, and "stop the agents" only
  stops the project you are talking about. These used to depend on the
  assistant's memory file; they are now built in.

## [2.1.1] - 2026-09-30

### Added

- **What you have selected is handed to the assistant.** ⌘/Ctrl+click any
  number of to-dos, scratchpads or agents — or select text anywhere, including
  in a terminal — and each appears as a chip above the assistant's composer.
  Your next message carries them along, so you can ask "what's left on these?"
  instead of pasting ids or re-explaining which items you mean. Merely opening
  an item does not send it: reading a to-do should not put it in your next
  message. Each chip has an × if you'd rather not send it, and a selection you
  make inside the chat panel itself is left alone.

## [2.1.0] - 2026-09-30

### Added

- **Run everything inside WSL on Windows.** Settings → General → Backend takes
  a WSL distro name; after a restart, services, terminals, agents, the
  personal assistant, git worktrees and the MCP install all run inside that
  distro, with projects opened from `\\wsl.localhost\<distro>\…`. Podium
  itself stays a native Windows app. It needs WSL's mirrored networking, and
  warns at startup when that is off.

### Changed

- **One title bar on macOS.** The window's native title bar is gone: the
  traffic lights now sit in Podium's own header strip, with the logo centered
  and the header still draggable. Native fullscreen and the menu bar are
  unchanged.

### Fixed

- **Background processes on Windows no longer wait for their terminal to be
  opened.** An `auto_start` service, an agent started from a to-do, or a
  supervised restart used to sit blocked until someone opened its terminal,
  because Windows' console layer waits for a reply that only the terminal view
  gave. Podium now answers it itself.
- **No more console windows flashing up on Windows.** Starting an agent or
  focusing a terminal briefly popped up an empty console window, because the
  git and shell calls Podium makes in the background each got a visible
  window of their own. They now run hidden.
- **No more double title bar on Windows.** The native Windows title bar sat
  on top of Podium's own, each with its own minimize/maximize/close buttons.
  Only Podium's remains, and it keeps working with Aero Snap.

## [2.0.4] - 2026-09-30

### Fixed

- **The "working…" indicator tells the truth.** Opening a terminal made its
  agent read "working…" for a few seconds, just from the terminal redrawing
  itself, and an agent whose terminal you never opened always read "idle". The
  state now comes from the agent's actual output, whether or not its terminal is
  open, and a single repaint no longer counts as work.

## [2.0.3] - 2026-09-29

### Fixed

- **No "check what it did" nudge for an agent you stopped.** Stopping an agent
  (the stop button, or the assistant stopping or respawning it) still made the
  assistant report it as finished. That report is now kept for agents that exit
  on their own or crash.

## [2.0.2] - 2026-09-29

### Fixed

- **A long assistant queue no longer hides the chat.** Messages waiting for the
  current turn grew the pinned list at the bottom of the panel until it covered
  the whole transcript. The list now tops out at 40% of the window height and
  scrolls within itself.
- **Waiting messages are cut off cleanly.** A queued message's preview broke
  mid-word after a few characters. It now wraps at word boundaries and shows at
  most three lines, ending in an ellipsis; the edit action still shows the full
  text.

## [2.0.1] - 2026-09-22

### Added

- **Scratchpads are named after what is in them.** A new scratchpad used to be
  called after the minute it was created (`07-14-09-30 Scratchpad`), so the
  sidebar read as a list of timestamps rather than a list of notes. The name is
  now the first usable line of the content, refreshed as you write, and
  `Untitled scratchpad` until there is something to name it after. Typing a
  title of your own pins it — later content edits leave it alone — and clearing
  it hands the name back to the content. Existing timestamp-named scratchpads
  adopt their content's name the next time they load; a name you chose yourself
  is never touched.
- **Agents can set a to-do's description when they create it.** The `add_todo`
  MCP tool now takes an optional `description` alongside the text, so a to-do
  handed over by an agent arrives complete instead of with an empty description
  nobody noticed until they opened it.

### Fixed

- **Tooltips no longer dump whole paragraphs next to the cursor.** To-do,
  scratchpad and Podium-nudge tooltips showed their free-form text verbatim,
  newlines and all, which the OS painted as an unreadable wall. They now show a
  single-line preview, cut at a word boundary and ellipsized.

## [2.0.0] - 2026-09-09

### Added

- **Podium has a personal assistant.** A collapsible, resizable chat panel on
  the right side of the window, backed by a long-lived headless Claude Code
  session that sees your whole workspace — every project, its processes,
  agents, to-dos and scratchpads. Ask it what is going on, hand it work, or
  press **Start my day** in the empty state for a prioritised rundown of open
  work across all projects. It never edits code itself; implementation work
  goes out to a spawned agent. The transcript is replayed when you reopen the
  panel, each launch starts a fresh conversation, and **Settings → Assistant**
  holds the autonomy mode and the proactivity toggle (the chat header keeps
  its own autonomy picker; conversation reset moved there too).
- **The assistant acts in Podium, gated by an autonomy mode.** It spawns
  agents, starts and stops processes, and manages to-dos and scratchpads
  through Podium itself. `confirm-destructive` (the default) parks stopping,
  archiving and removing actions as a proposal card you Confirm or Reject
  before anything runs; `propose-all` asks for everything that changes state,
  `full` asks for nothing. The gate is enforced by Podium, not by prompt
  wording, and a tool it does not recognise counts as state-changing.
- **The assistant keeps a memory you can read and edit.** Free-form markdown
  that survives new conversations and app restarts — it maintains the file
  itself, and the panel's memory view lets you read, edit or clear it.
- **Podium speaks first when something needs you.** With proactivity on
  (default), Podium raises an agent that finished or crashed, an agent stuck
  on a prompt or silent for five minutes, and — every six hours — to-dos left
  open longer than a week. A stuck agent that is still blocked ten minutes
  later is escalated: the assistant proposes the answer it would give, and
  once you agree it respawns the agent with it, since nothing can type into a
  running agent's terminal. Each notice says why the assistant is speaking.
  Desktop notifications stay quiet for these unless the assistant marks the
  turn as needing you now, so a steady trickle of "agent finished" does not
  reach your desktop.
- **The assistant can check your external sources.** Every ten minutes by
  default it looks over the enabled sources (Slack, Teams) for anything
  needing you, using your own MCP servers — no separate integration. A sweep
  that finds nothing says nothing: no chat row, no notification.
- **Send images and files to the assistant.** Drop them on the panel, paste
  into the composer, or use the paperclip picker; each attachment shows as a
  removable chip. Images travel inline and are shrunk to what the vision
  pipeline uses anyway; other files are handed over by path for the assistant
  to read. Oversized or unreadable attachments are refused by name — the rest
  of the message still goes.
- **Messages you send mid-turn wait their turn instead of landing
  mid-answer.** Waiting messages sit in a sticky block at the bottom of the
  transcript, below the thinking indicator, where each can be sent into the
  running turn right away, edited, or removed. An interrupt control stops the
  turn in flight and picks the conversation back up. Podium's own notices are
  delivered verbatim and consecutive ones arrive as a single turn, so a quiet
  day no longer stacks up a backlog.
- **The composer recalls what you sent before** with the arrow keys.
- **Podium's own sessions can message each other.** The assistant and every
  spawned agent can reach each other directly, and a session can ask Podium
  to verify who a message really came from — answered from Podium's own
  process registry, not from anything the message claims about itself.
- **New MCP tools for agents.** `report_to_assistant` (a finished or blocked
  agent hands the assistant a short summary), `respawn_agent` (replace a
  stuck agent, inheriting the to-dos it owned), `remove_process`,
  `verify_sender`, and `update_assistant_memory`. `spawn_agent` also takes
  extra CLI args now, so a caller can pick the model or reasoning effort for
  the agent it spawns.

### Changed

- **Breaking: the MCP `list_todos` tool no longer returns comment bodies.**
  Every to-do now carries a `commentCount` instead of its `comments` array —
  the full history used to fill an agent's context with discussion it never
  asked for. Pass the new `todo_id` argument to read one to-do in full,
  comments included. An external MCP client reading `comments` off the list
  response has to switch to that.

### Fixed

- **Agents and worktrees launch on Windows again.** Every agent spawn failed
  with `os error 5`, for two separate reasons: the project root was handed to
  Windows as a `\\?\`-prefixed verbatim path, which it refuses as a working
  directory, and the agent's own binary was resolved by a PATH search that
  accepted the first matching *directory* — Claude Code's data folder — as
  the executable. Both are fixed, and a Windows project's existing to-dos,
  scratchpads and sidebar entries are migrated onto the corrected path so its
  history does not fork in two.

### Security

- **An agent can no longer make a to-do comment look like you wrote it.**
  Author values that impersonate the user fall back to "agent", enforced by
  Podium rather than by asking politely.

## [1.6.0] - 2026-08-20

### Added

- **Worktrees have their own sidebar list, with terminals and quick commands
  that run inside them.** A new Worktrees section under Terminals lists every
  Podium-managed worktree; each row can open a terminal rooted in that
  checkout, open it in Warp, launch a quick command there (the Commands list,
  now targeted at the worktree), or remove it — the same dirty-changes
  confirmation as before, and removal stays blocked while a process runs in
  it. Terminals running in a worktree nest under their worktree's row instead
  of the plain Terminals list. The list stays live: a worktree created or
  removed by an agent over MCP shows up immediately.

### Changed

- **Worktree actions are much faster.** Listing, creating and removing
  worktrees spawned a fresh login shell for every git call, which took
  seconds per action on heavy shell profiles. The git binary is now resolved
  once and invoked directly, and the sidebar list renders instantly from a
  cache that is refreshed in the background.

### Removed

- **The Worktrees modal.** Its button on the Agents header is gone; the
  sidebar list replaces it.

## [1.5.0] - 2026-08-20

### Added

- **A terminal waiting on input now says so, just like an agent does.** A shell
  stalled on a `sudo` password, an `ssh` login, a key passphrase, a git
  credential request or a plain `[y/N]` confirmation looked exactly like an idle
  one, so you only found out by opening it. Every running process is now checked
  for a prompt on a quiet screen — its sidebar row and pane header show
  `needs input` with the blue dot, and you get a "Terminal needs your input"
  notification. Non-agents show only that state: `working…`/`idle` says nothing
  useful about a plain shell.

## [1.4.4] - 2026-08-20

### Changed

- **Terminal commands can be renamed and edited.** Your own quick commands in
  the Terminals "Commands" list were add-or-delete only, and a terminal started
  from one was named after the raw command line. Each command now takes an
  optional name — shown above the command line in the list — and an edit button
  loads it back into the form to change either. A terminal launched from a
  command inherits its name, and a justfile recipe now opens a terminal called
  `build` rather than `just build`. Existing commands keep working and simply
  start out unnamed.

## [1.4.3] - 2026-08-20

### Added

- **An idle agent now shows the shells still running behind it.** A session can
  be done talking to you while a build or test run continues in the background,
  which looked no different from a finished agent. Its status now carries the
  count — `idle · 1 shell`, `working… · 2 shells` — on both the sidebar row and
  the terminal-pane header, so you can tell a session that is waiting on you
  from one that is waiting on a build.

## [1.4.2] - 2026-08-20

### Changed

- **A quieter sidebar: secondary icons appear on hover.** Section headers now
  show only their "+" at rest — archive, worktrees, Open in Warp and Commands
  fade in when you hover the header. A project's rename, reload and remove
  buttons stay hidden until you hover that project's row, the way its drag
  handle already did. Nothing moves as the icons appear, and every button is
  still reachable by keyboard.

## [1.4.1] - 2026-08-20

### Changed

- **The agent makes its own worktree now.** Ticking "Work in a git worktree"
  when spawning an agent no longer creates the checkout up front: the agent
  starts in the project root — with the tooling and caches you set up there —
  and is told to create the worktree itself before its first code change. The
  worktree name is optional; leave it blank and the agent names it (and its
  `podium/<name>` branch) after the work, including when it is spawned across
  several to-dos.

### Removed

- **The per-worktree branch choice.** With the agent creating the worktree,
  "let the agent name the branch" is simply leaving the worktree name blank, so
  the detached-HEAD option is gone.

## [1.4.0] - 2026-08-20

### Added

- **Terminal quick commands.** The Terminals section header has a Commands
  button opening a per-project Commands modal. It lists the public recipes found
  in the project root's `justfile` (re-scanned every time you open it, no `just`
  binary needed) plus any commands you add yourself, and clicking one launches a
  new terminal running it. Your own commands are saved per project and can be
  removed on hover.
- **Open in Warp.** The Terminals section header can open the project root in a
  new Warp tab. If Warp isn't installed you get a toast instead.

## [1.3.0] - 2026-07-23

### Added

- **Git worktree management for agent isolation.** Agents can run in a
  Podium-managed git worktree under `.podium/worktrees/`, so their edits don't
  touch your working tree. Tick "Run in a git worktree" when spawning an agent,
  or have an agent create one mid-session over MCP (`create_worktree`,
  `list_worktrees`, `remove_worktree`). Worktrees list in a new Worktrees modal
  and show as a badge on the agent's sidebar row; removal is refused while an
  agent still runs in one or it has uncommitted changes (unless forced), and the
  branch is always kept.
- **Branch choice per worktree.** A worktree checks out a fresh `podium/<name>`
  branch by default, or you can let the agent name its own branch (it starts on
  a detached HEAD). Spawning one agent across several to-dos now forces you to
  name the worktree explicitly, rather than borrowing one arbitrary to-do's text.
- **Worktree prompting.** With the (default-on) "suggest worktree" setting,
  agents are asked to offer an isolated worktree before their first code change;
  an agent already running in a worktree is told so and won't offer another.
- **Git branch in the agent window header.** The focused agent window shows the
  git branch (and worktree) of its working directory.
- **Per-session CLI args.** The New agent dialog has an Arguments field, seeded
  from the adapter's default args, that overrides them for that spawn only.

### Fixed

- **Worktree deletion feedback.** Deleting a worktree now disables the delete
  buttons and marks the row "Removing…" while it is in flight, so it can't be
  triggered repeatedly.
- **Window-switch lag.** The git-branch lookup for a focused window now runs off
  the main thread (with a cache), so switching between agent windows no longer
  briefly freezes the UI.

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
