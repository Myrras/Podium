# Podium

A cross-platform **desktop cockpit for AI coding agents**, built in Rust +
Tauri. Open a project folder and Podium runs its dev servers, ad-hoc
terminals, and AI coding agents (Claude Code, Auggie) — each in its own PTY with
a real terminal view and lifecycle supervision. A built-in MCP server lets the
agents inside Podium inspect and orchestrate the very processes Podium manages,
including spawning more agents.

![Podium — the sidebar, a running agent's terminal and the assistant panel](docs/media/overview.gif)

## The shape of it

One window, three columns:

- **Left — the sidebar.** Your projects, each with its own To-dos, Agents,
  Processes, Terminals, Worktrees and Scratchpads.
- **Middle — the work area.** A real terminal for whatever you have focused,
  or the detail pane of a to-do or scratchpad.
- **Right — the personal assistant.** A chat panel that can see and drive
  everything Podium manages, across all open projects.

## Feature tour

### Projects

Open a folder and it stays. The sidebar is a **persistent workspace**: every
open project is remembered and re-opened at launch. Projects can be **renamed**
(double-click the name), **recoloured** (click the initials badge to pick a
colour; right-click it to go back to the theme's accent) and **reordered** (drag
the grip in the project header) — all survive a restart. A project whose folder
has moved is never silently dropped; its error toast offers "Remove from
workspace" instead.

Each project also gets an **Open in Warp** action and a **Commands** modal
that lists the `just` recipes it finds in the project's `justfile` alongside
your own saved per-project commands — one click runs any of them in a fresh
terminal.

The **Sections** button in a project header hides or shows its To-do's,
Terminals, Worktrees, Scratchpads and Git board, per project. Hiding is
presentation only: terminals keep running and agents still see everything
over MCP.
Both actions above live on the Terminals header, so they go away with that
section. With Worktrees off, a worktree's terminals are listed under Terminals.

### Processes, terminals and supervision

Three kinds of process, all in their own PTY with a real terminal view:

| Kind         | Comes from                        |
| ------------ | --------------------------------- |
| **Service**  | `podium.yml` (dev servers, …)     |
| **Terminal** | you, ad hoc                       |
| **Agent**    | an agent adapter (Claude Code, …) |

Commands run through your login shell (`$SHELL -lic`), so nvm/homebrew PATH
edits in `.zprofile` *and* `.zshrc` are honoured. Stopping kills the whole
**process group**, so children can't outlive their parent, and a stop you
asked for is never counted as a crash.

Services can be supervised: `auto_restart: on-crash | always` restarts with
exponential backoff (500 ms doubling up to 30 s), a circuit breaker at 5
restarts in a rolling 60 s window, and a backoff reset after 60 s of stable
running. Starting a process by hand clears both.

### Agents

Spawn a coding agent into any project. Adapters today: **Claude Code**
(`claude-code`) and **Auggie** (`auggie`); availability is probed through the
login shell, so version-manager installs are found. Up to **8 agents per
project** — agents can spawn agents over MCP, so there's a recursion guard.

A spawn can carry a prompt, one or more **to-dos** or **scratchpads** to work
on, a request to work inside a **git worktree**, and per-spawn CLI args
(`["--model", "opus"]` picks the model for that agent alone).

Every agent gets a **session colour** from an 8-hue palette, avoiding colours
live agents already hold. Its sidebar row and the to-dos it owns are tinted
the same colour, so you can see at a glance who is on what. A stopped agent
loses its colour (its to-dos stay linked, tinted grey, but you can start
another agent on them, which takes them over); restarting it gives it its
colour back (or a new one if another agent took it meanwhile) and, for
Claude Code, **resumes its conversation** instead of starting an empty one
(if Claude Code no longer has that conversation, or resuming it fails
straight away, the agent starts a fresh one from its original prompt). After
a `/clear` it resumes the conversation from after the clear, once Claude
Code has stored it — on macOS and Linux, when Podium starts `claude` itself
(no wrapper command, shell function or fish shell), and unless the agent
ended itself with `/exit`; otherwise, on Windows or WSL, or when reading
which session the agent is on takes over 200 ms, it is the one from before.
If you put another agent on a stopped agent's to-dos and then restart the
stopped one too, it resumes its own conversation and may still think it is
working on them. The row also
shows an activity state: **working…** (sustained output in the last 2.5 s —
a single redraw does not count, and it works whether or not you ever opened
the terminal), **idle**, or **needs input** — the last one is read off the
terminal screen for *every* process, so a terminal stalled on a `sudo`
prompt surfaces the same way.

An agent that finished, crashed or was stopped **leaves the list by itself**
5 minutes after it exited — its row, its terminal (an open one closes) and
its to-do links go, the same as removing it by hand. Restarting it before
then keeps it, and the 5 minutes count again from its next exit. Terminals,
services and the assistant's sub-agents (which tidy themselves up) are never
removed this way. Change the delay, or turn it off
with `0`, in Settings → Agents.

![Spawning an agent from a to-do — both rows pick up the same session colour](docs/media/agent.gif)

### To-dos

A shared to-do list per project, visible to you in the sidebar and to every
agent over MCP. A to-do carries a description, a **comment thread** agents
append progress notes to, and pinned **issue/PR links**. Done to-dos left
over from an earlier day are archived automatically; anything can be archived
by hand and found again in the Archive modal. Over MCP, `list_todos` leaves
the archive out unless it's called with `include_archived: true` — handy for a
script that syncs to-dos from an issue tracker and must not recreate one you
archived.

An agent **claims** a to-do when it starts working on it, so the list doubles
as a who-is-doing-what board.

### Scratchpads

Shared freeform markdown notes per project, again visible to you and to
agents. Rich editor with a table of contents, free-text tags, and archiving.
Concurrent edits are handled honestly: an update carries the timestamp it was
based on, and a stale one is rejected as a conflict with a reload/force-save
choice rather than silently overwriting what someone else wrote.

### Git worktrees

Podium manages worktrees at `<project_root>/.podium/worktrees/<name>`, each on
a fresh `podium/<name>` branch (`.podium/` is excluded via
`.git/info/exclude`, never your `.gitignore`). Agents create them themselves
when asked to — they always *start* in the project root, where your tooling
and caches live.

Each worktree row in the sidebar can open a terminal rooted in it, open it in
Warp, run the Commands modal against it, or be removed. Removal is refused
while a process is running in it or while it has uncommitted changes (unless
forced); the branch is always kept.

### Git board

The **Git** pill at the right end of a project's header (it shows how many
cards are open) opens a Kanban board of the repo
behind its `origin` remote. If the repo has its own board (a GitLab issue
board with label lists, or a GitHub Project with a Status field), its columns
are used; otherwise there are four:

