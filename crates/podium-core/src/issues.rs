//! The Git board: a Kanban view of a project's repo — issues and PRs/MRs in
//! the project's own board columns when the platform has them (GitLab board
//! label lists, a GitHub Project's Status field), else four default columns
//! (open / in progress / in review / done) — plus all-time PR/MR counts. The repo is auto-detected from the git `origin` remote; which
//! credentials fetch it is an [`IssueSetup`] from Settings → Issues (picked
//! per project, or matched by host). GitHub goes through the user's own `gh`
//! login, optionally a specific `gh` account (Podium stores no GitHub token).
//!
//! Self-hosted GitLab (temporary, see [`fetch_gitlab`]) uses a personal access
//! token kept only in the OS keychain, per setup: it is never returned, logged
//! or put in an error message.
//!
//! Everything here shells out or does blocking I/O, so call it from a blocking-friendly context.
//! Error messages are Podium-owned text — `gh`/`git` output is never captured
//! into them.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::ScopedJoinHandle;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// How far back the Done column reaches.
pub const DONE_DAYS: i64 = 14;

/// Per-list fetch cap; the counts are exact regardless.
const LIMIT: &str = "100";

/// One Kanban column. The ids `open` (lead: nothing placed), `review` (open
/// PR/MR not placed) and `done` (closed/merged in [`DONE_DAYS`]) are Podium's
/// own; project columns are `label:<name>` (GitLab) / `status:<optionId>`
/// (GitHub).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub title: String,
    /// A hex string (GitLab) or a lowercase GitHub option colour name.
    pub color: Option<String>,
}

fn col(id: &str, title: &str, color: Option<String>) -> Column {
    Column {
        id: id.to_string(),
        title: title.to_string(),
        color,
    }
}

fn default_columns() -> Vec<Column> {
    vec![
        col("open", "Open", None),
        col("inProgress", "In progress", None),
        col("review", "In review", None),
        col("done", "Done (14d)", None),
    ]
}

/// Lead column, the project's own columns, then `review` (only when a card
/// landed there) and `done` (`always_done`, or when non-empty).
fn project_columns(
    open_title: &str,
    mid: Vec<Column>,
    done_title: &str,
    always_done: bool,
    items: &[BoardItem],
) -> Vec<Column> {
    let has = |id: &str| items.iter().any(|i| i.column == id);
    let mut cols = vec![col("open", open_title, None)];
    cols.extend(mid);
    if has("review") {
        cols.push(col("review", "In review", None));
    }
    if always_done || has("done") {
        cols.push(col("done", done_title, None));
    }
    cols
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Issue,
    /// A GitHub PR or a GitLab MR.
    Pr,
}

/// One card on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardItem {
    pub kind: ItemKind,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub labels: Vec<String>,
    pub updated_at: String,
    /// A [`Column::id`] of the board.
    pub column: String,
    pub draft: bool,
    pub author: String,
}

/// All-time PR/MR totals in the board's scope; `None` when the platform
/// would not say (GitLab omits the total past 10k).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct PrCounts {
    pub open: Option<u64>,
    pub merged: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    /// Which platform answered, so the UI can say PR vs MR (`#n` vs `!n`).
    pub provider: IssueProvider,
    pub items: Vec<BoardItem>,
    /// Display order.
    pub columns: Vec<Column>,
    /// Name of the project board the columns come from; `None` = Podium's defaults.
    pub source: Option<String>,
    /// Podium-owned text: how to unlock the project's columns.
    pub hint: Option<String>,
    pub pr_counts: PrCounts,
}

/// Label names that mark an open issue as being worked on (`In progress`,
/// `in-progress`, `workflow::In Progress`, `doing`, `wip`).
fn in_progress_label(labels: &[String]) -> bool {
    labels.iter().any(|l| {
        let l = l.to_ascii_lowercase().replace(['-', '_', ' '], "");
        let tail = l.rsplit("::").next().unwrap_or(&l);
        tail.contains("inprogress") || tail == "doing" || tail == "wip"
    })
}

fn issue_column(labels: &[String], linked: bool) -> &'static str {
    if linked || in_progress_label(labels) {
        "inProgress"
    } else {
        "open"
    }
}

fn pr_column(draft: bool) -> &'static str {
    if draft {
        "inProgress"
    } else {
        "review"
    }
}

/// Newest first, the order every column shows.
fn sorted(mut items: Vec<BoardItem>) -> Vec<BoardItem> {
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    items
}

/// A panicked fetch thread becomes a plain fetch error.
fn join<T>(h: ScopedJoinHandle<'_, CoreResult<T>>) -> CoreResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(CoreError::IssuesFetch("fetch failed".to_string())))
}

/// Split a remote URL into `(host, path)` with `.git` and slashes stripped.
/// Handles `git@host:a/b.git`, `ssh://git@host[:port]/a/b.git` and
/// `https://host/a/b(.git)`; anything else is `None`.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url
        .strip_prefix("ssh://")
        .or_else(|| url.strip_prefix("https://"))
        .or_else(|| url.strip_prefix("http://"))
    {
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?;
        // Drop a port (`host:2222`).
        (authority.split(':').next()?, path)
    } else {
        // scp-like: `user@host:path`.
        let (authority, path) = url.split_once(':')?;
        if path.starts_with('/') && url.contains("://") {
            return None;
        }
        (authority.rsplit('@').next()?, path)
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || host.contains(char::is_whitespace) || !path.contains('/') {
        return None;
    }
    Some((host.to_ascii_lowercase(), path.to_string()))
}

/// `(host, path)` of the project's `origin` remote.
pub fn detect(root: &Path) -> CoreResult<(String, String)> {
    let output = Command::new(crate::platform::git_binary())
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CoreError::IssuesNoRemote)?;
    if !output.status.success() {
        return Err(CoreError::IssuesNoRemote);
    }
    parse_remote(&String::from_utf8_lossy(&output.stdout)).ok_or(CoreError::IssuesNoRemote)
}

// ---------------------------------------------------------------------------
// GitHub (via `gh`)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhItem {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    author: Option<GhUser>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    closing_issues_references: Vec<GhRef>,
}

#[derive(Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Deserialize)]
struct GhUser {
    login: String,
}

#[derive(Deserialize)]
struct GhRef {
    number: u64,
}

impl GhItem {
    fn into_item(self, kind: ItemKind, column: &str) -> BoardItem {
        BoardItem {
            kind,
            number: self.number,
            title: self.title,
            url: self.url,
            body: self.body,
            labels: self.labels.into_iter().map(|l| l.name).collect(),
            updated_at: self.updated_at,
            column: column.to_string(),
            draft: self.is_draft,
            author: self.author.map(|a| a.login).unwrap_or_default(),
        }
    }
}

fn parse_gh(json: &str) -> CoreResult<Vec<GhItem>> {
    serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesFetch("unexpected response from gh".to_string()))
}

