# Built-in MCP server

- **Built-in MCP server** (`mcp/`): rmcp streamable-HTTP nested in axum on
  `127.0.0.1:0` (ephemeral port) behind a **per-run bearer token**. Tools:
  `list_projects`, `list_processes`, `get_process_status`,
  `get_process_output` (tail, ANSI-stripped, default 100 / max 2000 lines),
  `spawn_agent` (its `args` param sets the model/reasoning effort for that
  agent, e.g. `["--model", "opus"]`), `start_process`, `stop_process`,
  `restart_process`, `remove_process`,
  `list_todos` (active only; `include_archived: true` appends the archive,
  read after the listing's auto-archive so nothing drops out of both), `add_todo`, `complete_todo`, `update_todo`, `comment_todo`
  (its `author` is rejected server-side if it impersonates the user —
  "you"/"user"/"gebruiker", case/whitespace-insensitive — falling back to
  `agent`, so no MCP caller can make a comment read as if the user wrote it),
  `add_todo_link` (pin an issue/PR URL to the top of a to-do),
  `set_todo_archived`, `assign_todo` (a running agent self-assigns a to-do via its
  `PODIUM_PROCESS_ID`, so the user sees who owns it — blocked if the to-do is
  already owned by another live session),
  `rename_session` (a running agent renames itself via its
  `PODIUM_PROCESS_ID` to reflect what the session is about), the
  scratchpad tools `list_scratchpads`, `create_scratchpad`,
  `update_scratchpad`, `add_scratchpad_tag`, `remove_scratchpad_tag`,
  `set_scratchpad_archived`, the worktree tools `create_worktree`,
  `list_worktrees`, `remove_worktree`, `update_assistant_memory` (the
  personal assistant rewriting its own long-term memory), and
  `report_to_assistant` (a finished or blocked agent handing the assistant a
  short summary of its own work, routed to `Orchestrator::assistant_nudge` so
  it gets the same live-or-queued delivery, cap and proactivity switch as any
  other nudge), `get_board` / `move_board_item` (the Git board, see _Git board_),
  `verify_sender` (see _Cross-session messaging_), and
  `add_response` (the assistant filing a Responses item — see _Personal
  assistant_; only the assistant route lists or runs it), and
  `spawn_subagent` / `list_subagents` / `answer_subagent` (the assistant's
  sub-agents — see _Sub-agents_; assistant route only, like `add_response`),
  and `ask_assistant` (a sub-agent's question to the assistant; every route
  but the assistant's, run only for a valid sub-agent caller key): 37 tools
  on the assistant route, 34 on the agents' route, 11 on the lookup route. `spawn_subagent { kind, task, name?, args? }` is
  gated by its `kind` (`assistant::requires_proposal_for`): `lookup` never,
  `full` like `spawn_agent` (a missing/unknown kind too), and `args` may only
  be `--model`/`--effort` with a plain value (`subagent::validate_args`).
  `list_subagents` is read-only. `answer_subagent { process_id, answer }` is
  an ordinary state-changing tool (gated only under `propose-all`).
  `ask_assistant { question }` takes no process id: the caller is resolved
  from its key (an `RequestContext` argument, `PodiumTools::caller`), and a
  call without one is refused.
  The same surface is served a second time at **`/mcp-assistant`**
  (`mcp::ASSISTANT_PATH`, same bearer token) for the personal assistant: that
  route is its identity, and `PodiumTools::call_tool` puts calls arriving there
  through the autonomy gate (see _Personal assistant_). A third route,
  **`/mcp-lookup`** (`mcp::LOOKUP_PATH`, same token), is a *lookup*
  sub-agent's: `PodiumTools::new_lookup` lists and runs only
  `subagent::unprompted_podium_tools` (the read-only tools without
  `add_response`/`list_subagents`, plus `report_to_assistant` and
  `ask_assistant`), and
  `call_tool` refuses anything else before dispatch, so a lookup's "no writes"
  holds on the server, not only through the tool names the CLI hides.
  **Caller identity**: the bearer token is shared, so each sub-agent's MCP
  config also carries a per-sub-agent secret as `X-Podium-Caller`
  (`mcp::CALLER_HEADER`; `SubagentRecord.caller_key`, generated at spawn,
  resolved by `Orchestrator::subagent_for_caller` in constant time).
  `call_tool` (`refuse`, before the gate, so a refusal is never a proposal)
  refuses `report_to_assistant` for a sub-agent id unless the call carries
  that sub-agent's own key — on the lookup route any report but the caller's
  own — and, on the agents' and lookup routes, `stop_process` of a sub-agent by
  anyone but itself (the assistant's gated stop and the user's are the ways).
  Any other call carrying a sub-agent's key (bar `ask_assistant`) also counts
  as that sub-agent going on: a `waiting` one is `running` again
  (`Orchestrator::subagent_resumed`).
  The key lives in the sub-agent's 0600 config file under `{app_data_dir}/mcp`,
  which a lookup is denied reading; a project agent with a shell could read it,
  like the bearer token — the OS user stays the trust boundary.
  Spawned agents get the URL + token via 0600 per-agent config files under
  `{app_data_dir}/mcp` (wiped on every start); the same dir holds
  `server.json` (current URL + token, 0600) for the **stdio bridge**:
  external MCP clients configure `Podium mcp-bridge` (a subcommand of the
  app binary, `mcp/bridge.rs`) once — the bridge proxies stdio ↔ HTTP,
  re-reads `server.json`, and transparently replays the `initialize`
  handshake with backoff when the app restarts, so client config never goes
  stale even though port and token rotate every launch
  (`PODIUM_APP_DATA_DIR` overrides the app data dir). `mcp/install.rs`
  registers the bridge with external clients (Claude Code —
  `claude mcp add --scope user --transport stdio podium -- <exe>
  mcp-bridge` — and Auggie —
  `auggie mcp add podium --command <exe> --args mcp-bridge --replace`, its
  installed-check reading `auggie mcp list --json`; all via the login
  shell) — each surfaced in the Settings → MCP tab as a one-click Run/Copy
  card with an installed indicator.