- **Open** — open issues nobody is visibly on yet.
- **In progress** — open issues with a linked PR/MR or an in-progress label
  (`In progress`, `workflow::In Progress`, `doing`, `wip`), and draft PRs/MRs.
- **In review** — open, non-draft PRs/MRs.
- **Done (14d)** — issues closed and PRs/MRs merged in the last 14 days.

GitHub Project columns need the `read:project` scope; if it is missing the
board says so and shows the default columns until you run
`gh auth refresh -s read:project`.

**Mine** (the default) shows issues assigned to you and PRs/MRs you opened;
**All** shows the whole repo. The header counts all-time open and merged
PRs (GitHub) or MRs (GitLab) in the same scope. Clicking a card opens its
description beside the board; each card can open in the browser, start an
agent on it, or become a to-do with the link pinned.

Drag a card to another column to move it on GitHub/GitLab itself: GitLab
labels (or close/reopen), GitHub close/reopen or your Project's Status (which
needs the `project` scope: `gh auth refresh -s project`). Columns only take
what makes sense — merged PRs/MRs stay put. A PR/MR that closes an issue on
the board is shown on that issue's card instead of as a card of its own; on
GitLab, an MR whose closed issue is not on the board lands on an issue it
mentions (`Onderdeel van #N`), so sub-issue MRs show on the parent. The
last board is kept on disk, so it appears at once, and agents can read and
move cards through the MCP tools.