/// A GitHub Project's Status field: its options as columns plus the option
/// each item (by issue/PR number, one number space per repo) is set to.
#[derive(Debug, PartialEq, Eq)]
struct GhProject {
    title: String,
    options: Vec<Column>,
    /// Number → column id (`status:<optionId>`), only for known options.
    status: std::collections::HashMap<u64, String>,
}

/// The first project of the `projectsV2` GraphQL answer; `None` without a
/// project or a Status field. A project can span repos, so only items of
/// `owner_repo` count — another repo's #7 is not this one's.
fn parse_gh_project(json: &str, owner_repo: &str) -> Option<GhProject> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let node = v["data"]["repository"]["projectsV2"]["nodes"].get(0)?;
    let options: Vec<Column> = node["field"]["options"]
        .as_array()?
        .iter()
        .filter_map(|o| {
            Some(col(
                &format!("status:{}", o["id"].as_str()?),
                o["name"].as_str()?,
                o["color"].as_str().map(str::to_lowercase),
            ))
        })
        .collect();
    if options.is_empty() {
        return None;
    }
    let status = node["items"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|it| {
            let repo = it["content"]["repository"]["nameWithOwner"].as_str()?;
            if !repo.eq_ignore_ascii_case(owner_repo) {
                return None;
            }
            let number = it["content"]["number"].as_u64()?;
            let id = format!("status:{}", it["fieldValueByName"]["optionId"].as_str()?);
            options.iter().any(|o| o.id == id).then_some((number, id))
        })
        .collect();
    Some(GhProject {
        title: node["title"].as_str().unwrap_or("Project").to_string(),
        options,
        status,
    })
}

/// Build the items from the four `gh … list --json` outputs. With a
/// `project`, an item with a Status sits in that column; the rest fall back to
/// open issue → `open`, open PR → `review`, closed/merged → `done`. Without
/// one, the default placement (linked PR, label, draft) applies.
fn assemble_github(
    open_issues: &str,
    closed_issues: &str,
    open_prs: &str,
    merged_prs: &str,
    project: Option<&GhProject>,
) -> CoreResult<Vec<BoardItem>> {
    let open_prs = parse_gh(open_prs)?;
    let linked: HashSet<u64> = open_prs
        .iter()
        .flat_map(|p| p.closing_issues_references.iter().map(|r| r.number))
        .collect();
    let place = |number: u64, default: &str| {
        project
            .and_then(|p| p.status.get(&number))
            .map_or(default, String::as_str)
            .to_string()
    };
    let mut items = Vec::new();
    for i in parse_gh(open_issues)? {
        let default = match project {
            Some(_) => "open",
            None => issue_column(
                &i.labels.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
                linked.contains(&i.number),
            ),
        };
        let column = place(i.number, default);
        items.push(i.into_item(ItemKind::Issue, &column));
    }
    for i in parse_gh(closed_issues)? {
        let column = place(i.number, "done");
        items.push(i.into_item(ItemKind::Issue, &column));
    }
    for p in open_prs {
        let default = if project.is_some() {
            "review"
        } else {
            pr_column(p.is_draft)
        };
        let column = place(p.number, default);
        items.push(p.into_item(ItemKind::Pr, &column));
    }
    for p in parse_gh(merged_prs)? {
        let column = place(p.number, "done");
        items.push(p.into_item(ItemKind::Pr, &column));
    }
    Ok(sorted(items))
}

/// `data.o.issueCount` / `data.m.issueCount` from the counts GraphQL query.
fn parse_gh_counts(json: &str) -> CoreResult<PrCounts> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesFetch("unexpected response from gh".to_string()))?;
    Ok(PrCounts {
        open: v["data"]["o"]["issueCount"].as_u64(),
        merged: v["data"]["m"]["issueCount"].as_u64(),
    })
}

/// The token of a specific `gh` account on `host`. `gh` already holds it, so
/// this only picks which one — Podium never stores it.
fn gh_account_token(gh: &str, host: &str, account: &str) -> CoreResult<String> {
    let output = Command::new(gh)
        .args(["auth", "token", "--hostname", host, "--user", account])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CoreError::IssuesGhUnavailable)?;
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || token.is_empty() {
        return Err(CoreError::IssuesGhUnavailable);
    }
    Ok(token)
}

/// Run one `gh` call; `token` (a picked account's) goes in via env only.
fn run_gh(gh: &str, host: &str, token: Option<&str>, args: &[&str]) -> CoreResult<String> {
    run_gh_scoped(gh, host, token, args).map_err(|(e, _)| e)
}

