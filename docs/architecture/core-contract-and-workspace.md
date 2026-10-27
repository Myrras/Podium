# Core contract and workspace

### The core contract

`Orchestrator` (`crates/podium-core/src/orchestrator.rs`) is the **single
entry point**: projects, processes, agents, attach/stdin/resize, events,
shutdown. `src-tauri` holds one `Arc<Orchestrator>` in `AppState` and never
reaches around it.

Each project's sidebar shows or hides its To-do's / Terminals / Worktrees /
Scratchpads / Git sections via `ProjectInfo.sections`
(`ProjectSections`, all on by default; partial JSON loads).
Presentation only: hiding never stops processes, deletes data or affects MCP.
Agents and Processes are never hidden. Persisted per project in
`workspace.json` (`WorkspaceEntry.sections`, `None` = defaults) and
re-applied in `project_open`.

Sidebar projects are a **persistent workspace**: `project_open` adds the
project's root to `workspace.json` (app data dir) and `project_close` removes
it; at startup the frontend calls `workspace_list` and re-opens every path via
`project_open`. A path that fails to open (folder moved/deleted, drive not yet
mounted) is *not* auto-pruned — a transient failure must not silently drop the
project — but its error toast offers a "Remove from workspace" action wired to
`workspace_remove` for permanent failures. Each entry is a `{ path, name?,
color? }` object — the optional `name` is a user-set display-name override (set
via `project_rename`), `color` the badge colour (`#rrggbb`, set via
`project_set_color`, validated in core since it lands in a CSS property) and
list position is the sidebar order (set via `project_reorder`); legacy
bare-path-string files are migrated on load. `project_open` re-applies a stored
name override and colour so a renamed/recoloured project comes back that way
after a restart. Opening the same folder concurrently (e.g. React StrictMode's
double-mount firing `restoreWorkspace` twice) dedupes to one project: the
"already open" check and the id reservation happen under one lock acquisition,
atomically, before the async `podium.yml` load.