The board stays off until you add a setup in Settings → Issues: before
that Podium fetches nothing (no `git`/`gh` calls in the background) and the
board just points you to Settings. GitHub works through your own `gh` login
(a setup can pick which `gh` account), github.com included; self-hosted
GitLab needs a personal access token with the `api` scope (moving cards and
unassigning write to GitLab), which lives only in the OS keychain (editing the
setup offers **Clear token**). A GitLab served under a path
(`https://host/gitlab`) keeps that path in the setup URL; the setup URL is
the GitLab itself, not a project page. In WSL-backend mode the distro's `gh`
and its login are used. The board refreshes every 5 minutes, on window focus after a minute, and on
demand; a failed refresh keeps the last board with the error inline.

### The personal assistant

A workspace-wide chat panel backed by a long-lived headless Claude Code
session with its own MCP endpoint into Podium. It reads and drives everything
Podium manages, but **only orchestrates** — project code work goes out to a
spawned agent, and lookups and other one-off work go to its own sub-agents (see
below); the one thing it does itself is send a message you ask it to send. Its
replies stay short: real warnings and whether agents are done, no recap of what
they did.

- **Autonomy** — `full`, `confirm-destructive` (default) or `propose-all`.
  Enforced server-side, not by prompt wording: a gated call never runs, it
  becomes a card you confirm or reject. Under `propose-all` the assistant
  also cannot message an agent directly, so a push go-ahead goes through a
  new agent you confirm; switching into or out of that mode restarts its
  session (between turns, keeping the conversation). The block applies from
  the next turn boundary: a turn already running when you switch can still
  message an agent until it ends.
- **Sub-agents** — anything beyond a reply from what it already knows (a
  lookup, a check, a one-off script) goes to a sub-agent, so the assistant
  answers you at once with one line and its result comes back later as a
  short summary, plus a to-do in the right project when follow-up is needed.
  A *lookup* sub-agent only reads (Slack, Teams, tickets, your MCP servers,
  summaries across projects): no shell, no file edits, and of Podium's own
  tools only the read-only ones — Podium's server refuses the rest. It runs
  without permission prompts, so it can still call your *other* MCP servers
  without asking, just as the assistant itself can; it is told to change
  nothing, but Podium cannot check that for servers that are not its own. A *full* one also has
  shell, files and git, for work that belongs to no single project; it asks
  for permission like any agent, and under `propose-all` starting one needs
  your confirmation (a lookup never does). Project code work still goes to a
  project agent. They are listed above the chat log with task, kind, running
  time and status (queued, running, needs input, done, failed, stopped);
  **stop** one, or
  **click** its row to open its terminal in the work area (for a lookup, that
  is its output).
  At most 4 run at once, the rest queue and start by themselves; finished ones
  stay a few minutes, then drop off. They never show in a project's sidebar
  and do not count towards its agent cap. A sub-agent you stop is reported to
  the assistant without a summary. Resetting the conversation while some are
  queued or running asks whether to keep them (their results then report
  into the new conversation) or stop them; quitting Podium stops them, and the
  quit warning counts them. They are not restored after a restart. A
  sub-agent stuck on a prompt is reported right away once you have opened its
  terminal; otherwise after five minutes of silence. **Questions:** a
  sub-agent only asks when it is truly blocked on a decision that changes the
  outcome; anything else it decides itself and mentions in its report. Its
  question goes to the assistant (the row shows *Needs input*, the question on
  hover), which answers it itself whenever it can tell from the task, the
  context, its memory or your to-dos — you only see it in the usual short
  report. Only a decision that is really yours (a preference, a risk, money,
  people) reaches you, as a *Needs you* item with the assistant's recommended
  answer and a desktop notification; reply to it and the assistant passes the
  answer on. You can also answer in the sub-agent's terminal yourself.
  **Cheap to start:** without a model in the assistant's args or your
  default Claude Code args, a lookup runs on Haiku and a full one on Sonnet.
  By default a sub-agent also starts *lean*: only Podium's MCP tools, no
  skills, no `CLAUDE.md`, hooks or auto-memory, and only the built-in tools
  its kind needs — about a quarter of the tokens of a normal first request.
  A lookup that needs your own MCP servers (Slack, Teams, a ticket system),
  a skill or a plugin is started without that, which the assistant decides
  per task. Your permission rules apply either way.
  Sub-agents load only your
  own Claude Code settings, not ones a folder could carry. If your default
  Claude Code args (Settings → Agents) carry your own `--settings`, sub-agents
  use yours instead of Podium's: a lookup then shows Claude Code's one-time
  bypass-mode warning in its terminal, and a full one asks before it reports
  back. Podium marks its own assistant folder as trusted in your Claude Code
  config (`~/.claude.json`, or the one in `CLAUDE_CONFIG_DIR`), so
  sub-agents never stop on Claude Code's folder-trust question. Only if it
  cannot (a WSL backend, no Claude Code config yet) does the first one ask:
  open it from the list and answer "Yes, I trust this folder" once.