/// [`run_gh`] that also says whether the failure was a missing token scope
/// (`read:project`), read from stderr — which is only inspected, never kept.
fn run_gh_scoped(
    gh: &str,
    host: &str,
    token: Option<&str>,
    args: &[&str],
) -> Result<String, (CoreError, bool)> {
    let mut cmd = Command::new(gh);
    cmd.args(args).stdin(Stdio::null());
    if let Some(token) = token {
        let var = if host == "github.com" {
            "GH_TOKEN"
        } else {
            "GH_ENTERPRISE_TOKEN"
        };
        cmd.env(var, token);
    }
    let output = cmd.output().map_err(|e| {
        (
            match e.kind() {
                std::io::ErrorKind::NotFound => CoreError::IssuesGhUnavailable,
                _ => CoreError::IssuesFetch("could not run gh".to_string()),
            },
            false,
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        // stderr is only inspected, never forwarded.
        let scope = stderr.contains("read:project") || stderr.contains("insufficient_scopes");
        return Err((
            if stderr.contains("auth login") || stderr.contains("not logged") {
                CoreError::IssuesGhUnavailable
            } else if stderr.contains("rate limit") {
                CoreError::IssuesFetch("GitHub rate limit reached, try again later".to_string())
            } else {
                CoreError::IssuesFetch("gh could not fetch the board".to_string())
            },
            scope,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

const GH_ISSUE_FIELDS: &str = "number,title,url,body,labels,updatedAt,author";
const GH_PR_FIELDS: &str =
    "number,title,url,body,labels,updatedAt,author,isDraft,closingIssuesReferences";
const GH_COUNTS_QUERY: &str = "query($o:String!,$m:String!){\
o:search(query:$o,type:ISSUE,first:0){issueCount} \
m:search(query:$m,type:ISSUE,first:0){issueCount}}";

const GH_PROJECT_QUERY: &str = "query($owner:String!,$name:String!){\
repository(owner:$owner,name:$name){projectsV2(first:1){nodes{title \
field(name:\"Status\"){... on ProjectV2SingleSelectField{options{id name color}}} \
items(first:100){nodes{content{... on Issue{number repository{nameWithOwner}} \
... on PullRequest{number repository{nameWithOwner}}} \
fieldValueByName(name:\"Status\"){... on ProjectV2ItemFieldSingleSelectValue{optionId}}}}}}}}";

const GH_SCOPE_HINT: &str =
    "To use your GitHub Project's columns, run: gh auth refresh -s read:project";

/// The board for `owner_repo` on `host`, via the `gh` binary at `gh`
/// (injectable so tests can point it at a fake script). `mine` narrows it to
/// issues assigned to and PRs authored by the user; `account` picks a
/// logged-in `gh` account, `None` uses the active one. The six `gh` calls
/// run in parallel; the sixth (the repo's first GitHub Project, for its
/// columns) never fails the board.
// ponytail: first project and first 100 items only; ceiling + upgrade path =
// pagination / a project picker.
pub fn fetch_github(
    gh: &str,
    host: &str,
    owner_repo: &str,
    account: Option<&str>,
    mine: bool,
    since: DateTime<Utc>,
) -> CoreResult<Board> {
    let token = account.map(|a| gh_account_token(gh, host, a)).transpose()?;
    let token = token.as_deref();
    let repo = format!("{host}/{owner_repo}");
    let day = since.format("%Y-%m-%d");
    let (assignee, author) = if mine {
        (" assignee:@me", " author:@me")
    } else {
        ("", "")
    };
    let list = |kind: &str, state: &str, search: String, fields: &str| {
        run_gh(
            gh,
            host,
            token,
            &[
                kind, "list", "--repo", &repo, "--state", state, "--limit", LIMIT, "--search",
                &search, "--json", fields,
            ],
        )
    };
    let count_scope = format!("repo:{owner_repo} is:pr{author}");
    let (owner, name) = owner_repo.split_once('/').unwrap_or((owner_repo, ""));
    let (open_issues, closed_issues, open_prs, merged_prs, counts, project) =
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                list(
                    "issue",
                    "open",
                    format!("is:open{assignee}"),
                    GH_ISSUE_FIELDS,
                )
            });
            let b = s.spawn(|| {
                list(
                    "issue",
                    "closed",
                    format!("closed:>={day}{assignee}"),
                    GH_ISSUE_FIELDS,
                )
            });
            let c = s.spawn(|| list("pr", "open", format!("is:open{author}"), GH_PR_FIELDS));
            let d = s.spawn(|| {
                list(
                    "pr",
                    "merged",
                    format!("merged:>={day}{author}"),
                    GH_PR_FIELDS,
                )
            });
            let e = s.spawn(|| {
                run_gh(
                    gh,
                    host,
                    token,
                    &[
                        "api",
                        "graphql",
                        "--hostname",
                        host,
                        "-f",
                        &format!("query={GH_COUNTS_QUERY}"),
                        "-f",
                        &format!("o={count_scope} is:open"),
                        "-f",
                        &format!("m={count_scope} is:merged"),
                    ],
                )
            });
            let f = s.spawn(|| {
                run_gh_scoped(
                    gh,
                    host,
                    token,
                    &[
                        "api",
                        "graphql",
                        "--hostname",
                        host,
                        "-f",
                        &format!("query={GH_PROJECT_QUERY}"),
                        "-f",
                        &format!("owner={owner}"),
                        "-f",
                        &format!("name={name}"),
                    ],
                )
            });
            (
                join(a),
                join(b),
                join(c),
                join(d),
                join(e),
                f.join()
                    .unwrap_or(Err((CoreError::IssuesGhUnavailable, false))),
            )
        });
    let (project, hint) = match project {
        Ok(json) => (parse_gh_project(&json, owner_repo), None),
        Err((_, true)) => (None, Some(GH_SCOPE_HINT.to_string())),
        Err(_) => (None, None),
    };
    let items = assemble_github(
        &open_issues?,
        &closed_issues?,
        &open_prs?,
        &merged_prs?,
        project.as_ref(),
    )?;
    let (columns, source) = match project {
        Some(p) => (
            project_columns("No status", p.options, "Done (14d)", false, &items),
            Some(p.title),
        ),
        None => (default_columns(), None),
    };
    Ok(Board {
        provider: IssueProvider::Github,
        items,
        columns,
        source,
        hint,
        pr_counts: parse_gh_counts(&counts?)?,
    })
}

/// Which platform a setup talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueProvider {
    Github,
    // ponytail: temporary, remove once Connect is on GitHub
    Gitlab,
}

/// One configured GitHub/GitLab connection (Settings → Issues).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSetup {
    pub id: String,
    pub name: String,
    pub provider: IssueProvider,
    /// Base URL with its scheme kept (`http://` is honoured for an instance
    /// without TLS), see [`normalize_url`].
    pub url: String,
    /// GitHub only: the `gh` account to use; `None` = gh's active account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// A setup as Settings → Issues shows it: `token_set` says whether a GitLab
/// token is in the keychain; the token itself never leaves this module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSetupInfo {
    #[serde(flatten)]
    pub setup: IssueSetup,
    pub token_set: bool,
}

/// Every setup plus each project's pick (project root → setup id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuesOverview {
    pub setups: Vec<IssueSetupInfo>,
    pub projects: std::collections::BTreeMap<std::path::PathBuf, String>,
}

/// Reduce user input to `scheme://host[:port]`: the scheme is kept (default
/// `https`), the host lowercased, any path dropped. `None` when no host is left.
pub fn normalize_url(input: &str) -> Option<String> {
    let s = input.trim();
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_string(), s),
    };
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = rest.split('/').next().unwrap_or("").trim();
    (!host.is_empty() && !host.contains(char::is_whitespace))
        .then(|| format!("{scheme}://{}", host.to_ascii_lowercase()))
}

/// The bare hostname of a normalized setup URL (port dropped, as
/// [`parse_remote`] drops it from the remote).
pub fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or("");
    authority.split(':').next().unwrap_or("")
}

// ponytail: temporary, remove once Connect is on GitHub — the GitLab parts
// of this module (this block, `IssueProvider::Gitlab`, the dispatch arm in
// `board`, the two errors, the Settings fields) go in one sweep.

const KEYCHAIN_SERVICE: &str = "com.podium.app.gitlab";

fn keychain_entry(setup_id: &str) -> CoreResult<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, setup_id)
        .map_err(|_| CoreError::IssuesFetch("could not open the keychain".to_string()))
}

/// Store the token for a setup in the OS keychain; a blank token deletes it.
pub fn gitlab_token_set(setup_id: &str, token: &str) -> CoreResult<()> {
    let entry = keychain_entry(setup_id)?;
    let result = if token.trim().is_empty() {
        match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        }
    } else {
        entry.set_password(token.trim())
    };
    result.map_err(|_| CoreError::IssuesFetch("could not update the keychain".to_string()))
}

fn gitlab_token_get(setup_id: &str) -> Option<String> {
    keychain_entry(setup_id).ok()?.get_password().ok()
}

/// Whether a token is stored for a setup (the token itself never leaves here).
pub fn gitlab_token_present(setup_id: &str) -> bool {
    gitlab_token_get(setup_id).is_some()
}

#[derive(Deserialize)]
struct GlItem {
    iid: u64,
    title: String,
    web_url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    author: Option<GlUser>,
    /// MRs only.
    #[serde(default)]
    draft: bool,
    /// Issues only: MRs that mention closing it.
    #[serde(default)]
    merge_requests_count: u64,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    merged_at: Option<String>,
}

#[derive(Deserialize)]
struct GlUser {
    username: String,
}

