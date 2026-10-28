# Core contract and workspace

`Orchestrator` (`crates/podium-core/src/orchestrator.rs`) is the single entry
point of the core, and the sidebar's projects form a persistent workspace
stored in `workspace.json`.

## The core contract

`Orchestrator` is the **single entry point**: projects, processes, agents,
attach/stdin/resize, events, shutdown. `src-tauri` holds one `Arc<Orchestrator>`
in `AppState` and never reaches around it.

## Sidebar sections

Each project's sidebar shows or hides its To-do's / Terminals / Worktrees /
Scratchpads / Git sections via `ProjectInfo.sections` (`ProjectSections`, all on
by default; partial JSON loads). This is presentation only: hiding never stops
processes, deletes data or affects MCP. Agents and Processes are never hidden.
Sections are persisted per project in `workspace.json` (`WorkspaceEntry.sections`,
`None` = defaults) and re-applied in `project_open`.

## Persistent workspace

Sidebar projects are a **persistent workspace**: `project_open` adds the
project's root to `workspace.json` (app data dir) and `project_close` removes
it. At startup the frontend calls `workspace_list` and re-opens every path via
`project_open`.

### Failed opens

A path that fails to open (folder moved/deleted, drive not yet mounted) is
*not* auto-pruned, because a transient failure must not silently drop the
project. Its error toast offers a "Remove from workspace" action wired to
`workspace_remove` for permanent failures.

### Entries

Each entry is a `{ path, name?, color? }` object.

- `name` is the optional user-set display-name override (set via
  `project_rename`).
- `color` is the badge colour (`#rrggbb`, set via `project_set_color`,
  validated in core since it lands in a CSS property).
- List position is the sidebar order (set via `project_reorder`).

Legacy bare-path-string files are migrated on load. `project_open` re-applies a
stored name override and colour so a renamed/recoloured project comes back that
way after a restart.

### Concurrent opens

Opening the same folder concurrently (e.g. React StrictMode's double-mount
firing `restoreWorkspace` twice) dedupes to one project. The "already open"
check and the id reservation happen under one lock acquisition, atomically,
before the async `podium.yml` load.