- **Memory** — a markdown file the assistant maintains itself and you can
  edit; it is the continuity between days, since each app launch starts with
  a blank conversation.
- **Proactivity** — Podium speaks first when an agent exits, when an agent
  looks stuck (silent for 5 minutes, escalating once after 10 with a
  suggested answer, then silent until you type into that agent), and on a
  6-hourly sweep for to-dos open longer than a week.
- **External sweep** — every 10 minutes by default, it checks the sources you
  enabled (Slack, Teams — through your *own* MCP servers) and stays silent
  when there is nothing to report. Its actions show as one faint "Podium
  check" line in the log.
- **Responses** — the assistant files a short *Needs you* or *FYI* item in a
  collapsible list pinned above the chat (agent, project, to-do, age), and
  only for things you actually need: an agent blocked on a question only you
  can answer, a message from Slack/Teams/mail that asks you something, a
  confirmation card for an action it proposes, finished work you asked for
  (a result, a PR/MR link), or an agent that crashed. No "no change"
  reminders, no agent that simply finished with nothing left for you. The
  details stay in its chat reply. Tick an item off, or reply inline — the
  reply goes to the assistant as your message and shows in the chat as a
  one-line "Re: …". A newer item about the same agent or to-do replaces the
  old one, and open items survive restarts and new conversations.
- **Desktop notifications** — a new Responses item is the only thing that
  notifies you (toast, system notification and sound), and only while Podium
  is not in front with the assistant panel open. Several new items arrive as
  one notification. Chat replies, confirmation cards and Podium's own
  `[Podium]` nudges do not notify on their own.
- **The agent you are looking at stays quiet** — the assistant knows which
  agent you have open (the window is in front and its terminal is showing —
  not covered by a to-do, scratchpad or the Git board). It does not nudge,
  report or file items about that agent, so it never tells you what you are
  already reading. Switch away and it speaks again for anything new.
- **Start my day** — one button for a prioritised summary of open work across
  all projects.
- **Queue** — one turn runs at a time; anything you send mid-turn waits, and
  can be edited, removed or pushed into the running turn.
- **Push go-ahead** — tell the assistant a Claude Code agent may push, and
  it messages that agent instead of starting a new one; the agent pushes its
  own branch (`git push -u origin HEAD`), opens its PR/MR in the same
  repository (`gh pr create` / `glab mr create`) and reports back with the
  link; an agent on main or master pushes nothing and says so. Spawned
  agents may do both without a prompt. Force, delete, tag, mirror and
  explicit main/master pushes, pushes to another URL and PRs/MRs against
  another repository are blocked in their plain spellings — a push written
  another way (`git -C … push`, an alias) falls back to the normal prompt.
  Passing your own `--settings` in an agent's args turns this off for that
  agent.
