# Tauri IPC contract

Commands return `Result<T, IpcError>` where `IpcError = { kind, message }`
(messages are Podium-owned text — never terminal output or secrets). Tauri 2
maps camelCase JS argument keys to the snake_case Rust parameters.

## Projects & workspace

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `project_open`          | `{ path }` (absolute, from folder picker)   | `ProjectInfo`     |
| `project_close`         | `{ projectId }`                             | –                 |
| `project_list`          | –                                           | `ProjectInfo[]`   |
| `project_config_reload` | `{ projectId }`                             | `ProjectInfo`     |
| `project_rename`        | `{ projectId, name? }` (blank clears)       | `ProjectInfo`     |
| `project_set_color`     | `{ projectId, color? }` (`#rrggbb`, blank clears) | `ProjectInfo` |
| `project_reorder`       | `{ ordered: string[] }` (project ids)       | `ProjectInfo[]`   |
| `project_set_sections`  | `{ projectId, sections: ProjectSections }`  | `ProjectInfo`     |
| `project_just_recipes`  | `{ projectId }`                             | `string[]`        |
| `project_open_in_warp`  | `{ projectId }`                             | –                 |
| `recents_list`          | –                                           | `RecentProject[]` |
| `recents_remove`        | `{ path }`                                  | `RecentProject[]` |
| `workspace_list`        | –                                           | `string[]`        |
| `workspace_remove`      | `{ path }`                                  | `string[]`        |

## Processes

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `process_add`           | `{ projectId, spec: NewProcess }`           | `ProcessInfo`     |
| `process_remove`        | `{ processId }`                             | –                 |
| `process_list`          | `{ projectId? }`                            | `ProcessInfo[]`   |
| `process_git_branch`    | `{ processId }`                             | `string \| null`  |
| `process_report_waiting` | `{ processId }` (agent showing a prompt)   | –                 |
| `process_report_viewing` | `{ processId? }` (the agent open on screen; omitted = none; in memory only) | – |
| `process_activity`      | –                                           | `Record<ProcessId, OutputActivity>` (`{ quietMs, burstMs }` per running process that printed) |
| `process_rename`        | `{ processId, name }`                       | `ProcessInfo`     |
| `process_start`         | `{ processId }`                             | –                 |
| `process_stop`          | `{ processId }`                             | –                 |
| `process_restart`       | `{ processId }`                             | –                 |
| `process_write`         | `{ processId, dataB64 }`                    | –                 |
| `process_resize`        | `{ processId, cols, rows }`                 | –                 |
| `process_attach`        | `{ processId, channel: Channel<TermEvent> }`| –                 |

