# Cross-session messaging

- **Cross-session messaging**: Podium's own sessions — the assistant and
  every spawned agent — can reach each other with Claude Code's `SendMessage`
  / `ListAgents`. Two things make that work, both set per session so the
  user's own `~/.claude/settings.json` is never touched:
  - **`crossSessionInbound: "accept"`**, passed inline via `--settings`
    (`agent::peer_inbound_settings`). Without it the CLI decides per message
    from the two sessions' permission-mode classes, and the assistant
    (`bypassPermissions`) never matches an agent (prompting): a message to an
    agent is held for approval, and one to the assistant expires after
    `dialogExpiry` because a `-p` session has no TTY to show the dialog on.
    A `--settings` the user put in `extra_args` wins over Podium's. The
    assistant's sub-agents get neither this nor the rules below: the assistant
    never messages them, and they are denied `SendMessage`.
  - **Go-ahead rules.** A Claude Code agent's `--settings`
    (`agent::agent_session_settings`, built on the same inbound setting)
    also allows `git push`, `gh pr create` and `glab mr create`, so a
    go-ahead the assistant relays by message can run in the existing
    session — no peer message can approve a prompt. `GO_AHEAD_DENY_RULES`
    (deny beats allow) block force, `+refspec`, delete, mirror/prune/all,
    `--receive-pack`/`--exec`, submodules, push options, `--no-verify`, tag
    pushes, URL/`@` destinations, main/master as the destination and any
    quoting, `$`/backtick/brace expansion or a literal `*` (glob refspec)
    in a push, any of the three fed through `xargs` (a prefix allow also
    matches `xargs <prefix> …`, with the arguments out of sight on stdin),
    plus `-R`/`--repo` on `gh`/`glab`, wherever the flag sits (mid-pattern
    `*`; long git options cut to their shortest unambiguous prefix), and
    every way to read a local file into the PR/MR — `gh --body-file`/`-F`,
    `glab --description-file`/`--attach`, `gh --attach` — which would
    publish e.g. an ssh key without a prompt (the prompt's retry is
    `--fill`, or `--body` inline), plus glab's other-repository flags
    `-H`/`--head` and `--target-p…` (not in its docs; denied anyway), and
    glab's `-s`/`--source-branch` (also quoted/escaped): glab
    `--fill`/`--push` pushes `HEAD:<source-branch>` itself, around the git
    push rules. They
    match command text only:
    `git -C … push`, aliases, some flag clusters, an scp-style `host:path`
    destination with no user (`example.com:other.git`, an ssh-config
    alias), an explicit push to another non-main branch (`HEAD:develop`)
    and an implicit push of a checked-out
    main slip past (see the code comment); the identity prompt has the agent
    check it is not on main/master before pushing. The test's matcher
    mirrors CLI 2.1.292 — re-check it on a CLI upgrade. With the user's own
    `--settings` (either form) none of this applies, and the identity
    prompt drops the go-ahead. The assistant's fresh fallback agent is a
    `claude-code` spawn with no `--allowedTools`: Podium's settings already
    carry the allow and deny rules, and with the user's own `--settings` an
    `--allowedTools` would grant a push with no deny list behind it. When
    those `--settings` come from the default agent args or a project's
    `agents.extra_args` (merged in by `spawn_agent`), the fallback has no
    rules either: the push waits for approval in its terminal, and the
    assistant is told to say so.
  - **A Podium-stamped session name**, `podium-<tag>` where the tag is the
    first 8 hex of the session's id (`-n`, `agent::peer_session_name`). The
    assistant's tag is a random per-app-run UUID prefix, not a fixed literal
    any local session could name itself. Sender verification is
    `Orchestrator::verify_peer_sender`, exposed as the **`verify_sender`**
    MCP tool: it resolves a sender name against Podium's own process registry
    (live agent sessions) plus that tag, and answers `fleet: false` for
    everything else. It is deliberately **not** a secret the model compares —
    a value in the model's context can be leaked or argued about by a
    prompt-injected message; the authority here is Podium's Rust layer,
    reached over the bearer-token-authenticated MCP channel, and nothing the
    message asserts about its own origin is consulted. `verify_sender` is
    classified read-only so the autonomy gate can never park a security check
    as a proposal. Note what this is *not*: inbound peer messages arrive on
    the CLI's own inbox socket (`/tmp/cc-socks`, owner-only), never through
    Podium, so Podium cannot filter them before the model sees them —
    the OS-user boundary remains the real trust boundary, and `verify_sender`
    is what tells Podium's fleet apart inside it.
