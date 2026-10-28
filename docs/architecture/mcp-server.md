# Built-in MCP server

The built-in MCP server (`mcp/`) is how agents inspect and orchestrate what
Podium manages. It is rmcp streamable-HTTP nested in axum on `127.0.0.1:0`
(ephemeral port) behind a **per-run bearer token**.

## Tools

37 tools on the assistant route, 34 on the agents' route, 11 on the lookup
route (see [Routes](#routes)).

### Projects and processes

| Tool | Meaning |
| ---- | ------- |
| `list_projects` | List projects. |
| `list_processes` | List processes. |
| `get_process_status` | Status of one process. |
| `get_process_output` | Tail, ANSI-stripped, default 100 / max 2000 lines. |
| `spawn_agent` | Its `args` param sets the model/reasoning effort for that agent, e.g. `["--model", "opus"]`. |
| `start_process` | Start a process. |
| `stop_process` | Stop a process. |
| `restart_process` | Restart a process. |
| `remove_process` | Remove a process. |

### To-dos and session identity

| Tool | Meaning |
| ---- | ------- |
| `list_todos` | Active only. `include_archived: true` appends the archive, read after the listing's auto-archive so nothing drops out of both. |
| `add_todo` | Add a to-do. |
| `complete_todo` | Complete a to-do. |
| `update_todo` | Update a to-do. |
| `comment_todo` | Its `author` is rejected server-side if it impersonates the user (`"you"`/`"user"`/`"gebruiker"`, case/whitespace-insensitive), falling back to `agent`, so no MCP caller can make a comment read as if the user wrote it. |
| `add_todo_link` | Pin an issue/PR URL to the top of a to-do. |
| `set_todo_archived` | Archive or unarchive a to-do. |
| `assign_todo` | A running agent self-assigns a to-do via its `PODIUM_PROCESS_ID`, so the user sees who owns it. Blocked if the to-do is already owned by another live session. |
| `rename_session` | A running agent renames itself via its `PODIUM_PROCESS_ID` to reflect what the session is about. |

### Scratchpads and worktrees

| Tool | Meaning |
| ---- | ------- |
| `list_scratchpads` | List scratchpads. |
| `create_scratchpad` | Create a scratchpad. |
| `update_scratchpad` | Update a scratchpad. |
| `add_scratchpad_tag` | Add a tag. |
| `remove_scratchpad_tag` | Remove a tag. |
| `set_scratchpad_archived` | Archive or unarchive a scratchpad. |
| `create_worktree` | Create a worktree. |
| `list_worktrees` | List worktrees. |
| `remove_worktree` | Remove a worktree. |

### Git board

| Tool | Meaning |
| ---- | ------- |
| `get_board` | Read the Git board. See [Git board](git-board.md). |
| `move_board_item` | Move a card. See [Git board](git-board.md). |

### Assistant and sub-agents

| Tool | Meaning |
| ---- | ------- |
| `update_assistant_memory` | The personal assistant rewriting its own long-term memory. |
| `report_to_assistant` | A finished or blocked agent handing the assistant a short summary of its own work, routed to `Orchestrator::assistant_nudge` so it gets the same live-or-queued delivery, cap and proactivity switch as any other nudge. |
| `verify_sender` | See [Cross-session messaging](cross-session-messaging.md). |
| `add_response` | The assistant filing a Responses item (see [Personal assistant](assistant.md)). Only the assistant route lists or runs it. |
| `spawn_subagent` | The assistant's sub-agents (see [Assistant sub-agents](assistant-subagents.md)). Assistant route only, like `add_response`. |
| `list_subagents` | Same. Read-only. |
| `answer_subagent` | Same. |
| `ask_assistant` | A sub-agent's question to the assistant. Every route but the assistant's, run only for a valid sub-agent caller key. |

### Sub-agent tool details

`spawn_subagent { kind, task, name?, args? }` is gated by its `kind`
(`assistant::requires_proposal_for`): `lookup` never, `full` like
`spawn_agent` (a missing/unknown kind too). `args` may only be
`--model`/`--effort` with a plain value (`subagent::validate_args`).

`list_subagents` is read-only. `answer_subagent { process_id, answer }` is an
ordinary state-changing tool (gated only under `propose-all`).

`ask_assistant { question }` takes no process id: the caller is resolved from
its key (an `RequestContext` argument, `PodiumTools::caller`), and a call
without one is refused.

## Routes

The same surface is served on three routes, all behind the same bearer token.

| Route | For | Behaviour |
| ----- | --- | --------- |
| agents' route | spawned agents | 34 tools. |
| `/mcp-assistant` (`mcp::ASSISTANT_PATH`) | the personal assistant | 37 tools. That route is its identity, and `PodiumTools::call_tool` puts calls arriving there through the autonomy gate (see [Personal assistant](assistant.md)). |
| `/mcp-lookup` (`mcp::LOOKUP_PATH`) | a *lookup* sub-agent | 11 tools. `PodiumTools::new_lookup` lists and runs only `subagent::unprompted_podium_tools` (the read-only tools without `add_response`/`list_subagents`, plus `report_to_assistant` and `ask_assistant`). |

On the lookup route `call_tool` refuses anything else before dispatch, so a
lookup's "no writes" holds on the server, not only through the tool names the
CLI hides.

## Caller identity

The bearer token is shared, so each sub-agent's MCP config also carries a
per-sub-agent secret as `X-Podium-Caller` (`mcp::CALLER_HEADER`;
`SubagentRecord.caller_key`, generated at spawn, resolved by
`Orchestrator::subagent_for_caller` in constant time).

`call_tool` (`refuse`, before the gate, so a refusal is never a proposal)
refuses:

- `report_to_assistant` for a sub-agent id unless the call carries that
  sub-agent's own key. On the lookup route any report but the caller's own.
- On the agents' and lookup routes, `stop_process` of a sub-agent by anyone
  but itself (the assistant's gated stop and the user's are the ways).

Any other call carrying a sub-agent's key (bar `ask_assistant`) also counts as
that sub-agent going on: a `waiting` one is `running` again
(`Orchestrator::subagent_resumed`).

The key lives in the sub-agent's 0600 config file under
`{app_data_dir}/mcp`, which a lookup is denied reading. A project agent with a
shell could read it, like the bearer token. The OS user stays the trust
boundary.

## Config files and the stdio bridge

Spawned agents get the URL + token via 0600 per-agent config files under
`{app_data_dir}/mcp` (wiped on every start). The same dir holds `server.json`
(current URL + token, 0600) for the **stdio bridge**.

External MCP clients configure `Podium mcp-bridge` (a subcommand of the app
binary, `mcp/bridge.rs`) once. The bridge proxies stdio ↔ HTTP, re-reads
`server.json`, and transparently replays the `initialize` handshake with
backoff when the app restarts, so client config never goes stale even though
port and token rotate every launch. `PODIUM_APP_DATA_DIR` overrides the app
data dir.

## Client install

`mcp/install.rs` registers the bridge with external clients, all via the login
shell:

- Claude Code: `claude mcp add --scope user --transport stdio podium -- <exe>
  mcp-bridge`.
- Auggie: `auggie mcp add podium --command <exe> --args mcp-bridge
  --replace`, its installed-check reading `auggie mcp list --json`.

Each is surfaced in the Settings → MCP tab as a one-click Run/Copy card with
an installed indicator.
