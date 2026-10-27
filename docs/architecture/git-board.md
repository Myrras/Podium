# Git board

- **Git board** (`issues.rs`): the project row's **Git** header button
  (`GitButton.tsx`, a pill at the header's far end — the one action visible
  at rest, so the hover-only ones reveal to its left without moving it — with
  the open-card count, an accent fill while the board is open and a warning
  dot on error; per-project section toggle "Git board", key `issues`, on by
  default) opens a Kanban of the repo behind the project's `origin`
  remote in the work area (`GitBoard.tsx`, `layoutStore.openBoard`, exclusive
  with the process/to-do/scratchpad). `issues::board` returns a `Board {
  provider, items, columns, source, hint, prCounts }`: issues and PRs/MRs as
  `BoardItem`s, each already in a `Column { id, title, color }` (`BoardItem.
  column` = a column id; `color` is a GitLab hex or a lowercase GitHub option
  name, which the UI maps to `--board-<name>` tokens). **Columns** come from
  the project's own board when the platform has one (`source` = its name),
  else the defaults `open` / `inProgress` (open issue with a linked open PR —
  GitHub `closingIssuesReferences`, GitLab `merge_requests_count > 0` — or an
  in-progress label, plus draft PRs/MRs) / `review` / `done` (closed/merged
  within `DONE_DAYS` = 14). Fixed system ids are reused either way: `open`
  (lead: unplaced issues), `review` (unplaced open PR/MR, shown only when
  non-empty with a project), `done`. *GitLab*: the `/boards` request (a 6th
  parallel call; first board with a label list only, lists by `position`,
  label-less lists skipped) gives `label:<name>` columns; an open card sits in
  the first list whose label it carries. Any `/boards` failure or no usable
  board → defaults, never an error. *GitHub*: a 6th `gh api graphql` call reads
  the repo's first Project v2 (first 100 items) and its `Status` options as
  `status:<optionId>` columns, items matched by number; it needs the
  `read:project` scope, and when stderr says it is missing the board falls back
  to defaults with `hint` = the `gh auth refresh -s read:project` command
  (stderr is only inspected). Any other failure there → defaults, no hint.
  `mine` (the
  default) narrows issues to assigned-to-me and PRs/MRs to authored-by-me;
  the counts (`prCounts`, all-time open/merged in the same scope) come from a
  GraphQL `search … issueCount` on GitHub and `X-Total` on GitLab (`None` when
  GitLab omits it). Each board is seven parallel calls (`std::thread::scope`),
  lists capped at 100. The repo is detected via `detect` → `parse_remote`
  (ssh/`ssh://`/https, `.git` stripped). Credentials come from **setups**
  (`AgentSettings.issues.setups`, Settings → Issues): each an `IssueSetup
  { id, name, provider: github|gitlab, url, account? }`, `url` normalized to
  `scheme://host[:port][/path]` with the scheme **kept** (`http://` for an
  instance without TLS; `https` when omitted) and the path kept minus a
  trailing slash (a GitLab under `/gitlab`); `gitlab_target` cuts that path
  off an HTTPS remote's project path (SSH remotes never carry it). A URL
  copied from the browser is cut at GitLab's `/-/` (and loses `?`/`#`) on
  save; a project path still left in it is not guessed away — the fetch's 404
  says to check the setup's URL. GitLab calls never follow a
  redirect (the token header would go along to the new host); a 3xx is a
  "check the setup's URL" error. A project can pin one setup from the
  board's picker (`issues.projects`, project root → setup id, app-side);
  otherwise `pick` takes the first setup whose host matches the remote, and a
  pinned setup for another host is `IssuesHostMismatch`. There is **no
  zero-config**: with no setup at all `board`/`move_item`/`unassign_item`
  fail with `IssuesNotConfigured` before spawning anything (not even the `git
  remote` lookup), and `issueStore.refresh` does not even call the backend
  then (the Git button shows no warning dot for it) — so a user who never set
  up a board never pays for `git`/`gh` calls on every refresh; github.com
  without a matching setup is `IssuesUnsupportedHost` like any other host.
  GitHub goes through the user's own `gh` login
  (`gh issue|pr list --repo host/owner/repo --search …`, `gh api graphql`,
  binary resolved once by `platform::gh_binary()` and run via
  `platform::gh_command()` + `run_gated` — in WSL-backend mode the distro's
  `gh` (`wsl.exe -e <path>`, `GH_TOKEN` passed through `WSLENV` by
  `platform::backend_env`), so probe and call never use different `gh`
  logins); a setup's `account` picks a
  logged-in `gh` account via `gh auth token --user`, handed to those `gh` calls
  as `GH_TOKEN` — Podium stores no GitHub token. Self-hosted GitLab is fetched
  from the setup's URL (`/api/v4/projects/:path/{issues,merge_requests}`,
  `PRIVATE-TOKEN`, blocking reqwest + rustls; closed/merged lists filtered by
  `updated_after`, then by the real `closed_at`/`merged_at`). **Temporary** —
  Connect still runs on GitLab; removing it is `IssueProvider::Gitlab`, its
  arm in `issues::board` and the `// ponytail: temporary` blocks. **Token
  handling:** a GitLab token lives only in the OS keychain (`keyring`, service
  `com.podium.app.gitlab`, account = setup id; on Linux the Secret Service
  over D-Bus, so building there needs `libdbus-1-dev`); `issues_settings_get` reports
  just `tokenSet`, the token is never returned over IPC, logged, or put in an
  error, and the Settings field is never prefilled. Failures are typed
  (`IssuesNotConfigured` / `IssuesNoRemote` / `IssuesGhUnavailable` / `IssuesUnsupportedHost` /
  `IssuesHostMismatch` / `IssuesGitlabNotConfigured` / `IssuesGitlabAuth` /
  `IssuesFetch`, the last also for rate limits) and never carry `gh`/`git`
  stderr or a GitLab response body; the setup kinds link the board to Settings
  → Issues (`settingsRequestStore`), as does its gear. Refresh is a frontend
  concern (`issueStore`, which also caches the setups overview and refetches
  every shown project when it changes, and holds the per-project Mine/All
  scope — a switch mid-fetch refetches once the old answer lands): one
  in-flight fetch per project, the stale board is kept on error, and
  `startAutoRefresh` (mounted with the Git button, so its "N open" —
  every card not in `done` — tooltip stays live) fetches immediately, every 5 min, and on window focus
  when the last attempt is over 60 s old; errors show inline, never as a
  toast. Those automatic fetches share one pace across projects — starts at
  least 15 s apart (`AUTO_FETCH_SPACING_MS`), since a GitHub board costs ~6
  of GitHub's 30 searches a minute — while a refresh the user asks for skips
  it. A queued auto refresh is not `loading` (it waits *before* that), so a
  manual refresh still runs and the queued one lapses; closing the project
  drops it, and a project stuck on a local error (no remote, unsupported
  host, …) takes no slot, except on a setups change, which paces every shown
  project. A setups change during a fetch reruns it once that fetch is done,
  and a section shown again refetches when its cache predates the last setups
  change. A card click opens a read-only drawer (`IssueDetailPane`) beside the
  columns; browser, "start agent" (`NewAgentModal` prefilled via
  `initialPrompt`) and "turn into to-do" (`todoAdd` → `todoUpdate` →
  `todoAddLink`, hidden while an active to-do already links the URL) are card
  actions. GitLab MRs are referenced as `!n`, everything else `#n`.

  **Cache** (`boards.rs`, `BoardStore`, like `TodoStore`): `Orchestrator::
  issue_board` stores every successful board in one `boards.json` (app data
  dir, atomic writes — serialised under the store lock, written after it is
  released, an older snapshot never over a newer one — keyed by project
  root, per scope `mine`/`all`) as a
  `CachedBoard { board, fetchedAt }`; `cached_issue_board` /
  `issues_board_cached` read it without network, so the UI paints at once. The
  cache also keeps **columns stable**: when GitLab's `/boards` request *fails*
  (not: answers without a label-list board) `fetch_gitlab` places cards in the
  cached board's `label:` columns instead of the defaults. Project boards
  always carry `review` and `done`. A fetch that started before a move or
  unassign of that project landed is not stored (`BoardStore`'s per-root edit
  counter, taken before the fetch); `issue_board` then serves the cache, which
  has the edit, with its real `fetchedAt` (what `get_board` reports).
  Closed issues and merged PRs are always in `done`, also on a GitHub Project
  with a Status set, so `done` means closed on both providers.

  **Drop rules** ride on the wire: `Column.accepts` (`("issue"|"pr")[]`) —
  default board `open` [issue], `inProgress` [], `review` [], `done` [issue];
  project board `open` [issue], every `label:*`/`status:*` [issue, pr],
  `review` [pr], `done` [issue] — and `BoardItem.movable` (false for a PR/MR in
  `done`). **PRs/MRs that close an issue on the same board are folded**: the
  card is dropped and rides on the issue as `BoardItem.linked: LinkedPr[]`
  (`{ number, title, url, draft, merged }`); a PR whose issue is not on the
  board stays a card. GitHub uses `closingIssuesReferences`; GitLab makes no
  extra calls and matches the MR description against the default closing
  pattern (`closes|fixes|resolves|implements` + `#N` / `group/project#N`) plus
  a source-branch segment that is `N`, optionally one letter, then `-…` or
  nothing (`gl_closing`, a heuristic — upgrade path
  `/merge_requests/:iid/closes_issues`).

  **Moves** (`Orchestrator::move_board_item` → `issues::validate_move` then
  `move_item`, blocking): validated against the cached board — of the scopes
  holding the card, the most recently fetched, since its `done` decides
  close/reopen (item there, column exists and accepts the kind, movable;
  without a project board `open` is only reachable from `done`; already in
  that column is a no-op success that syncs both scopes), refusals are
  `CoreError::IssuesMove` (`issuesMove`, Podium-owned text). GitLab: one `PUT`
  on the issue/MR — `add_labels`/`remove_labels` for `label:*` (other list
  labels removed), all list labels removed for `open`/`review`,
  `state_event=close` for `done`, plus `state_event=reopen` out of `done`.
  GitHub: `gh issue close|reopen`; on a Project, GraphQL looks up the first
  project's Status field and the card's item, adds it when missing, then
  `updateProjectV2ItemFieldValue` (or `clearProjectV2ItemFieldValue` for
  `open`/`review`); a missing token scope becomes the `gh auth refresh -s
  project` message. On success both cached scopes are updated (`column`,
  GitLab `labels`; `linked` untouched) and `PodiumEvent::BoardChanged` →
  `board:changed`. MCP: `get_board` (cache if under 5 min old and not
  `refresh`; no bodies; classified read-only) and `move_board_item` (same
  path as the IPC command; **destructive** for the autonomy gate, since it
  closes/reopens issues other people see).

  **Unassign** (`Orchestrator::unassign_board_item` → `issues::
  validate_unassign` then `unassign_item`): each board also carries `me` (the
  signed-in login — a 7th parallel call, `gh api user` / GitLab `/user`, whose
  failure only hides the action) and every `BoardItem.assignees`; the card and
  drawer show "Unassign me" when `me` is among them. GitHub: `gh api --method
  DELETE repos/:repo/issues/:n/assignees` (PRs too). GitLab: a fresh `GET` of
  the issue/MR, then a `PUT` with the remaining `assignee_ids[]` (`0` = none),
  so a co-assignee added since the fetch is kept. On success `me` leaves the
  item's assignees in both cached scopes, an *issue* leaves the `mine` board
  outright, and `board:changed` is published. IPC only, no MCP tool.

  **Approvals**: open PRs/MRs (cards and linked pills) carry `approvals:
  { by, required }`, shown as a `✓ n/required` badge (`approvalBadge`,
  accented when `me` is among `by`). GitHub: `latestReviews` in state
  `APPROVED` (`required` unknown → `None`). GitLab: one GraphQL call after the
  lists (`/api/graphql`, same `PRIVATE-TOKEN`, `mergeRequests(iids:)` →
  `approvedBy` + `approvalsRequired`), whose failure only hides the badges.
  The **search** field (Cmd/Ctrl+F focuses it, Escape clears) filters every
  column by ref, title, labels, author, assignees and linked PRs.
  MR ↔ issue linking also takes a branch segment that is the issue number
  (`issue/12661`, `issue/12661b`, `42-fix`), Connect's convention — not
  `2fa-login`, `release/2026.10` or `hotfix/3.4.1`.

  **GitLab connection retries** (`gitlab_send`): a request that fails before
  any HTTP status (VPN/Wi-Fi not back yet after sleep — the focus refresh
  fires right then — or a dropped connection) is retried twice (500 ms, 1.5 s);
  the error names the reason (`timed out` / `connection failed`), never
  reqwest's text. An HTTP status is never retried.