impl GlItem {
    fn into_item(self, kind: ItemKind, column: &str) -> BoardItem {
        BoardItem {
            kind,
            number: self.iid,
            title: self.title,
            url: self.web_url,
            body: self.description.unwrap_or_default(),
            labels: self.labels,
            updated_at: self.updated_at,
            column: column.to_string(),
            draft: self.draft,
            author: self.author.map(|a| a.username).unwrap_or_default(),
        }
    }
}

fn parse_gl(json: &str) -> CoreResult<Vec<GlItem>> {
    serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesFetch("unexpected response from GitLab".to_string()))
}

/// Whether an RFC 3339 timestamp is at or after `since` (a missing or
/// unparseable one is not).
fn at_or_after(ts: Option<&str>, since: DateTime<Utc>) -> bool {
    ts.and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| t >= since)
}

/// The first board with a label list, as `(board name, label columns in
/// position order)`. Lists without a label (assignee, milestone) are skipped.
// ponytail: first board only; a per-project board picker is the upgrade path.
fn parse_gl_board(json: &str) -> Option<(String, Vec<Column>)> {
    let boards: serde_json::Value = serde_json::from_str(json).ok()?;
    boards.as_array()?.iter().find_map(|b| {
        let mut lists: Vec<_> = b["lists"]
            .as_array()?
            .iter()
            .filter(|l| l["label"]["name"].is_string())
            .collect();
        lists.sort_by_key(|l| l["position"].as_i64().unwrap_or(0));
        let cols: Vec<Column> = lists
            .iter()
            .map(|l| {
                let name = l["label"]["name"].as_str().unwrap_or_default();
                col(
                    &format!("label:{name}"),
                    name,
                    l["label"]["color"].as_str().map(str::to_string),
                )
            })
            .collect();
        (!cols.is_empty()).then(|| (b["name"].as_str().unwrap_or("Board").to_string(), cols))
    })
}

/// Build the items from the four GitLab list responses. The closed/merged
/// lists are filtered by `updated_after`, so the real closed/merged time is
/// checked here. With board `lists`, an open card sits in the first list whose
/// label it carries, else issue → `open`, MR → `review`.
fn assemble_gitlab(
    open_issues: &str,
    closed_issues: &str,
    open_mrs: &str,
    merged_mrs: &str,
    lists: &[Column],
    since: DateTime<Utc>,
) -> CoreResult<Vec<BoardItem>> {
    let listed = |labels: &[String], default: &str| {
        lists
            .iter()
            .find(|c| {
                c.id.strip_prefix("label:")
                    .is_some_and(|l| labels.iter().any(|x| x == l))
            })
            .map_or(default, |c| c.id.as_str())
            .to_string()
    };
    let mut items = Vec::new();
    for i in parse_gl(open_issues)? {
        let column = if lists.is_empty() {
            issue_column(&i.labels, i.merge_requests_count > 0).to_string()
        } else {
            listed(&i.labels, "open")
        };
        items.push(i.into_item(ItemKind::Issue, &column));
    }
    for i in parse_gl(closed_issues)? {
        if at_or_after(i.closed_at.as_deref(), since) {
            items.push(i.into_item(ItemKind::Issue, "done"));
        }
    }
    for m in parse_gl(open_mrs)? {
        let column = if lists.is_empty() {
            pr_column(m.draft).to_string()
        } else {
            listed(&m.labels, "review")
        };
        items.push(m.into_item(ItemKind::Pr, &column));
    }
    for m in parse_gl(merged_mrs)? {
        if at_or_after(m.merged_at.as_deref(), since) {
            items.push(m.into_item(ItemKind::Pr, "done"));
        }
    }
    Ok(sorted(items))
}

/// One GitLab API GET: the body plus the `X-Total` header.
fn gitlab_get(base: &str, token: &str, path: &str) -> CoreResult<(String, Option<u64>)> {
    let response = reqwest::blocking::Client::new()
        .get(format!("{base}{path}"))
        .header("PRIVATE-TOKEN", token)
        .send()
        .map_err(|_| CoreError::IssuesFetch("could not reach GitLab".to_string()))?;
    match response.status().as_u16() {
        200 => {
            let total = response
                .headers()
                .get("x-total")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            let body = response
                .text()
                .map_err(|_| CoreError::IssuesFetch("could not read GitLab's response".into()))?;
            Ok((body, total))
        }
        401 | 403 => Err(CoreError::IssuesGitlabAuth),
        429 => Err(CoreError::IssuesFetch(
            "GitLab rate limit reached, try again later".to_string(),
        )),
        _ => Err(CoreError::IssuesFetch(
            "GitLab could not fetch the board".to_string(),
        )),
    }
}

/// The board for `project_path` on the instance at `base` (`https://host`;
/// injectable so tests can use plain HTTP). `mine` narrows it to issues
/// assigned to and MRs created by the token's owner. The six requests run in
/// parallel; the sixth (`/boards`, for the project's columns) never fails the
/// board.
pub fn fetch_gitlab(
    base: &str,
    project_path: &str,
    token: &str,
    mine: bool,
    since: DateTime<Utc>,
) -> CoreResult<Board> {
    let p = format!("/api/v4/projects/{}", project_path.replace('/', "%2F"));
    let (issue_scope, mr_scope) = if mine {
        ("assigned_to_me", "created_by_me")
    } else {
        ("all", "all")
    };
    let after = since.format("%Y-%m-%dT%H:%M:%SZ");
    let get = |path: String| gitlab_get(base, token, &path);
    let (open_issues, closed_issues, open_mrs, merged_mrs, merged_count, boards) =
        std::thread::scope(|s| {
            let a = s.spawn(|| {
                get(format!(
                    "{p}/issues?state=opened&scope={issue_scope}&per_page={LIMIT}"
                ))
            });
            let b = s.spawn(|| {
                get(format!(
                    "{p}/issues?state=closed&scope={issue_scope}&updated_after={after}&per_page={LIMIT}"
                ))
            });
            let c = s.spawn(|| {
                get(format!(
                    "{p}/merge_requests?state=opened&scope={mr_scope}&per_page={LIMIT}"
                ))
            });
            let d = s.spawn(|| {
                get(format!(
                    "{p}/merge_requests?state=merged&scope={mr_scope}&updated_after={after}&per_page={LIMIT}"
                ))
            });
            let e = s.spawn(|| {
                get(format!(
                    "{p}/merge_requests?state=merged&scope={mr_scope}&per_page=1"
                ))
            });
            let f = s.spawn(|| get(format!("{p}/boards")));
            (join(a), join(b), join(c), join(d), join(e), join(f))
        });
    let (open_mrs, open_total) = open_mrs?;
    // Any boards failure (403/404 included) just means the default columns.
    let (source, lists) = match boards.ok().and_then(|(json, _)| parse_gl_board(&json)) {
        Some((name, lists)) => (Some(name), lists),
        None => (None, Vec::new()),
    };
    let items = assemble_gitlab(
        &open_issues?.0,
        &closed_issues?.0,
        &open_mrs,
        &merged_mrs?.0,
        &lists,
        since,
    )?;
    let columns = if source.is_some() {
        project_columns("Open", lists, "Closed (14d)", true, &items)
    } else {
        default_columns()
    };
    Ok(Board {
        provider: IssueProvider::Gitlab,
        items,
        columns,
        source,
        hint: None,
        pr_counts: PrCounts {
            open: open_total,
            merged: merged_count?.1,
        },
    })
}