## Backend & adapters

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `backend_settings_get`  | –                                           | `BackendSettingsDto` (`{ wslDistro, activeWslDistro, nativeDriveProjects, activeNativeDriveProjects }`, empty distro = native) |
| `backend_settings_set_wsl_distro` | `{ distro? }` (blank = native; must be installed — checked against `wsl.exe -l -q`; applies on restart) | `BackendSettingsDto` |
| `backend_settings_set_native_drive_projects` | `{ enabled }` (WSL mode: run Windows-drive projects natively; applies on restart) | `BackendSettingsDto` |
| `backend_wsl_networking_mode` | – (WSL mode only; the app warns when not `mirrored`) | `string \| null` |
| `adapters_list`         | `{ projectId? }` (probe on that project's backend) | `AdapterInfo[]`   |

## Agents & settings

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `agent_spawn`           | `{ projectId, adapterId?, name?, prompt?, todoIds?, scratchpadIds?, worktree?, worktreeName?, args? }` | `ProcessInfo` |
| `agent_settings_get`    | –                                           | `AgentSettingsDto` |
| `agent_settings_set_adapter` | `{ adapterId, command?, defaultArgs }` | `AgentSettingsDto` |
| `agent_settings_set_default_adapter` | `{ adapterId? }` (blank clears) | `AgentSettingsDto` |
| `agent_settings_set_merge_mode` | `{ mode }`                       | `AgentSettingsDto` |
| `agent_settings_set_suggest_worktree` | `{ enabled }`             | `AgentSettingsDto` |
| `agent_settings_set_remember_sessions` | `{ enabled }`            | `AgentSettingsDto` |
| `agent_settings_set_remove_exited_after` | `{ minutes }` (0 = never, max 1440) | `AgentSettingsDto` (`removeExitedAfterMinutes`) |

## MCP

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `mcp_status`            | –                                           | `{ running, url? }` (token-free by design) |
| `mcp_clients_status`    | –                                           | `McpClientInfo[]` |
| `mcp_client_install`    | `{ clientId }`                              | `McpClientInfo[]` |

## To-dos

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `todo_list`             | `{ projectId }`                             | `TodoInfo[]`      |
| `todo_list_archived`    | `{ projectId }`                             | `TodoInfo[]`      |
| `todo_set_archived`     | `{ projectId, todoId, archived }`           | `TodoInfo`        |
| `todo_add`              | `{ projectId, text }`                       | `TodoInfo`        |
| `todo_set_done`         | `{ projectId, todoId, done }`               | `TodoInfo`        |
| `todo_update`           | `{ projectId, todoId, text?, description? }`| `TodoInfo`        |
| `todo_comment`          | `{ projectId, todoId, text, author? }`      | `TodoInfo`        |
| `todo_comment_update`   | `{ projectId, todoId, commentId, text }`    | `TodoInfo`        |
| `todo_comment_remove`   | `{ projectId, todoId, commentId }`          | `TodoInfo`        |
| `todo_add_link`         | `{ projectId, todoId, url, label? }`        | `TodoInfo`        |
| `todo_remove_link`      | `{ projectId, todoId, linkId }`             | `TodoInfo`        |
| `todo_remove`           | `{ projectId, todoId }`                     | –                 |
| `todo_unassign`         | `{ projectId, todoId }`                     | `TodoInfo`        |

## Scratchpads

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `scratchpad_list`       | `{ projectId }`                             | `ScratchpadInfo[]`|
| `scratchpad_list_archived` | `{ projectId }`                          | `ScratchpadInfo[]`|
| `scratchpad_add`        | `{ projectId }`                             | `ScratchpadInfo`  |
| `scratchpad_update_content` | `{ projectId, id, content, expectedUpdatedAt }` | `ScratchpadInfo` |
| `scratchpad_update_title`   | `{ projectId, id, title, expectedUpdatedAt }`   | `ScratchpadInfo` |
| `scratchpad_add_tag`    | `{ projectId, id, tag }`                    | `ScratchpadInfo`  |
| `scratchpad_remove_tag` | `{ projectId, id, tag }`                    | `ScratchpadInfo`  |
| `scratchpad_set_archived` | `{ projectId, id, archived }`             | `ScratchpadInfo`  |
| `scratchpad_remove`     | `{ projectId, id }`                         | –                 |
| `scratchpad_unassign`   | `{ projectId, id }`                         | `ScratchpadInfo`  |

> ⚠️ **Breaking change (Phase 4):** `scratchpad_update_content` and
> `scratchpad_update_title` — plus the matching `Orchestrator` methods
> (`update_scratchpad_content`/`update_scratchpad_title`) and the
> `update_scratchpad` MCP tool — now require `expectedUpdatedAt`
> (`expected_updated_at` for the MCP tool), the scratchpad's current
> `updatedAt` echoed back verbatim. A stale value fails with
> `IpcError.kind === "scratchpadConflict"` over Tauri, or an
> `invalid_params` MCP error whose message starts with `scratchpad
> conflict:`. Any caller written against Phase 1's 4-argument signature
> (frontend, MCP client, or another agent) must be updated to pass the
> timestamp through.

## Git board & issues

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `issues_board`          | `{ projectId, mine }`                       | `Board`           |
| `issues_board_cached`   | `{ projectId, mine }`                       | `CachedBoard \| null` (`{ board, fetchedAt }`, no network) |
| `issues_board_move`     | `{ projectId, kind: "issue"\|"pr", number, column }` | – (refusals: `issuesMove`; UI follows `board:changed`) |
| `issues_board_unassign` | `{ projectId, kind: "issue"\|"pr", number }` | – (removes the signed-in user from the assignees; refusals: `issuesMove`; UI follows `board:changed`) |
| `issues_settings_get`   | –                                           | `IssuesOverview` (`{ setups, projects }`, never a token) |
| `issues_setup_save`     | `{ id?, name, provider, url, account?, token? }` (token omitted = keep, blank = delete) | `IssuesOverview` |
| `issues_setup_remove`   | `{ id }`                                    | `IssuesOverview`  |
| `issues_project_setup_set` | `{ projectId, setupId? }` (omitted = match by host) | `IssuesOverview` |

## Worktrees

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `worktree_list`         | `{ projectId }`                             | `WorktreeInfo[]`  |
| `worktree_remove`       | `{ projectId, name, force }`                | `WorktreeInfo[]` (refreshed list) |
| `worktree_open_in_warp` | `{ projectId, name }`                       | –                 |

## Assistant

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `assistant_send`        | `{ text, attachments }` (`Attachment[]`)    | `bool` — delivered, not queued (reply streams as `assistant:event`) |
| `assistant_day_start`   | –                                           | `bool` — as `assistant_send` |
| `assistant_transcript`  | –                                           | `TranscriptEntry[]` |
| `assistant_new_conversation` | `{ stopSubagents }` (false = sub-agents keep running into the new conversation) | – |
| `assistant_subagents_list` | –                                         | `SubagentInfo[]` (oldest first; changes stream as `subagentsChanged`) |
| `assistant_subagent_stop` | `{ id }` (process id; ungated user stop, queued = cancelled; the assistant is told) | –   |
| `assistant_memory_get`  | –                                           | `string`          |
| `assistant_memory_set`  | `{ content }` (blank clears)                | –                 |
| `assistant_autonomy_get` | –                                          | `AutonomyMode`    |
| `assistant_autonomy_set` | `{ mode }`                                 | `AutonomyMode`    |
| `assistant_enabled_get` | –                                           | `boolean`         |
| `assistant_enabled_set` | `{ enabled }` (off: session killed quietly, sub-agents stopped, input refused with `assistantDisabled`) | `boolean` |
| `assistant_proactivity_get` | –                                       | `boolean`         |
| `assistant_proactivity_set` | `{ enabled }`                           | `boolean`         |
| `assistant_polling_get` | –                                           | `AssistantPolling` |
| `assistant_polling_set` | `{ intervalMinutes, slack, teams }` (0 = off) | `AssistantPolling` |
| `assistant_resolve_proposal` | `{ id, approved }`                     | – (outcome streams as `assistant:event`) |
| `assistant_queue_edit`  | `{ id, text }`                              | –                 |
| `assistant_queue_send_now` | `{ id }` (waiting user message)          | –                 |
| `assistant_queue_remove` | `{ id }`                                   | –                 |
| `assistant_interrupt`   | –                                           | – (queue drain streams as `assistant:event`) |
| `assistant_responses_list` | –                                        | `ResponseItem[]` (newest first) |
| `assistant_response_dismiss` | `{ id }` (unknown id: `responseNotFound`) | –              |
| `assistant_response_reply` | `{ id, text }`                           | `bool` — as `assistant_send` |

## Window

| Command                 | Args                                        | Returns           |
| ----------------------- | ------------------------------------------- | ----------------- |
| `window_confirm_close`  | –                                           | –                 |

`window_confirm_close` is the frontend's answer to the app-quit close guard:
when agents/terminals are still running the quit is intercepted and a warning
modal shown; confirming calls this command, which sets the force-close flag so
the next quit proceeds.

## NewProcess

`NewProcess` is `{ name, command?, cwd?, kind, adapter?, restartPolicy? }`
(`kind` is serde-flattened). `cwd` is **relative to the project root**; the
core rejects absolute paths and traversal out of the root. Terminals default
`command` to an interactive shell (`exec "$SHELL" -i`).

## Terminal streaming

`process_attach` is *snapshot + channel*:
one `{type:"snapshot", seq, dataB64}` (full scrollback; `seq` = first live
chunk after it), then batched `{type:"data", seq, dataB64}` (flushed every
16ms or at 64KiB). The frontend drops any batch whose `seq` is below the
snapshot's (no double-writes) and must **re-attach on `{type:"lagged"}`** —
a partial stream would corrupt the terminal. Payloads are base64 both ways
(`process_write` too) so raw bytes survive the JSON hop.

## Lifecycle events

Lifecycle events are global Tauri events (low-volume only; terminal
output never travels this path).

| Event | Payload |
| ----- | ------- |
| `process:added` / `process:removed` / `process:updated` | `{projectId, processId}` |
| `process:status` | `{projectId, processId, status}` |
| `project:opened` / `project:updated` / `project:closed` / `todo:changed` / `scratchpad:changed` / `worktree:changed` / `board:changed` | `{projectId}` |
| `assistant:event` | the assistant's chat stream — a bare, `type`-tagged `AssistantEvent` |

## Field casing

> ⚠️ **Field casing for the frontend:** everything on the wire is
> **camelCase** — DTO fields (`projectId`, `dataB64`, `configError`, …) and
> the serde tags: `ProcessKind` is tagged by `kind`
> (`service`/`terminal`/`agent {adapter}`), `ProcessStatus` by `state`
> (`notStarted`/`running {pid, since}`/`stopping`/`exited {code, crashed, stopped, …}`),
> `TermEvent` by `type` (`snapshot`/`data`/`lagged`). The one exception:
> `RestartPolicy` values are **kebab-case** (`never`/`on-crash`/`always`) —
> matching `podium.yml`'s `auto_restart` — and so are `MergeMode` and
> `AutonomyMode` (`full`/`confirm-destructive`/`propose-all`), which share the
> settings file with it. Match these exactly in the hand-written types in
> `src/ipc/types.ts`.
