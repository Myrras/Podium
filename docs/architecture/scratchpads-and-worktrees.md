# Scratchpads and worktrees

Scratchpads (`scratchpad.rs`) are shared freeform notes per project. Worktrees
(`worktree.rs`) are Podium-managed git worktrees for agent isolation.

## Scratchpads

Each project has shared freeform-notes scratchpads, visible to the user and
every agent (MCP).

### Storage

Persisted in one `scratchpads.json`, keyed by project root path like to-dos.

### Tags and archive

Scratchpads carry free-text **tags** (`add_tag`/`remove_tag`, dedup by exact
value). They can be **archived/unarchived** manually (`set_archived`, mirroring
to-dos' archive semantics but with no auto-archive sweep) and **deleted** from
the Archive view (`scratchpad_remove` / `Orchestrator::remove_scratchpad`,
again mirroring to-dos). Deletion lives behind archiving.

### Conflict check

Content/title updates require an `expected_updated_at` matching the
scratchpad's current `updatedAt`. A stale value (a concurrent user or agent
edit landed first) is rejected as `CoreError::ScratchpadConflict` instead of
silently overwriting it. The frontend surfaces this as an in-pane
reload/force-save choice rather than a toast.

### Naming

A scratchpad is **named after its content**. The title is the first usable line
(heading/bullet/quote markers stripped, truncated to 60 chars on a word
boundary), refreshed on every content update, with `Untitled scratchpad` until
something is written. Setting a title (`update_title`) pins it, so later
content edits leave it alone. A blank title hands the name back to the content.

Scratchpads stored with the old `MM-DD-HH-MM Scratchpad` title adopt their
content's name on load (`adopt_auto_titles`, idempotent). A hand-picked name is
left alone.

## Worktrees

### Layout

Worktrees live at `<project_root>/.podium/worktrees/<name>` (name slugified,
auto-de-duplicated with `-2`, `-3`, …). `create(root, name)` checks out a fresh
`podium/<name>` branch off HEAD. `.podium/` is excluded via
`.git/info/exclude` (never `.gitignore`).

### Creation

**Podium never creates a worktree at spawn time.** An agent always starts in
the project root (where the user's tooling and caches live). `agent_spawn`'s
`worktree` flag, the spawn checkbox, only makes its identity prompt tell it to
create one itself via the `create_worktree` MCP tool before its first code
change, named `worktreeName` when given, else a name the agent picks. So every
worktree comes from `create_worktree`, whether at the start of a session or
mid-session.

The global `suggest_worktree` setting (default on) makes agent identity prompts
*ask* the user about a worktree before the first code change. The spawn flag is
the same instruction without the asking, and wins over the setting.

### Backend and state

Every worktree git call runs `platform::git_command(wsl)` (the project's
backend) rather than going through the shell. Natively that is a git path
resolved once via the login shell, then reused (not a login shell per call,
which was slow on heavy shell profiles). In WSL-backend mode it is the distro's
git via `wsl.exe -e git`.

`git worktree list --porcelain` is the source of truth and `list` has no
events. `create`/`remove` emit `WorktreesChanged` for the project, which the
frontend's `worktreeStore` uses (alongside a background load on project open)
to keep its cache warm.

`ProcessInfo.worktree` is derived from the process cwd and feeds the sidebar
badge, `in_use` and the removal guard. An agent that creates its own worktree
keeps the root as cwd, so it does not light those up.

### Removal

Removal is refused while an active process runs in the worktree
(`WorktreeInUse`) or while it has uncommitted changes unless forced
(`WorktreeDirty`). The branch is always kept.

### Sidebar

`WorktreeSubsection.tsx` renders a "Worktrees" subsection directly under
Terminals, reading from `worktreeStore`. It is hidden entirely when the project
has none (no header "+"; worktrees only ever come from `create_worktree`, never
a sidebar action).

Each row carries four actions:

- open a terminal rooted in it (`cwd: .podium/worktrees/<name>`),
- open it in Warp (`worktree_open_in_warp`, same URI scheme as the
  project-level button),
- open the Commands modal targeted at it (its quick commands then run there
  instead of the project root), and
- remove it (`worktreeRemove(force=false)`; a `worktreeDirty` rejection prompts
  a confirm before retrying with `force=true`).

A worktree's own terminal processes (`p.worktree === wt.name`) render nested
under its row via the shared `ProcessRow` and are excluded from the plain
Terminals list. "In use" for the delete button is derived live from the process
store (any non-exited process with that `worktree`) rather than the worktree's
own cached `inUse` field, which can go stale between backend refreshes while
the sidebar stays live. The backend's `WorktreeInUse` guard remains the
authority and surfaces as a toast if it fires anyway.