/// The setup that serves `remote_host`: the project's `selected` one (which
/// must be for that host), else the first configured for it. `Ok(None)` means
/// github.com with nothing configured — the active `gh` account, zero-config.
fn pick<'a>(
    remote_host: &str,
    setups: &'a [IssueSetup],
    selected: Option<&'a IssueSetup>,
) -> CoreResult<Option<&'a IssueSetup>> {
    if let Some(setup) = selected {
        return if url_host(&setup.url) == remote_host {
            Ok(Some(setup))
        } else {
            Err(CoreError::IssuesHostMismatch {
                setup: setup.name.clone(),
                remote: remote_host.to_string(),
            })
        };
    }
    match setups.iter().find(|s| url_host(&s.url) == remote_host) {
        Some(setup) => Ok(Some(setup)),
        None if remote_host == "github.com" => Ok(None),
        None => Err(CoreError::IssuesUnsupportedHost),
    }
}

/// Detect the repo behind `root` and build its board, using the project's
/// `selected` setup or the first matching one.
pub fn board(
    root: &Path,
    setups: &[IssueSetup],
    selected: Option<&IssueSetup>,
    mine: bool,
) -> CoreResult<Board> {
    let (host, path) = detect(root)?;
    let setup = pick(&host, setups, selected)?;
    let since = Utc::now() - chrono::Duration::days(DONE_DAYS);
    match setup {
        Some(IssueSetup {
            provider: IssueProvider::Gitlab,
            id,
            url,
            ..
        }) => {
            // ponytail: temporary, remove once Connect is on GitHub
            let token = gitlab_token_get(id).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            fetch_gitlab(url, &path, &token, mine, since)
        }
        _ => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            let account = setup.and_then(|s| s.account.as_deref());
            fetch_github(gh, &host, &path, account, mine, since)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn since() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-15T00:00:00Z")
            .unwrap()
            .into()
    }

    #[test]
    fn parse_remote_table() {
        let gh = Some(("github.com".to_string(), "acme/widgets".to_string()));
        for url in [
            "git@github.com:acme/widgets.git",
            "git@github.com:acme/widgets",
            "ssh://git@github.com/acme/widgets.git",
            "ssh://git@github.com:2222/acme/widgets.git",
            "https://github.com/acme/widgets.git",
            "https://github.com/acme/widgets",
            "https://user@github.com/acme/widgets/",
            "  https://GitHub.com/acme/widgets.git\n",
        ] {
            assert_eq!(parse_remote(url), gh, "{url}");
        }
        assert_eq!(
            parse_remote("git@git.example.test:group/sub/proj.git"),
            Some(("git.example.test".to_string(), "group/sub/proj".to_string()))
        );
        for junk in [
            "",
            "nonsense",
            "https://github.com",
            "https://github.com/x",
            "/local/path",
        ] {
            assert_eq!(parse_remote(junk), None, "{junk}");
        }
    }

    #[test]
    fn in_progress_labels() {
        let l = |s: &str| vec![s.to_string()];
        for yes in [
            "In progress",
            "in-progress",
            "workflow::In Progress",
            "Doing",
            "WIP",
        ] {
            assert!(in_progress_label(&l(yes)), "{yes}");
        }
        for no in ["bug", "progress report", "wipe", "status::todo"] {
            assert!(!in_progress_label(&l(no)), "{no}");
        }
    }

    #[test]
    fn github_board_columns() {
        let open_issues = r#"[
          {"number":7,"title":"Fix widget","url":"https://github.com/acme/widgets/issues/7",
           "body":"It breaks.","labels":[{"name":"bug","color":"d73a4a"}],
           "updatedAt":"2026-09-01T10:00:00Z","author":{"login":"sam"}},
          {"number":8,"title":"Linked","url":"u8","updatedAt":"2026-09-03T10:00:00Z"},
          {"number":9,"title":"Labelled","url":"u9","labels":[{"name":"in progress"}],
           "updatedAt":"2026-09-02T10:00:00Z"}
        ]"#;
        let closed =
            r#"[{"number":3,"title":"Old","url":"u3","updatedAt":"2026-09-20T10:00:00Z"}]"#;
        let open_prs = r#"[
          {"number":20,"title":"Fix 8","url":"u20","isDraft":false,
           "closingIssuesReferences":[{"number":8}],"updatedAt":"2026-09-04T10:00:00Z"},
          {"number":21,"title":"WIP","url":"u21","isDraft":true,"updatedAt":"2026-09-05T10:00:00Z"}
        ]"#;
        let merged =
            r#"[{"number":19,"title":"Done","url":"u19","updatedAt":"2026-09-21T10:00:00Z"}]"#;
        let items = assemble_github(open_issues, closed, open_prs, merged, None).unwrap();
        let col = |n: u64, k: ItemKind| {
            items
                .iter()
                .find(|i| i.number == n && i.kind == k)
                .unwrap()
                .column
                .as_str()
        };
        assert_eq!(col(7, ItemKind::Issue), "open");
        assert_eq!(col(8, ItemKind::Issue), "inProgress");
        assert_eq!(col(9, ItemKind::Issue), "inProgress");
        assert_eq!(col(3, ItemKind::Issue), "done");
        assert_eq!(col(20, ItemKind::Pr), "review");
        assert_eq!(col(21, ItemKind::Pr), "inProgress");
        assert_eq!(col(19, ItemKind::Pr), "done");
        // Newest first.
        assert_eq!(items[0].number, 19);
        let seven = items.iter().find(|i| i.number == 7).unwrap();
        assert_eq!(seven.labels, vec!["bug"]);
        assert_eq!(seven.author, "sam");
        assert!(matches!(
            assemble_github("not json", "[]", "[]", "[]", None),
            Err(CoreError::IssuesFetch(_))
        ));
    }

    #[test]
    fn github_counts() {
        let c =
            parse_gh_counts(r#"{"data":{"o":{"issueCount":5},"m":{"issueCount":120}}}"#).unwrap();
        assert_eq!(
            c,
            PrCounts {
                open: Some(5),
                merged: Some(120)
            }
        );
        assert_eq!(parse_gh_counts("{}").unwrap(), PrCounts::default());
    }

    #[test]
    fn normalizes_url() {
        for (input, want) in [
            ("git.example.test", Some("https://git.example.test")),
            (
                "https://Git.Example.test/",
                Some("https://git.example.test"),
            ),
            (
                "  http://git.example.test/group/proj ",
                Some("http://git.example.test"),
            ),
            ("HTTP://git.example.test", Some("http://git.example.test")),
            (
                "git.example.test:8443",
                Some("https://git.example.test:8443"),
            ),
            ("ftp://git.example.test", None),
            ("", None),
            ("   ", None),
            ("https://", None),
            ("two words", None),
        ] {
            assert_eq!(normalize_url(input).as_deref(), want, "{input:?}");
        }
        assert_eq!(url_host("http://git.example.test:8080"), "git.example.test");
        assert_eq!(url_host("https://github.com"), "github.com");
    }

    #[test]
    fn gitlab_board_columns() {
        let open_issues = r#"[
          {"id":501,"iid":12,"title":"Fix invoice","web_url":"https://git.example.test/grp/proj/-/issues/12",
           "description":"Broken.","labels":["bug"],"updated_at":"2026-09-01T10:00:00Z",
           "author":{"username":"sam"},"merge_requests_count":0},
          {"iid":13,"title":"Has MR","web_url":"u13","description":null,"labels":[],
           "updated_at":"2026-09-02T10:00:00Z","merge_requests_count":1}
        ]"#;
        let closed = r#"[
          {"iid":4,"title":"Recent","web_url":"u4","updated_at":"2026-09-20T10:00:00Z",
           "closed_at":"2026-09-20T10:00:00.000Z"},
          {"iid":5,"title":"Touched, closed long ago","web_url":"u5",
           "updated_at":"2026-09-20T10:00:00Z","closed_at":"2026-08-01T10:00:00Z"}
        ]"#;
        let open_mrs = r#"[
          {"iid":40,"title":"Ready","web_url":"u40","draft":false,"updated_at":"2026-09-03T10:00:00Z"},
          {"iid":41,"title":"Draft","web_url":"u41","draft":true,"updated_at":"2026-09-04T10:00:00Z"}
        ]"#;
        let merged = r#"[{"iid":38,"title":"Merged","web_url":"u38",
          "updated_at":"2026-09-21T10:00:00Z","merged_at":"2026-09-21T10:00:00+02:00"}]"#;
        let items = assemble_gitlab(open_issues, closed, open_mrs, merged, &[], since()).unwrap();
        let col = |n: u64| {
            items
                .iter()
                .find(|i| i.number == n)
                .map(|i| i.column.as_str())
        };
        assert_eq!(col(12), Some("open"));
        assert_eq!(col(13), Some("inProgress"));
        assert_eq!(col(4), Some("done"));
        assert_eq!(col(5), None);
        assert_eq!(col(40), Some("review"));
        assert_eq!(col(41), Some("inProgress"));
        assert_eq!(col(38), Some("done"));
        let twelve = items.iter().find(|i| i.number == 12).unwrap();
        assert_eq!(twelve.body, "Broken.");
        assert_eq!(twelve.author, "sam");
        assert_eq!(twelve.kind, ItemKind::Issue);
        assert!(matches!(
            assemble_gitlab("{}", "[]", "[]", "[]", &[], since()),
            Err(CoreError::IssuesFetch(_))
        ));
    }

    fn setup(id: &str, provider: IssueProvider, url: &str) -> IssueSetup {
        IssueSetup {
            id: id.into(),
            name: id.into(),
            provider,
            url: url.into(),
            account: None,
        }
    }

    #[test]
    fn picks_setup_by_selection_then_host() {
        let gl = setup("gl", IssueProvider::Gitlab, "http://git.example.test");
        let gh2 = setup("gh2", IssueProvider::Github, "https://github.com");
        let setups = vec![gl.clone(), gh2.clone()];

        // Zero-config github.com, and unknown hosts.
        assert_eq!(pick("github.com", &[], None).unwrap(), None);
        assert!(matches!(
            pick("other.test", &setups, None),
            Err(CoreError::IssuesUnsupportedHost)
        ));
        // Auto: first setup whose host matches.
        assert_eq!(pick("git.example.test", &setups, None).unwrap(), Some(&gl));
        assert_eq!(pick("github.com", &setups, None).unwrap(), Some(&gh2));
        // An explicit pick wins, but must be for the remote's host.
        assert_eq!(pick("github.com", &setups, Some(&gh2)).unwrap(), Some(&gh2));
        assert!(matches!(
            pick("github.com", &setups, Some(&gl)),
            Err(CoreError::IssuesHostMismatch { .. })
        ));
    }

    /// Local HTTP server answering every request by path: an MR list query
    /// gets an `X-Total`, everything else `[]`. Returns the base URL and the
    /// request lines it saw once `n` requests arrived.
    fn serve_board(
        status: &'static str,
        n: usize,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..n {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let got = sock.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..got]).into_owned();
                let total = if req.contains("merge_requests?state=opened") {
                    "X-Total: 7\r\n"
                } else if req.contains("per_page=1 ") {
                    "X-Total: 250\r\n"
                } else {
                    ""
                };
                let body = if status.starts_with("200") {
                    "[]"
                } else {
                    "secret-ish body"
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\n{total}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(reply.as_bytes()).unwrap();
                seen.push(req);
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn gitlab_fetch_sends_token_scope_and_reads_totals() {
        let (base, seen) = serve_board("200 OK", 6);
        let board = fetch_gitlab(&base, "grp/sub/proj", "fake-token", true, since()).unwrap();
        assert_eq!(board.provider, IssueProvider::Gitlab);
        // The empty /boards answer means the default columns, no error.
        assert_eq!(board.columns, default_columns());
        assert_eq!((board.source, board.hint), (None, None));
        assert_eq!(
            board.pr_counts,
            PrCounts {
                open: Some(7),
                merged: Some(250)
            }
        );
        let reqs = seen.join().unwrap();
        assert!(reqs
            .iter()
            .all(|r| r.to_lowercase().contains("private-token: fake-token")));
        assert!(reqs.iter().any(|r| r.starts_with(
            "GET /api/v4/projects/grp%2Fsub%2Fproj/issues?state=opened&scope=assigned_to_me"
        )));
        assert!(reqs
            .iter()
            .any(|r| r.contains("merge_requests?state=opened&scope=created_by_me")));
        assert!(reqs
            .iter()
            .any(|r| r.contains("updated_after=2026-09-15T00:00:00Z")));
        assert!(reqs
            .iter()
            .any(|r| r.starts_with("GET /api/v4/projects/grp%2Fsub%2Fproj/boards ")));

        let (base, seen) = serve_board("200 OK", 6);
        fetch_gitlab(&base, "a/b", "t", false, since()).unwrap();
        assert!(seen
            .join()
            .unwrap()
            .iter()
            .filter(|r| !r.contains("/boards "))
            .all(|r| r.contains("scope=all")));
    }

    #[test]
    fn gitlab_fetch_maps_status() {
        for code in ["401 Unauthorized", "403 Forbidden"] {
            let (base, _h) = serve_board(code, 6);
            assert!(matches!(
                fetch_gitlab(&base, "a/b", "wrong", true, since()),
                Err(CoreError::IssuesGitlabAuth)
            ));
        }
        let (base, _h) = serve_board("500 Internal Server Error", 6);
        let err = fetch_gitlab(&base, "a/b", "t", true, since()).unwrap_err();
        assert!(matches!(err, CoreError::IssuesFetch(_)));
        assert!(!err.to_string().contains("secret-ish"));
    }

    #[test]
    fn serializes_camel_case() {
        let items = assemble_github(
            r#"[{"number":1,"title":"t","url":"u","updatedAt":"x"}]"#,
            "[]",
            r#"[{"number":2,"title":"p","url":"u2","isDraft":false,"updatedAt":"y"}]"#,
            "[]",
            None,
        )
        .unwrap();
        let v = serde_json::to_value(Board {
            provider: IssueProvider::Github,
            items,
            columns: default_columns(),
            source: None,
            hint: None,
            pr_counts: PrCounts::default(),
        })
        .unwrap();
        assert_eq!(v["items"][0]["updatedAt"], "y");
        assert_eq!(v["items"][0]["kind"], "pr");
        assert_eq!(v["items"][0]["column"], "review");
        assert_eq!(v["items"][1]["column"], "open");
        assert!(v["prCounts"]["open"].is_null());
        assert_eq!(v["provider"], "github");
        assert_eq!(v["columns"][1]["id"], "inProgress");
        assert_eq!(v["columns"][3]["title"], "Done (14d)");
        assert!(v["columns"][0]["color"].is_null());
        assert!(v["source"].is_null() && v["hint"].is_null());
    }

    const GL_BOARDS: &str = r##"[
      {"name":"Empty","lists":[{"position":0,"assignee":{"username":"sam"},"label":null}]},
      {"name":"Development","lists":[
        {"position":2,"label":{"name":"review-me","color":"#A295D6"}},
        {"position":0,"label":{"name":"doing","color":"#428BCA"}},
        {"position":1,"assignee":{"username":"sam"},"label":null}]}
    ]"##;

    #[test]
    fn gitlab_board_lists_sorted_and_labelless_skipped() {
        let (name, lists) = parse_gl_board(GL_BOARDS).unwrap();
        assert_eq!(name, "Development");
        let ids: Vec<_> = lists.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["label:doing", "label:review-me"]);
        assert_eq!(lists[0].color.as_deref(), Some("#428BCA"));
        assert_eq!(parse_gl_board("[]"), None);
        assert_eq!(parse_gl_board(r#"[{"name":"x","lists":[]}]"#), None);
        assert_eq!(parse_gl_board("nope"), None);
    }

    #[test]
    fn gitlab_placement_with_board() {
        let (_, lists) = parse_gl_board(GL_BOARDS).unwrap();
        let open_issues = r#"[
          {"iid":1,"title":"a","web_url":"u","labels":["review-me","doing"]},
          {"iid":2,"title":"b","web_url":"u","labels":["bug"]}
        ]"#;
        let closed = r#"[{"iid":3,"title":"c","web_url":"u","labels":["doing"],
          "closed_at":"2026-09-20T10:00:00Z"}]"#;
        let open_mrs = r#"[
          {"iid":10,"title":"m","web_url":"u","labels":["review-me"]},
          {"iid":11,"title":"n","web_url":"u","labels":[]}
        ]"#;
        let merged = r#"[{"iid":12,"title":"o","web_url":"u","labels":["doing"],
          "merged_at":"2026-09-20T10:00:00Z"}]"#;
        let items =
            assemble_gitlab(open_issues, closed, open_mrs, merged, &lists, since()).unwrap();
        let col = |n: u64, k: ItemKind| {
            items
                .iter()
                .find(|i| i.number == n && i.kind == k)
                .unwrap()
                .column
                .clone()
        };
        // First list in position order wins; unmatched issue -> open.
        assert_eq!(col(1, ItemKind::Issue), "label:doing");
        assert_eq!(col(2, ItemKind::Issue), "open");
        assert_eq!(col(3, ItemKind::Issue), "done");
        assert_eq!(col(10, ItemKind::Pr), "label:review-me");
        assert_eq!(col(11, ItemKind::Pr), "review");
        assert_eq!(col(12, ItemKind::Pr), "done");
        let ids = |items: &[BoardItem]| -> Vec<String> {
            project_columns("Open", lists.clone(), "Closed (14d)", true, items)
                .into_iter()
                .map(|c| c.id)
                .collect()
        };
        assert_eq!(
            ids(&items),
            ["open", "label:doing", "label:review-me", "review", "done"]
        );
        // Review column only when an MR landed there; done always.
        let no_review: Vec<_> = items
            .iter()
            .filter(|i| i.column != "review")
            .cloned()
            .collect();
        assert_eq!(
            ids(&no_review),
            ["open", "label:doing", "label:review-me", "done"]
        );
    }

    const GH_PROJECT: &str = r#"{"data":{"repository":{"projectsV2":{"nodes":[{
      "title":"Widgets roadmap",
      "field":{"options":[{"id":"o1","name":"Todo","color":"GRAY"},
                          {"id":"o2","name":"Doing","color":"BLUE"}]},
      "items":{"nodes":[
        {"content":{"number":7,"repository":{"nameWithOwner":"acme/widgets"}},"fieldValueByName":{"optionId":"o2"}},
        {"content":{"number":20,"repository":{"nameWithOwner":"acme/widgets"}},"fieldValueByName":{"optionId":"o1"}},
        {"content":{"number":3,"repository":{"nameWithOwner":"acme/widgets"}},"fieldValueByName":{"optionId":"o2"}},
        {"content":{"number":8,"repository":{"nameWithOwner":"acme/widgets"}},"fieldValueByName":null},
        {"content":{"number":8,"repository":{"nameWithOwner":"acme/other"}},
         "fieldValueByName":{"optionId":"o1"}},
        {"content":{"number":9,"repository":{"nameWithOwner":"Acme/Widgets"}},"fieldValueByName":{"optionId":"gone"}},
        {"content":{}}]}}]}}}}"#;

    #[test]
    fn github_placement_with_project() {
        let p = parse_gh_project(GH_PROJECT, "acme/widgets").unwrap();
        assert_eq!(p.title, "Widgets roadmap");
        assert_eq!(p.options[1].id, "status:o2");
        assert_eq!(p.options[1].color.as_deref(), Some("blue"));
        let items = assemble_github(
            r#"[{"number":7,"title":"a","url":"u"},{"number":8,"title":"b","url":"u"},
                {"number":9,"title":"stale option","url":"u"}]"#,
            r#"[{"number":3,"title":"closed but placed","url":"u"},
                {"number":4,"title":"closed","url":"u"}]"#,
            r#"[{"number":20,"title":"pr placed","url":"u"},
                {"number":21,"title":"pr draft","url":"u","isDraft":true}]"#,
            "[]",
            Some(&p),
        )
        .unwrap();
        let col = |n: u64| {
            items
                .iter()
                .find(|i| i.number == n)
                .unwrap()
                .column
                .as_str()
        };
        assert_eq!(col(7), "status:o2");
        // #8 of another repo in the same project is not this repo's #8.
        assert_eq!(col(8), "open");
        assert_eq!(col(9), "open");
        assert_eq!(col(3), "status:o2");
        assert_eq!(col(4), "done");
        assert_eq!(col(20), "status:o1");
        // The default draft/linked placement does not apply with a project.
        assert_eq!(col(21), "review");
        let ids: Vec<_> =
            project_columns("No status", p.options.clone(), "Done (14d)", false, &items)
                .into_iter()
                .map(|c| c.id)
                .collect();
        assert_eq!(ids, ["open", "status:o1", "status:o2", "review", "done"]);
        // No project, no Status field or no options: none.
        assert_eq!(parse_gh_project("{}", "acme/widgets"), None);
        assert_eq!(
            parse_gh_project(
                r#"{"data":{"repository":{"projectsV2":{"nodes":[]}}}}"#,
                "acme/widgets"
            ),
            None
        );
        assert_eq!(
            parse_gh_project(
                r#"{"data":{"repository":{"projectsV2":{"nodes":[{"title":"t","field":null}]}}}}"#,
                "acme/widgets"
            ),
            None
        );
    }

    #[cfg(unix)]
    mod fake_gh {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(dir: &Path, body: &str) -> String {
            let p = dir.join("gh");
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p.to_string_lossy().into_owned()
        }

        /// Prepended to [`FAKE`]: answers the Project query.
        fn with_project(reply: &str) -> String {
            format!("case \"$*\" in *projectsV2*) {reply};; esac\n{FAKE}")
        }

        /// A fake `gh` answering lists with one item echoing its args, and the
        /// counts query with fixed numbers.
        const FAKE: &str = r#"if [ "$1" = auth ]; then [ "$6" = work ] && echo fake-tok && exit 0; exit 1; fi
if [ "$1" = api ]; then echo '{"data":{"o":{"issueCount":2},"m":{"issueCount":9}}}'; exit 0; fi
echo "[{\"number\":1,\"title\":\"$1 $6 ${GH_TOKEN:-none} $4\",\"url\":\"u\",\"body\":\"${10}\"}]""#;

        #[test]
        fn builds_board_and_scopes_searches() {
            let dir = tempfile::tempdir().unwrap();
            let gh = script(dir.path(), FAKE);
            let board =
                fetch_github(&gh, "github.com", "acme/widgets", None, true, since()).unwrap();
            assert_eq!(
                board.pr_counts,
                PrCounts {
                    open: Some(2),
                    merged: Some(9)
                }
            );
            let find = |prefix: &str| {
                board
                    .items
                    .iter()
                    .find(|i| i.title.starts_with(prefix))
                    .unwrap()
                    .clone()
            };
            assert_eq!(find("issue open").column, "open");
            assert_eq!(find("issue closed").column, "done");
            assert_eq!(find("pr open").column, "review");
            assert_eq!(find("pr merged").column, "done");
            // The fake answers the Project query with the counts JSON: no project.
            assert_eq!(board.columns, default_columns());
            assert_eq!((board.source, board.hint), (None, None));
            assert_eq!(find("issue open").body, "is:open assignee:@me");
            assert_eq!(find("pr merged").body, "merged:>=2026-09-15 author:@me");
            assert!(find("pr open").title.ends_with("github.com/acme/widgets"));

            let all =
                fetch_github(&gh, "github.com", "acme/widgets", None, false, since()).unwrap();
            assert!(all.items.iter().all(|i| !i.body.contains("@me")));
        }

        #[test]
        fn project_columns_and_scope_hint() {
            let dir = tempfile::tempdir().unwrap();
            let one_line = GH_PROJECT.replace('\n', "");
            let gh = script(
                dir.path(),
                &with_project(&format!("echo '{one_line}'; exit 0")),
            );
            let b = fetch_github(&gh, "github.com", "acme/widgets", None, true, since()).unwrap();
            assert_eq!(b.source.as_deref(), Some("Widgets roadmap"));
            assert_eq!(b.hint, None);
            let ids: Vec<_> = b.columns.iter().map(|c| c.id.as_str()).collect();
            // Fake items (numbers 1) have no status; the open PR lands in review.
            assert_eq!(ids, ["open", "status:o1", "status:o2", "review", "done"]);
            assert_eq!(b.columns[0].title, "No status");

            // Missing read:project scope: defaults plus the hint, board intact.
            let gh = script(
                dir.path(),
                &with_project("echo 'needs the read:project scope' >&2; exit 1"),
            );
            let b = fetch_github(&gh, "github.com", "acme/widgets", None, true, since()).unwrap();
            assert_eq!(b.columns, default_columns());
            assert_eq!(b.source, None);
            let hint = b.hint.unwrap();
            assert!(hint.contains("gh auth refresh -s read:project"));
            assert!(!hint.contains("needs the"));

            // Any other failure of that call: defaults, no hint.
            let gh = script(dir.path(), &with_project("echo 'boom' >&2; exit 1"));
            let b = fetch_github(&gh, "github.com", "acme/widgets", None, true, since()).unwrap();
            assert_eq!((b.columns, b.hint), (default_columns(), None));
        }

        #[test]
        fn account_token_is_passed_to_gh_only_via_env() {
            let dir = tempfile::tempdir().unwrap();
            let gh = script(dir.path(), FAKE);
            let b = fetch_github(
                &gh,
                "github.com",
                "acme/widgets",
                Some("work"),
                true,
                since(),
            )
            .unwrap();
            assert!(b.items.iter().all(|i| i.title.contains(" fake-tok ")));
            let b = fetch_github(&gh, "github.com", "acme/widgets", None, true, since()).unwrap();
            assert!(b.items.iter().all(|i| i.title.contains(" none ")));
            assert!(matches!(
                fetch_github(
                    &gh,
                    "github.com",
                    "acme/widgets",
                    Some("nobody"),
                    true,
                    since()
                ),
                Err(CoreError::IssuesGhUnavailable)
            ));
        }

        #[test]
        fn error_classification() {
            let dir = tempfile::tempdir().unwrap();
            let auth = script(dir.path(), "echo 'run: gh auth login' >&2; exit 4");
            assert!(matches!(
                fetch_github(&auth, "github.com", "acme/widgets", None, true, since()),
                Err(CoreError::IssuesGhUnavailable)
            ));

            let other = script(dir.path(), "echo 'secret-ish stderr' >&2; exit 1");
            let err = fetch_github(&other, "github.com", "acme/widgets", None, true, since())
                .unwrap_err();
            assert!(matches!(err, CoreError::IssuesFetch(_)));
            assert!(!err.to_string().contains("secret-ish"));

            let limited = script(dir.path(), "echo 'API rate limit exceeded' >&2; exit 1");
            let err = fetch_github(&limited, "github.com", "acme/widgets", None, true, since())
                .unwrap_err();
            assert!(err.to_string().contains("rate limit"));

            assert!(matches!(
                fetch_github(
                    "/nonexistent/gh",
                    "github.com",
                    "acme/widgets",
                    None,
                    true,
                    since()
                ),
                Err(CoreError::IssuesGhUnavailable)
            ));
        }
    }
}