- **Folded activity** — a run of actions is one collapsed line ("Ran 12
  actions · Slack, Podium, Notion, +2 ⚠ 1"); open it for each action with a count.
  Failures show as a ⚠ count, each with a clipped error inside (full text on hover), and Podium's own
  nudges get a fainter line. The log keeps your scroll position while it
  streams; a "New activity ↓" button (shown only when a new reply or notice arrives) jumps back to the bottom.
- **Attachments** — drop, paste or pick files; images are inlined (shrunk to
  1568 px), text is inlined, everything else travels as a path.

![The assistant parking a destructive action as a proposal card, then spawning an agent](docs/media/assistant.gif)

### Built-in MCP server

Podium runs a bearer-authenticated MCP server on an ephemeral localhost port
and exposes **36 tools** over it (33 to the agents it spawns; the assistant's route adds `add_response`, `spawn_subagent` and `list_subagents`; a lookup sub-agent's own route serves only the read-only tools and its report): process inspection and control, agent
spawning (and respawning a stuck agent with the answer in its prompt), the
full to-do, scratchpad and worktree surfaces, the Git board (`get_board`,
`move_board_item`), assistant memory, Responses items and sub-agents, and sender
verification for cross-session messages.

Agents Podium spawns get the URL + token automatically. External clients go
through the stdio bridge (see below).

## Requirements

- **macOS or Windows** (Linux runs but isn't packaged yet)
- **Rust** toolchain (stable; see `rust-toolchain.toml`)
- **Node.js** + **pnpm**
- An agent CLI on your `PATH` if you want to spawn agents — e.g. `claude`
  ([Claude Code](https://docs.claude.com/en/docs/claude-code))
- Optional, on Windows: a WSL 2 distro, to run everything inside WSL (see
  [Windows with a WSL backend](#windows-with-a-wsl-backend))

## Build & run

Tasks run through [`just`](https://github.com/casey/just) (run `just` to list
recipes); the Tauri CLI is the local dev-dependency, so no global install.

```sh
just dev      # run the app (Vite + Tauri, hot reload)
just build    # production desktop bundle (.app + .dmg on macOS)
just test     # Rust workspace tests + frontend Vitest
just lint     # clippy -D warnings + eslint
```

## Configuring a project

Drop an optional `podium.yml` at a project root to declare dev-server
processes and agent defaults. The file is **strict** — an unknown key is a
readable error rather than a silent no-op, and a broken config still opens the
project.

```yaml
name: Webshop            # display name (default: folder name)
icon_initials: WS        # sidebar badge, max 2 chars
processes:
  - name: dev-server     # required
    command: pnpm dev    # required; run via $SHELL -lic
    cwd: web             # relative to the project root (no absolute paths, no ..)
    auto_start: true     # default false
    auto_restart: on-crash   # never (default) | on-crash | always
    env:
      PORT: "3000"
agents:
  default_adapter: claude-code   # wins over the global default
  extra_args: []                 # combined with global args per the merge mode
```

A folder without a `podium.yml` still opens fine. Reloading the config
replaces the processes it owns (stopping running ones) and leaves terminals
you added by hand alone.

## Settings

| Tab           | What's in it                                                                                                                          |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| **General**   | Theme (**dark**, **light**, **retro**), reduce motion, terminal font size and shell command, notification sound (including your own file); on Windows, the WSL backend distro and whether Windows-drive projects run natively |
| **Agents**    | Default adapter, per-adapter command override and default args, how global and `podium.yml` args merge, "suggest worktree", "remember agent sessions" — when you quit Podium, each running Claude Code agent is resumed on its conversation (after a `/clear`, the one from after it, with the same limits as a restart) the next time its project opens, with its to-dos and model; off by default, not for agents whose own args pick the session, kept for 14 days, forgotten when turned off; "remove exited agents after" — minutes after its last exit that a finished, crashed or stopped agent is removed from the list, with its terminal and to-do links (default 5, `0` = never, at most 1440; terminals and services stay) |
| **Assistant** | Autonomy mode, proactive nudges on/off, external polling interval + Slack/Teams, reset conversation (asks keep or stop when sub-agents are running)                                 |
| **Issues**    | GitHub/GitLab setups for the Git board: name, URL (`http://` kept), `gh` account or GitLab token (stored in the OS keychain)          |
| **MCP**       | One-click registration of the stdio bridge with Claude Code and Auggie, with an installed indicator and a copyable command             |

## Windows with a WSL backend

On Windows, Podium itself runs natively, but it can run **everything it
executes** inside a WSL distro: services from `podium.yml`, terminals,
agents, the personal assistant, git and worktrees, and the MCP install. Your
projects then live in the Linux filesystem, where builds and git are fast
and your Linux toolchain applies.

**Prerequisite: mirrored networking.** Agents in WSL reach Podium's MCP server
on `127.0.0.1`, which only works with WSL's mirrored networking. Add this to
`%UserProfile%\.wslconfig`, then run `wsl --shutdown`:

```ini
[wsl2]
networkingMode=mirrored
```

Podium checks this at startup in WSL mode and shows a warning if it's off.

**Turn it on.**

1. Install the agent CLIs you use (e.g. `claude`) **inside the distro**. In
   WSL mode Podium looks for them there, not on Windows.
2. In **Settings → General → Backend**, enter the distro name (for example
   `Ubuntu`; `wsl -l -q` lists yours).
3. Restart Podium. The backend is fixed at startup; the field shows a restart
   hint until you do.
4. Open projects by their WSL path, e.g.
   `\\wsl.localhost\Ubuntu\home\you\webshop`.

Commands in `podium.yml` then run through `bash -lic` inside the distro, so
your `.bashrc`/`.profile` PATH setup applies. Terminals open your distro
shell. **Settings → MCP → install** registers the bridge with the distro's
`claude`/`auggie`. Clear the field and restart to go back to native Windows.

**Projects on a Windows drive.** By default a project opened from `C:\…`
(or any other drive) runs in the distro too, through `/mnt/c/…`. If you would
rather keep those on Windows, turn on **Run Windows-drive projects natively**
under the distro field (it appears once a distro is set) and restart: a
drive project then runs its services, terminals, agents and git on Windows
itself (cmd.exe, Windows git, the Windows `claude`), while projects inside the
distro (`\\wsl.localhost\…`) keep running in WSL. With it on:

- install the agent CLIs on Windows as well — the **New agent** dialog
  checks for them where that project's agents will actually run;
- new terminals in a drive project open PowerShell, whatever the **Shell**
  setting says (that setting is for the distro's shell);
- `podium.yml` commands in a drive project are `cmd.exe` syntax, not bash.

The personal assistant and the MCP install always use the configured
backend.

Limits: the distro is one global setting, **Settings → Agents** reports the
CLIs found in the configured backend (the distro), and **Open in Warp** is
unchanged.

## Things you'd otherwise never find

Podium leans on direct manipulation, so a fair amount is not signposted.

**Keyboard**

| Keys                       | Where                | Does                                                             |
| -------------------------- | -------------------- | ---------------------------------------------------------------- |
| `⌘,` / `Ctrl+,`            | anywhere             | Open Settings (macOS goes through the native menu item)           |
| `Enter` / `Shift+Enter`    | assistant composer   | Send / newline                                                    |
| `↑` / `↓`                  | assistant composer   | Shell-style recall of your previous messages                      |
| `Enter` / `Escape`         | any inline field     | Commit / cancel a rename, tag, to-do or title edit                |
| `F5`, `⌘R` / `Ctrl+R`      | anywhere             | Deliberately swallowed — a reload would take your agents with it  |

Arrow-key recall only fires when the caret is on the composer's first (or
last) line, so a multi-line draft still navigates normally.

**Mouse**

- **Double-click** a project name, process name or to-do title to rename it.
- **Click** a project's initials badge to give it its own colour;
  **right-click** it (or press **Delete** while it has focus) to reset it to
  the theme's accent.
- **Click** a subsection title (To-dos, Agents, Terminals, …) to collapse it to
  one line with a count; the collapse/expand-all button in the project header
  does every subsection of that project at once. Remembered across restarts.
  A collapsed Agents, Processes, Terminals or Worktrees header shows a pulsing
  blue dot when a row it hides needs your input, and a multi-select of to-dos
  or scratchpads keeps its "Start agent" bar while its section is folded.
- **⌘/Ctrl+click** a to-do or scratchpad title, or an agent/process row, to
  add it to a multi-select; **Shift+click** to extend the range over a list.
  Selected rows become chips above the assistant's composer, so your next
  message to it carries them as context; for to-dos and scratchpads the
  selection bar also starts one agent on the whole group. Selecting *text*
  anywhere — a detail pane, a terminal — does the same. Opening an item
  deliberately does not select it, and each chip has an × to drop it from the
  message.
- **⌘/Ctrl+click** a spawn button to pick the adapter instead of using the
  default.
- **Drag** the project header grip to reorder projects.
- **Drag** the divider beside the sidebar or the assistant to resize it —
  both widths persist.
- **Collapse** a project header to fold all its subsections away; that too is
  remembered, per project.
- **Drop or paste** files and images straight onto the assistant panel.
- Right-click is suppressed outside text fields — the webview's "Save as /
  Print" menu is not something a desktop app should offer.

**Behaviour worth knowing**

- Quitting while agents or terminals are running is intercepted with a
  warning, and window size and position are restored on next launch.
- Terminal scrollback survives tab switches: xterm instances live outside
  React in a registry keyed by process id.
- A comment written over MCP cannot impersonate you — an author of
  "you"/"user" falls back to `agent`.
- Spawned agents get `PODIUM_PROJECT_ID` and `PODIUM_PROCESS_ID` in their
  environment; that's how an agent claims a to-do or renames its own session.
- `PODIUM_APP_DATA_DIR` overrides where the app (and the bridge) looks for its
  data directory.
- Podium's own sessions can message each other, and `verify_sender` is the
  only trustworthy answer to "did this really come from Podium?".

## Pointing an external agent at Podium

Podium runs a bearer-authenticated MCP server and ships a stdio bridge
(`mcp-bridge`, a subcommand of the app binary) so external MCP clients keep
working across restarts even though the port and token rotate each launch.
Register it once — the Settings → MCP tab has one-click install cards, or by
hand for Claude Code:

```sh
claude mcp add --scope user --transport stdio podium -- /path/to/podium mcp-bridge
```

For Auggie:

```sh
auggie mcp add podium --command /path/to/podium --args mcp-bridge --replace
```

Agents Podium spawns itself get the URL + token automatically. The token never
crosses the Tauri IPC bridge, is never logged, and reaches agents and the
stdio bridge only through 0600 files that are wiped on every start.

## Architecture

`podium-core` (zero Tauri dependency) holds the orchestration/PTY/MCP domain;
`src-tauri` is a thin IPC adapter; `src/` is the React frontend. See
[`docs/architecture/`](./docs/architecture/) for the full architecture and IPC
contract, and [`CLAUDE.md`](./CLAUDE.md) for conventions.

## Keeping this file current

This README is the user-facing half of the documentation; `CLAUDE.md` and `docs/architecture/` are the
contributor-facing half. A change that alters what a user can *see or do* —
a new feature, a shortcut, a setting, a `podium.yml` key, an MCP tool — belongs
in both, in the same PR as the change itself.
