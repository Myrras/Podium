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

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub title: String,
    /// A hex string (GitLab) or a lowercase GitHub option colour name.
    pub color: Option<String>,
    /// The kinds a card may be dropped here as (see [`col`]).
    #[serde(default)]
    pub accepts: Vec<ItemKind>,
}

/// A column with the drop rules of its id: `open`/`done` take issues,
/// `review` PRs/MRs, project columns (`label:*`/`status:*`) both, and
/// `inProgress` nothing (placement there is derived, not dropped).
fn col(id: &str, title: &str, color: Option<String>) -> Column {
    let accepts = match id {
        "open" | "done" => vec![ItemKind::Issue],
        "review" => vec![ItemKind::Pr],
        "inProgress" => vec![],
        _ => vec![ItemKind::Issue, ItemKind::Pr],
    };
    Column {
        id: id.to_string(),
        title: title.to_string(),
        color,
        accepts,
    }
}

fn default_columns() -> Vec<Column> {
    let mut review = col("review", "In review", None);
    // Without a project board an open PR is placed by its state alone.
    review.accepts.clear();
    vec![
        col("open", "Open", None),
        col("inProgress", "In progress", None),
        review,
        col("done", "Done (14d)", None),
    ]
}

/// Lead column, the project's own columns, then `review` and `done` — always
/// both, so columns do not come and go between refreshes.
fn project_columns(open_title: &str, mid: Vec<Column>, done_title: &str) -> Vec<Column> {
    let mut cols = vec![col("open", open_title, None)];
    cols.extend(mid);
    cols.push(col("review", "In review", None));
    cols.push(col("done", done_title, None));
    cols
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Issue,
    /// A GitHub PR or a GitLab MR.
    Pr,
}

/// Who approved an open PR/MR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approvals {
    /// Logins/usernames that approved.
    pub by: Vec<String>,
    /// Approvals the platform requires (GitLab); `None` when it does not say.
    pub required: Option<u32>,
}

/// A PR/MR that closes an issue on the board, shown on the issue's card
/// instead of as a card of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub merged: bool,
    #[serde(default)]
    pub approvals: Option<Approvals>,
}

fn yes() -> bool {
    true
}

/// A merged PR/MR is history; everything else can be dragged.
pub(crate) fn movable(kind: ItemKind, column: &str) -> bool {
    !(kind == ItemKind::Pr && column == "done")
}

/// One card on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// False for a merged PR/MR.
    #[serde(default = "yes")]
    pub movable: bool,
    /// PRs/MRs folded into this issue (always empty on PRs).
    #[serde(default)]
    pub linked: Vec<LinkedPr>,
    /// Logins/usernames the item is assigned to.
    #[serde(default)]
    pub assignees: Vec<String>,
    /// Open PRs/MRs only; `None` when unknown.
    #[serde(default)]
    pub approvals: Option<Approvals>,
}

/// All-time PR/MR totals in the board's scope; `None` when the platform
/// would not say (GitLab omits the total past 10k).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCounts {
    pub open: Option<u64>,
    pub merged: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The signed-in user's login/username; `None` when the platform would
    /// not say (or on a board cached before this existed).
    #[serde(default)]
    pub me: Option<String>,
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

/// What one PR/MR closes, for [`fold_linked`].
struct Closing {
    pr: u64,
    merged: bool,
    issues: Vec<u64>,
    /// Issues it only mentions (`Onderdeel van #N`): the fallback when none
    /// of `issues` is on the board, so an MR closing a sub-issue that is not
    /// yours lands on the parent issue that is.
    mentions: Vec<u64>,
}

/// Fold every PR/MR that closes an issue on the board into that issue's
/// `linked` and drop its own card; failing that, into the issues on the board
/// it mentions. A PR with neither on the board stays a card.
fn fold_linked(items: Vec<BoardItem>, closing: &[Closing]) -> Vec<BoardItem> {
    let issues: HashSet<u64> = items
        .iter()
        .filter(|i| i.kind == ItemKind::Issue)
        .map(|i| i.number)
        .collect();
    let on_board =
        |ns: &[u64]| -> Vec<u64> { ns.iter().copied().filter(|n| issues.contains(n)).collect() };
    let folded: Vec<Closing> = closing
        .iter()
        .filter_map(|c| {
            let mut targets = on_board(&c.issues);
            if targets.is_empty() {
                targets = on_board(&c.mentions);
            }
            (!targets.is_empty()).then(|| Closing {
                pr: c.pr,
                merged: c.merged,
                issues: targets,
                mentions: Vec::new(),
            })
        })
        .collect();
    let links: HashMap<u64, LinkedPr> = items
        .iter()
        .filter(|i| i.kind == ItemKind::Pr)
        .filter_map(|i| {
            let c = folded.iter().find(|c| c.pr == i.number)?;
            Some((
                i.number,
                LinkedPr {
                    number: i.number,
                    title: i.title.clone(),
                    url: i.url.clone(),
                    draft: i.draft,
                    merged: c.merged,
                    approvals: i.approvals.clone(),
                },
            ))
        })
        .collect();
    let mut out = Vec::new();
    for mut item in items {
        match item.kind {
            ItemKind::Pr if links.contains_key(&item.number) => continue,
            ItemKind::Issue => {
                item.linked = folded
                    .iter()
                    .filter(|c| c.issues.contains(&item.number))
                    .filter_map(|c| links.get(&c.pr).cloned())
                    .collect();
            }
            _ => {}
        }
        out.push(item);
    }
    out
}

/// A panicked fetch thread becomes a plain fetch error.
fn join<T>(h: ScopedJoinHandle<'_, CoreResult<T>>) -> CoreResult<T> {
    h.join()
        .unwrap_or_else(|_| Err(CoreError::IssuesFetch("fetch failed".to_string())))
}

/// Split a remote URL into `(host, path)` with `.git` and slashes stripped.
/// Handles `git@host:a/b.git`, `ssh://git@host[:port]/a/b.git` and
/// `https://host/a/b(.git)`; anything else is `None`.
/// The signed-in user's name from a "who am I" call; its failure never fails
/// the board (unassign just stays hidden).
fn me(h: ScopedJoinHandle<'_, CoreResult<String>>) -> Option<String> {
    let name = join(h).ok()?.trim().to_string();
    (!name.is_empty()).then_some(name)
}

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
    let root_arg = crate::platform::backend_path(root).map_err(|_| CoreError::IssuesNoRemote)?;
    let mut cmd = crate::platform::git_command();
    cmd.arg("-C")
        .arg(root_arg)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = crate::platform::run_gated(&mut cmd).map_err(|_| CoreError::IssuesNoRemote)?;
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
    #[serde(default)]
    assignees: Vec<GhUser>,
    /// PRs only: each reviewer's latest review.
    #[serde(default)]
    latest_reviews: Option<Vec<GhReview>>,
}

#[derive(Deserialize)]
struct GhReview {
    #[serde(default)]
    author: Option<GhUser>,
    #[serde(default)]
    state: String,
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
            movable: movable(kind, column),
            linked: Vec::new(),
            assignees: self.assignees.into_iter().map(|a| a.login).collect(),
            approvals: self.latest_reviews.map(|reviews| Approvals {
                by: reviews
                    .into_iter()
                    .filter(|r| r.state == "APPROVED")
                    .filter_map(|r| r.author.map(|a| a.login))
                    .collect(),
                required: None,
            }),
        }
    }
}

fn gh_closing(p: &GhItem, merged: bool) -> Closing {
    Closing {
        pr: p.number,
        merged,
        issues: p
            .closing_issues_references
            .iter()
            .map(|r| r.number)
            .collect(),
        // ponytail: GitHub has real sub-issues; mention fallback is GitLab-only.
        mentions: Vec::new(),
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
    let mut closing = Vec::new();
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
        closing.push(gh_closing(&p, false));
        items.push(p.into_item(ItemKind::Pr, &column));
    }
    for p in parse_gh(merged_prs)? {
        let column = place(p.number, "done");
        closing.push(gh_closing(&p, true));
        items.push(p.into_item(ItemKind::Pr, &column));
    }
    Ok(sorted(fold_linked(items, &closing)))
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
    let mut cmd = crate::platform::gh_command(gh);
    cmd.args(["auth", "token", "--hostname", host, "--user", account])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output =
        crate::platform::run_gated(&mut cmd).map_err(|_| CoreError::IssuesGhUnavailable)?;
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
    let mut cmd = crate::platform::gh_command(gh);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(token) = token {
        let var = if host == "github.com" {
            "GH_TOKEN"
        } else {
            "GH_ENTERPRISE_TOKEN"
        };
        crate::platform::backend_env(&mut cmd, var, token);
    }
    let output = crate::platform::run_gated(&mut cmd).map_err(|e| {
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
        let scope = stderr.contains("read:project")
            || stderr.contains("insufficient_scopes")
            || stderr.contains("required scope");
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

const GH_ISSUE_FIELDS: &str = "number,title,url,body,labels,updatedAt,author,assignees";
const GH_PR_FIELDS: &str = "number,title,url,body,labels,updatedAt,author,assignees,isDraft,\
closingIssuesReferences,latestReviews";
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
    let (open_issues, closed_issues, open_prs, merged_prs, counts, project, me) =
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
            let g = s.spawn(|| {
                run_gh(
                    gh,
                    host,
                    token,
                    &["api", "user", "--hostname", host, "--jq", ".login"],
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
                me(g),
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
            project_columns("No status", p.options, "Done (14d)"),
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
        me,
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
    /// MRs only.
    #[serde(default)]
    source_branch: Option<String>,
    #[serde(default)]
    assignees: Vec<GlUser>,
}

#[derive(Deserialize)]
struct GlUser {
    #[serde(default)]
    id: u64,
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
            movable: movable(kind, column),
            linked: Vec::new(),
            assignees: self.assignees.into_iter().map(|a| a.username).collect(),
            approvals: None,
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

/// Issue numbers a MR closes: GitLab's default closing pattern in the
/// description (`closes #N`, `fixes group/project#N`, `resolves: #N, #M` …,
/// case-insensitive) plus the `N-…` source branch GitLab's "create MR from
/// issue" makes.
// ponytail: heuristic, misses custom closing patterns and cross-project
// links; upgrade path = `/merge_requests/:iid/closes_issues` per MR.
fn gl_closing(m: &GlItem, project: &str) -> Vec<u64> {
    const KEYWORDS: &[&str] = &[
        "closes",
        "closed",
        "close",
        "fixes",
        "fixed",
        "fix",
        "resolves",
        "resolved",
        "resolve",
        "implements",
        "implemented",
        "implement",
    ];
    /// `#N` or `<project>#N`, then the rest.
    fn take_ref<'a>(s: &'a str, project: &str) -> Option<(u64, &'a str)> {
        let s = s.strip_prefix(project).unwrap_or(s).strip_prefix('#')?;
        let digits = s.bytes().take_while(u8::is_ascii_digit).count();
        Some((s[..digits].parse().ok()?, &s[digits..]))
    }
    let text = m.description.as_deref().unwrap_or_default().to_lowercase();
    let project = project.to_lowercase();
    let mut out = Vec::new();
    for (i, _) in text.char_indices() {
        if text[..i]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        let Some(kw) = KEYWORDS.iter().find(|k| text[i..].starts_with(**k)) else {
            continue;
        };
        let mut rest = &text[i + kw.len()..];
        if !rest.starts_with(|c: char| c == ':' || c.is_whitespace()) {
            continue;
        }
        rest = rest.strip_prefix(':').unwrap_or(rest);
        while let Some((n, after)) = take_ref(rest.trim_start(), &project) {
            out.push(n);
            let after = after.trim_start();
            match after
                .strip_prefix(',')
                .or_else(|| after.strip_prefix("and "))
            {
                Some(next) => rest = next,
                None => break,
            }
        }
    }
    // A branch segment that starts with the issue number: `42-fix`,
    // `issue/12661`, `issue/12661b`, `feature/7-x`. The first one wins.
    let branch = m.source_branch.as_deref().unwrap_or_default();
    if let Some(n) = branch.split('/').find_map(|seg| {
        let digits = seg.bytes().take_while(u8::is_ascii_digit).count();
        seg[..digits].parse::<u64>().ok()
    }) {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Every issue a MR's description refers to as `#N` or `<project>#N`; a
/// ref into another project (`other/proj#N`) or an HTML entity (`&#N`) is not
/// one.
fn gl_mentions(m: &GlItem, project: &str) -> Vec<u64> {
    let text = m.description.as_deref().unwrap_or_default().to_lowercase();
    let project = project.to_lowercase();
    let mut out = Vec::new();
    for (i, _) in text.match_indices('#') {
        let before = &text[..i];
        let ours = before.ends_with(&project)
            && !before[..before.len() - project.len()]
                .ends_with(|c: char| c.is_alphanumeric() || "/-_.".contains(c));
        if !ours && before.ends_with(|c: char| c.is_alphanumeric() || "/-_.&".contains(c)) {
            continue;
        }
        let rest = &text[i + 1..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if let Ok(n) = rest[..digits].parse::<u64>() {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// Build the items from the four GitLab list responses. The closed/merged
/// lists are filtered by `updated_after`, so the real closed/merged time is
/// checked here. With board `lists`, an open card sits in the first list whose
/// label it carries, else issue → `open`, MR → `review`.
#[allow(clippy::too_many_arguments)]
fn assemble_gitlab(
    open_issues: &str,
    closed_issues: &str,
    open_mrs: &str,
    merged_mrs: &str,
    lists: &[Column],
    project: &str,
    since: DateTime<Utc>,
    approvals: &HashMap<u64, Approvals>,
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
    let mut closing = Vec::new();
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
        closing.push(Closing {
            pr: m.iid,
            merged: false,
            issues: gl_closing(&m, project),
            mentions: gl_mentions(&m, project),
        });
        let column = if lists.is_empty() {
            pr_column(m.draft).to_string()
        } else {
            listed(&m.labels, "review")
        };
        let approved = approvals.get(&m.iid).cloned();
        let mut item = m.into_item(ItemKind::Pr, &column);
        item.approvals = approved;
        items.push(item);
    }
    for m in parse_gl(merged_mrs)? {
        if at_or_after(m.merged_at.as_deref(), since) {
            closing.push(Closing {
                pr: m.iid,
                merged: true,
                issues: gl_closing(&m, project),
                mentions: gl_mentions(&m, project),
            });
            items.push(m.into_item(ItemKind::Pr, "done"));
        }
    }
    Ok(sorted(fold_linked(items, &closing)))
}

/// Send a GitLab request, retrying connection-level failures (a VPN or Wi-Fi
/// coming back after sleep, a dropped keep-alive) twice with a short backoff.
/// A failure that made it to an HTTP status is never retried here. The error
/// is a Podium-owned reason (timed out / connection failed), never reqwest's
/// text, which carries the URL.
fn gitlab_send(
    request: impl Fn() -> reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::Response, &'static str> {
    let mut last = "connection failed";
    for delay_ms in [0, 500, 1500] {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        match request().send() {
            Ok(response) => return Ok(response),
            Err(e) => {
                tracing::debug!("GitLab request failed: {e}");
                last = if e.is_timeout() {
                    "timed out"
                } else {
                    "connection failed"
                };
            }
        }
    }
    Err(last)
}

/// One GitLab API GET: the body plus the `X-Total` header.
fn gitlab_get(base: &str, token: &str, path: &str) -> CoreResult<(String, Option<u64>)> {
    let client = reqwest::blocking::Client::new();
    let response = gitlab_send(|| {
        client
            .get(format!("{base}{path}"))
            .header("PRIVATE-TOKEN", token)
    })
    .map_err(|e| CoreError::IssuesFetch(format!("could not reach GitLab ({e})")))?;
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
/// board. When `/boards` *fails* (rather than answering without a label-list
/// board) the `fallback` columns — the last known board's — place the cards,
/// so a hiccup does not drop them into the default columns.
pub fn fetch_gitlab(
    base: &str,
    project_path: &str,
    token: &str,
    mine: bool,
    since: DateTime<Utc>,
    fallback: Option<(String, Vec<Column>)>,
) -> CoreResult<Board> {
    let p = format!("/api/v4/projects/{}", project_path.replace('/', "%2F"));
    let (issue_scope, mr_scope) = if mine {
        ("assigned_to_me", "created_by_me")
    } else {
        ("all", "all")
    };
    let after = since.format("%Y-%m-%dT%H:%M:%SZ");
    let get = |path: String| gitlab_get(base, token, &path);
    let (open_issues, closed_issues, open_mrs, merged_mrs, merged_count, boards, me) =
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
            let g = s.spawn(|| {
                get("/api/v4/user".to_string()).and_then(|(json, _)| {
                    serde_json::from_str::<GlUser>(&json)
                        .map(|u| u.username)
                        .map_err(|_| CoreError::IssuesFetch(String::new()))
                })
            });
            (join(a), join(b), join(c), join(d), join(e), join(f), me(g))
        });
    let (open_mrs, open_total) = open_mrs?;
    let (open_issues, closed_issues, merged_mrs) =
        (open_issues?.0, closed_issues?.0, merged_mrs?.0);
    let (open_mrs, merged_mrs) = if mine {
        with_related_mrs(
            base,
            token,
            &p,
            project_path,
            &[&open_issues, &closed_issues],
            open_mrs,
            merged_mrs,
        )?
    } else {
        (open_mrs, merged_mrs)
    };
    let (source, lists) = match boards {
        Ok((json, _)) => parse_gl_board(&json).map_or((None, Vec::new()), |(n, l)| (Some(n), l)),
        Err(_) => fallback.map_or((None, Vec::new()), |(n, l)| (Some(n), l)),
    };
    let approvals = gitlab_approvals(base, token, project_path, &open_mrs);
    let items = assemble_gitlab(
        &open_issues,
        &closed_issues,
        &open_mrs,
        &merged_mrs,
        &lists,
        project_path,
        since,
        &approvals,
    )?;
    let columns = if source.is_some() {
        project_columns("Open", lists, "Closed (14d)")
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
        me,
    })
}

/// Mine only: add someone else's MRs that belong to one of my issues (a
/// colleague's MR on an issue I'm co-assigned to), which `created_by_me`
/// never returns. One `related_merge_requests` call per issue on the board no
/// fetched MR links to yet; an MR is kept when [`gl_closing`] links it to that
/// issue, so a passing mention does not become a stray card. A failed lookup
/// just leaves that issue unlinked.
// ponytail: one request per unlinked issue (≤ 2×LIMIT, all in parallel); a
// GraphQL batch is the upgrade path if that gets slow.
fn with_related_mrs(
    base: &str,
    token: &str,
    p: &str,
    project_path: &str,
    issues: &[&str],
    open_mrs: String,
    merged_mrs: String,
) -> CoreResult<(String, String)> {
    let values = |json: &str| {
        serde_json::from_str::<Vec<serde_json::Value>>(json)
            .map_err(|_| CoreError::IssuesFetch("unexpected response from GitLab".to_string()))
    };
    let item = |v: &serde_json::Value| serde_json::from_value::<GlItem>(v.clone()).ok();
    let (mut open, mut merged) = (values(&open_mrs)?, values(&merged_mrs)?);
    let mut known: HashSet<u64> = HashSet::new();
    let mut linked: HashSet<u64> = HashSet::new();
    for m in open.iter().chain(&merged).filter_map(item) {
        known.insert(m.iid);
        linked.extend(gl_closing(&m, project_path));
    }
    let mut unlinked = Vec::new();
    for json in issues {
        unlinked.extend(
            parse_gl(json)?
                .into_iter()
                .map(|i| i.iid)
                .filter(|n| !linked.contains(n)),
        );
    }
    if unlinked.is_empty() {
        return Ok((open_mrs, merged_mrs));
    }
    let related: Vec<(u64, Vec<serde_json::Value>)> = std::thread::scope(|s| {
        let handles: Vec<_> = unlinked
            .iter()
            .map(|&n| {
                s.spawn(move || {
                    let path = format!("{p}/issues/{n}/related_merge_requests?per_page={LIMIT}");
                    let found = gitlab_get(base, token, &path)
                        .ok()
                        .and_then(|(json, _)| values(&json).ok())
                        .unwrap_or_default();
                    (n, found)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    for (n, mrs) in related {
        for v in mrs {
            let Some(m) = item(&v) else { continue };
            if known.contains(&m.iid) || !gl_closing(&m, project_path).contains(&n) {
                continue;
            }
            known.insert(m.iid);
            match v["state"].as_str() {
                Some("opened") => open.push(v),
                Some("merged") => merged.push(v),
                _ => {}
            }
        }
    }
    let json = |v: Vec<serde_json::Value>| serde_json::Value::Array(v).to_string();
    Ok((json(open), json(merged)))
}

const GL_APPROVALS_QUERY: &str = "query($p:ID!,$i:[String!]){project(fullPath:$p){\
mergeRequests(iids:$i,first:100){nodes{iid approvalsRequired approvedBy{nodes{username}}}}}}";

/// Approvals of the open MRs, one GraphQL call after the lists (the REST MR
/// list does not carry them). Any failure — CE without `approvalsRequired`,
/// no GraphQL — is an empty map: the board just shows no approvals.
fn gitlab_approvals(
    base: &str,
    token: &str,
    project_path: &str,
    open_mrs: &str,
) -> HashMap<u64, Approvals> {
    let iids: Vec<String> = parse_gl(open_mrs)
        .unwrap_or_default()
        .iter()
        .map(|m| m.iid.to_string())
        .collect();
    if iids.is_empty() {
        return HashMap::new();
    }
    let body = serde_json::json!({
        "query": GL_APPROVALS_QUERY,
        "variables": { "p": project_path, "i": iids },
    });
    let client = reqwest::blocking::Client::new();
    let Ok(response) = gitlab_send(|| {
        client
            .post(format!("{base}/api/graphql"))
            .header("PRIVATE-TOKEN", token)
            .header("Content-Type", "application/json")
            .body(body.to_string())
    }) else {
        return HashMap::new();
    };
    response
        .text()
        .ok()
        .map(|json| parse_gl_approvals(&json))
        .unwrap_or_default()
}

fn parse_gl_approvals(json: &str) -> HashMap<u64, Approvals> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    v["data"]["project"]["mergeRequests"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            let iid = n["iid"].as_str()?.parse().ok()?;
            let by = n["approvedBy"]["nodes"]
                .as_array()?
                .iter()
                .filter_map(|u| u["username"].as_str().map(str::to_string))
                .collect();
            let required = n["approvalsRequired"]
                .as_u64()
                .and_then(|r| u32::try_from(r).ok());
            Some((iid, Approvals { by, required }))
        })
        .collect()
}

/// The setup that serves `remote_host`: the project's `selected` one (which
/// must be for that host), else the first configured for it. There is no
/// zero-config fallback: a board is only fetched once a setup exists, so
/// Podium never shells out to `gh` for a user who never asked for a board.
fn pick<'a>(
    remote_host: &str,
    setups: &'a [IssueSetup],
    selected: Option<&'a IssueSetup>,
) -> CoreResult<&'a IssueSetup> {
    if let Some(setup) = selected {
        return if url_host(&setup.url) == remote_host {
            Ok(setup)
        } else {
            Err(CoreError::IssuesHostMismatch {
                setup: setup.name.clone(),
                remote: remote_host.to_string(),
            })
        };
    }
    setups
        .iter()
        .find(|s| url_host(&s.url) == remote_host)
        .ok_or(CoreError::IssuesUnsupportedHost)
}

/// Refuse before anything is spawned (not even the `git remote` lookup) when
/// Settings → Issues has no setup at all.
fn require_setup(setups: &[IssueSetup]) -> CoreResult<()> {
    if setups.is_empty() {
        Err(CoreError::IssuesNotConfigured)
    } else {
        Ok(())
    }
}

/// The `label:` columns of a GitLab board plus its name: what
/// [`fetch_gitlab`] reuses when `/boards` fails.
pub fn gitlab_fallback(board: &Board) -> Option<(String, Vec<Column>)> {
    let name = board
        .source
        .clone()
        .filter(|_| board.provider == IssueProvider::Gitlab)?;
    let lists = board
        .columns
        .iter()
        .filter(|c| c.id.starts_with("label:"))
        .cloned()
        .collect();
    Some((name, lists))
}

/// Detect the repo behind `root` and build its board, using the project's
/// `selected` setup or the first matching one. `cached` is the last known
/// board of this repo (GitLab column fallback).
pub fn board(
    root: &Path,
    setups: &[IssueSetup],
    selected: Option<&IssueSetup>,
    mine: bool,
    cached: Option<&Board>,
) -> CoreResult<Board> {
    require_setup(setups)?;
    let (host, path) = detect(root)?;
    let setup = pick(&host, setups, selected)?;
    let since = Utc::now() - chrono::Duration::days(DONE_DAYS);
    match setup {
        IssueSetup {
            provider: IssueProvider::Gitlab,
            id,
            url,
            ..
        } => {
            // ponytail: temporary, remove once Connect is on GitHub
            let token = gitlab_token_get(id).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            fetch_gitlab(
                url,
                &path,
                &token,
                mine,
                since,
                cached.and_then(gitlab_fallback),
            )
        }
        _ => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            let account = setup.account.as_deref();
            fetch_github(gh, &host, &path, account, mine, since)
        }
    }
}

// ---------------------------------------------------------------------------
// Moving a card
// ---------------------------------------------------------------------------

fn refused<T>(msg: &str) -> CoreResult<T> {
    Err(CoreError::IssuesMove(msg.to_string()))
}

/// Check a move of `kind` #`number` to column `to` against the board the user
/// sees; returns the item. The board's own drop rules are the authority.
pub fn validate_move<'a>(
    board: &'a Board,
    kind: ItemKind,
    number: u64,
    to: &str,
) -> CoreResult<&'a BoardItem> {
    let Some(item) = board
        .items
        .iter()
        .find(|i| i.kind == kind && i.number == number)
    else {
        return refused("That card is not on the board. Refresh the board first.");
    };
    let Some(column) = board.columns.iter().find(|c| c.id == to) else {
        return refused("Unknown column.");
    };
    if !column.accepts.contains(&kind) {
        return refused("That column does not accept this kind of card.");
    }
    if !item.movable {
        return refused("A merged PR/MR cannot be moved.");
    }
    if item.column == to {
        return refused("The card is already in that column.");
    }
    if board.source.is_none() && to == "open" && item.column != "done" {
        return refused("Without a project board, cards only return to Open from Done.");
    }
    Ok(item)
}

/// The list labels of a GitLab board (its `label:` columns).
fn list_labels(board: &Board) -> Vec<&str> {
    board
        .columns
        .iter()
        .filter_map(|c| c.id.strip_prefix("label:"))
        .collect()
}

/// Query params of the GitLab `PUT` that moves a card `from` → `to`.
fn gitlab_move_params(board: &Board, from: &str, to: &str) -> Vec<(&'static str, String)> {
    let lists = list_labels(board);
    let mut params = Vec::new();
    if let Some(label) = to.strip_prefix("label:") {
        params.push(("add_labels", label.to_string()));
        let others: Vec<_> = lists.iter().copied().filter(|l| *l != label).collect();
        if !others.is_empty() {
            params.push(("remove_labels", others.join(",")));
        }
    } else if to == "done" {
        params.push(("state_event", "close".to_string()));
    } else if !lists.is_empty() {
        params.push(("remove_labels", lists.join(",")));
    }
    if from == "done" {
        params.push(("state_event", "reopen".to_string()));
    }
    params
}

/// The item's labels once the GitLab move went through.
fn gitlab_labels_after(board: &Board, item: &BoardItem, to: &str) -> Vec<String> {
    if to == "done" {
        return item.labels.clone();
    }
    let lists = list_labels(board);
    let mut labels: Vec<String> = item
        .labels
        .iter()
        .filter(|l| !lists.contains(&l.as_str()))
        .cloned()
        .collect();
    if let Some(label) = to.strip_prefix("label:") {
        labels.push(label.to_string());
    }
    labels
}

/// One GitLab API PUT with query params (an issue/MR edit).
fn gitlab_put(
    base: &str,
    token: &str,
    path: &str,
    params: &[(&str, String)],
    failed: &str,
) -> CoreResult<()> {
    let client = reqwest::blocking::Client::new();
    let response = gitlab_send(|| {
        client
            .put(format!("{base}{path}"))
            .header("PRIVATE-TOKEN", token)
            .query(params)
    })
    .map_err(|e| CoreError::IssuesMove(format!("Could not reach GitLab ({e}).")))?;
    match response.status().as_u16() {
        200 => Ok(()),
        401 | 403 => Err(CoreError::IssuesGitlabAuth),
        429 => refused("GitLab rate limit reached, try again later."),
        _ => refused(failed),
    }
}

/// The API path of a GitLab issue/MR.
fn gitlab_item_path(project_path: &str, item: &BoardItem) -> String {
    let kind = match item.kind {
        ItemKind::Issue => "issues",
        ItemKind::Pr => "merge_requests",
    };
    format!(
        "/api/v4/projects/{}/{kind}/{}",
        project_path.replace('/', "%2F"),
        item.number
    )
}

/// Move a validated card on GitLab; returns the item's labels afterwards.
pub fn move_gitlab(
    base: &str,
    project_path: &str,
    token: &str,
    board: &Board,
    item: &BoardItem,
    to: &str,
) -> CoreResult<Vec<String>> {
    let path = gitlab_item_path(project_path, item);
    gitlab_put(
        base,
        token,
        &path,
        &gitlab_move_params(board, &item.column, to),
        "GitLab could not move the card.",
    )?;
    Ok(gitlab_labels_after(board, item, to))
}

const GH_MOVE_SCOPE_HINT: &str =
    "To move cards on your GitHub Project, run: gh auth refresh -s project";

const GH_MOVE_LOOKUP: &str = "query($owner:String!,$name:String!,$number:Int!){\
repository(owner:$owner,name:$name){projectsV2(first:1){nodes{id \
field(name:\"Status\"){... on ProjectV2SingleSelectField{id}}}} \
issueOrPullRequest(number:$number){\
... on Issue{id projectItems(first:20){nodes{id project{id}}}} \
... on PullRequest{id projectItems(first:20){nodes{id project{id}}}}}}}";
const GH_ADD_ITEM: &str = "mutation($project:ID!,$content:ID!){\
addProjectV2ItemById(input:{projectId:$project,contentId:$content}){item{id}}}";
const GH_SET_STATUS: &str = "mutation($project:ID!,$item:ID!,$field:ID!,$option:String!){\
updateProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,fieldId:$field,\
value:{singleSelectOptionId:$option}}){projectV2Item{id}}}";
const GH_CLEAR_STATUS: &str = "mutation($project:ID!,$item:ID!,$field:ID!){\
clearProjectV2ItemFieldValue(input:{projectId:$project,itemId:$item,fieldId:$field}){projectV2Item{id}}}";

/// What the move lookup found: the repo's first project, its Status field, the
/// card's own node id and its item in that project (when it is on it).
#[derive(Debug, PartialEq, Eq)]
struct GhMoveIds {
    project: String,
    field: String,
    content: String,
    item: Option<String>,
}

fn parse_gh_move_lookup(json: &str) -> Option<GhMoveIds> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let repo = &v["data"]["repository"];
    let node = repo["projectsV2"]["nodes"].get(0)?;
    let project = node["id"].as_str()?.to_string();
    let content = &repo["issueOrPullRequest"];
    let item = content["projectItems"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["project"]["id"].as_str() == Some(&project))
        .and_then(|i| i["id"].as_str())
        .map(str::to_string);
    Some(GhMoveIds {
        field: node["field"]["id"].as_str()?.to_string(),
        content: content["id"].as_str()?.to_string(),
        project,
        item,
    })
}

/// Move a validated card on GitHub: issues close/reopen, and on a project
/// board the Status field is set (or cleared for `open`/`review`). `gh` output
/// is only inspected, never forwarded.
pub fn move_github(
    gh: &str,
    host: &str,
    owner_repo: &str,
    account: Option<&str>,
    board: &Board,
    item: &BoardItem,
    to: &str,
) -> CoreResult<()> {
    let token = account.map(|a| gh_account_token(gh, host, a)).transpose()?;
    let token = token.as_deref();
    let repo = format!("{host}/{owner_repo}");
    let number = item.number.to_string();
    let run = |args: &[&str]| {
        run_gh_scoped(gh, host, token, args).map_err(|(e, scope)| match e {
            _ if scope => CoreError::IssuesMove(GH_MOVE_SCOPE_HINT.to_string()),
            CoreError::IssuesGhUnavailable => e,
            _ => CoreError::IssuesMove("gh could not move the card.".to_string()),
        })
    };
    let graphql = |query: &str, vars: &[(&str, &str)]| {
        let mut args = vec![
            "api".to_string(),
            "graphql".to_string(),
            "--hostname".to_string(),
            host.to_string(),
            "-f".to_string(),
            format!("query={query}"),
        ];
        for (k, v) in vars {
            // `number` must go as an Int, the rest as strings.
            args.push(if *k == "number" { "-F" } else { "-f" }.to_string());
            args.push(format!("{k}={v}"));
        }
        run(&args.iter().map(String::as_str).collect::<Vec<_>>())
    };
    if item.kind == ItemKind::Issue && item.column == "done" {
        run(&["issue", "reopen", &number, "--repo", &repo])?;
    }
    if to == "done" {
        run(&["issue", "close", &number, "--repo", &repo])?;
        return Ok(());
    }
    if board.source.is_none() {
        return Ok(());
    }
    let (owner, name) = owner_repo.split_once('/').unwrap_or((owner_repo, ""));
    let found = graphql(
        GH_MOVE_LOOKUP,
        &[("owner", owner), ("name", name), ("number", &number)],
    )?;
    let Some(ids) = parse_gh_move_lookup(&found) else {
        return refused("Could not find this card in your GitHub Project.");
    };
    match to.strip_prefix("status:") {
        Some(option) => {
            let item_id = match ids.item {
                Some(id) => id,
                None => {
                    let added = graphql(
                        GH_ADD_ITEM,
                        &[("project", &ids.project), ("content", &ids.content)],
                    )?;
                    let v: serde_json::Value = serde_json::from_str(&added).unwrap_or_default();
                    match v["data"]["addProjectV2ItemById"]["item"]["id"].as_str() {
                        Some(id) => id.to_string(),
                        None => return refused("Could not add the card to your GitHub Project."),
                    }
                }
            };
            graphql(
                GH_SET_STATUS,
                &[
                    ("project", &ids.project),
                    ("item", &item_id),
                    ("field", &ids.field),
                    ("option", option),
                ],
            )?;
        }
        None => {
            if let Some(item_id) = ids.item {
                graphql(
                    GH_CLEAR_STATUS,
                    &[
                        ("project", &ids.project),
                        ("item", &item_id),
                        ("field", &ids.field),
                    ],
                )?;
            }
        }
    }
    Ok(())
}

/// Move a validated card on the platform behind `root`; GitLab returns the
/// item's labels afterwards.
pub fn move_item(
    root: &Path,
    setups: &[IssueSetup],
    selected: Option<&IssueSetup>,
    board: &Board,
    item: &BoardItem,
    to: &str,
) -> CoreResult<Option<Vec<String>>> {
    require_setup(setups)?;
    let (host, path) = detect(root)?;
    let setup = pick(&host, setups, selected)?;
    match setup {
        IssueSetup {
            provider: IssueProvider::Gitlab,
            id,
            url,
            ..
        } => {
            // ponytail: temporary, remove once Connect is on GitHub
            let token = gitlab_token_get(id).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            move_gitlab(url, &path, &token, board, item, to).map(Some)
        }
        _ => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            let account = setup.account.as_deref();
            move_github(gh, &host, &path, account, board, item, to)?;
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Unassigning yourself
// ---------------------------------------------------------------------------

/// Check that the signed-in user can be unassigned from `kind` #`number` on
/// the board the user sees; returns the item and the user's name.
pub fn validate_unassign(
    board: &Board,
    kind: ItemKind,
    number: u64,
) -> CoreResult<(&BoardItem, &str)> {
    let Some(item) = board
        .items
        .iter()
        .find(|i| i.kind == kind && i.number == number)
    else {
        return refused("That card is not on the board. Refresh the board first.");
    };
    let Some(me) = board.me.as_deref() else {
        return refused("Refresh the board first.");
    };
    if !item.assignees.iter().any(|a| a == me) {
        return refused("You are not assigned to that card.");
    }
    Ok((item, me))
}

/// The `assignee_ids` params that keep everyone but `me` (`0` = nobody, which
/// is how GitLab spells "unassign all").
fn gitlab_unassign_params(json: &str, me: &str) -> CoreResult<Vec<(&'static str, String)>> {
    #[derive(Deserialize)]
    struct Assigned {
        #[serde(default)]
        assignees: Vec<GlUser>,
    }
    let assigned: Assigned = serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesMove("Unexpected response from GitLab.".to_string()))?;
    let rest: Vec<_> = assigned
        .assignees
        .iter()
        .filter(|a| a.username != me)
        .map(|a| ("assignee_ids[]", a.id.to_string()))
        .collect();
    Ok(if rest.is_empty() {
        vec![("assignee_ids", "0".to_string())]
    } else {
        rest
    })
}

/// Remove `me` from a GitLab issue/MR's assignees, reading them fresh first so
/// a co-assignee added since the last fetch is kept.
pub fn unassign_gitlab(
    base: &str,
    project_path: &str,
    token: &str,
    item: &BoardItem,
    me: &str,
) -> CoreResult<()> {
    let path = gitlab_item_path(project_path, item);
    let (json, _) = gitlab_get(base, token, &path).map_err(|e| match e {
        CoreError::IssuesGitlabAuth => e,
        _ => CoreError::IssuesMove("Could not read the card from GitLab.".to_string()),
    })?;
    gitlab_put(
        base,
        token,
        &path,
        &gitlab_unassign_params(&json, me)?,
        "GitLab could not unassign you.",
    )
}

/// Remove `me` from a GitHub issue/PR's assignees (PRs are issues to this
/// REST endpoint). `gh` output is only inspected, never forwarded.
pub fn unassign_github(
    gh: &str,
    host: &str,
    owner_repo: &str,
    account: Option<&str>,
    item: &BoardItem,
    me: &str,
) -> CoreResult<()> {
    let token = account.map(|a| gh_account_token(gh, host, a)).transpose()?;
    run_gh(
        gh,
        host,
        token.as_deref(),
        &[
            "api",
            "--method",
            "DELETE",
            "--hostname",
            host,
            &format!("repos/{owner_repo}/issues/{}/assignees", item.number),
            "-f",
            &format!("assignees[]={me}"),
        ],
    )
    .map_err(|e| match e {
        CoreError::IssuesGhUnavailable => e,
        _ => CoreError::IssuesMove("gh could not unassign you.".to_string()),
    })?;
    Ok(())
}

/// Unassign the signed-in user from a validated card on the platform behind
/// `root`.
pub fn unassign_item(
    root: &Path,
    setups: &[IssueSetup],
    selected: Option<&IssueSetup>,
    item: &BoardItem,
    me: &str,
) -> CoreResult<()> {
    require_setup(setups)?;
    let (host, path) = detect(root)?;
    match pick(&host, setups, selected)? {
        IssueSetup {
            provider: IssueProvider::Gitlab,
            id,
            url,
            ..
        } => {
            // ponytail: temporary, remove once Connect is on GitHub
            let token = gitlab_token_get(id).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            unassign_gitlab(url, &path, &token, item, me)
        }
        setup => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            let account = setup.account.as_deref();
            unassign_github(gh, &host, &path, account, item, me)
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
        // #20 closes #8, which is on the board: folded into it, no card of its own.
        assert!(!items
            .iter()
            .any(|i| i.kind == ItemKind::Pr && i.number == 20));
        let eight = items.iter().find(|i| i.number == 8).unwrap();
        assert_eq!(eight.linked.len(), 1);
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
        let items = assemble_gitlab(
            open_issues,
            closed,
            open_mrs,
            merged,
            &[],
            "grp/proj",
            since(),
            &HashMap::new(),
        )
        .unwrap();
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
            assemble_gitlab(
                "{}",
                "[]",
                "[]",
                "[]",
                &[],
                "grp/proj",
                since(),
                &HashMap::new()
            ),
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

        // No zero-config github.com any more, and unknown hosts.
        assert!(matches!(
            pick("github.com", std::slice::from_ref(&gl), None),
            Err(CoreError::IssuesUnsupportedHost)
        ));
        assert!(matches!(
            pick("other.test", &setups, None),
            Err(CoreError::IssuesUnsupportedHost)
        ));
        // Auto: first setup whose host matches.
        assert_eq!(pick("git.example.test", &setups, None).unwrap(), &gl);
        assert_eq!(pick("github.com", &setups, None).unwrap(), &gh2);
        // An explicit pick wins, but must be for the remote's host.
        assert_eq!(pick("github.com", &setups, Some(&gh2)).unwrap(), &gh2);
        assert!(matches!(
            pick("github.com", &setups, Some(&gl)),
            Err(CoreError::IssuesHostMismatch { .. })
        ));
    }

    #[test]
    fn no_setup_refuses_before_spawning_anything() {
        // Not a git repo: reaching `detect` would fail with IssuesNoRemote.
        let dir = tempfile::tempdir().unwrap();
        let item = item(ItemKind::Issue, 1, "open");
        assert!(matches!(
            board(dir.path(), &[], None, true, None),
            Err(CoreError::IssuesNotConfigured)
        ));
        assert!(matches!(
            unassign_item(dir.path(), &[], None, &item, "someone"),
            Err(CoreError::IssuesNotConfigured)
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
                let body = if !status.starts_with("200") {
                    "secret-ish body"
                } else if req.starts_with("GET /api/v4/user ") {
                    r#"{"id":5,"username":"fixture-me"}"#
                } else if req.starts_with("GET ") && req.contains("/issues/2 ") {
                    r#"{"assignees":[{"id":5,"username":"fixture-me"},{"id":6,"username":"kim"}]}"#
                } else if req.starts_with("GET ") && req.contains("/merge_requests/10 ") {
                    r#"{"assignees":[{"id":5,"username":"fixture-me"}]}"#
                } else {
                    "[]"
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
        let (base, seen) = serve_board("200 OK", 7);
        let board = fetch_gitlab(&base, "grp/sub/proj", "fake-token", true, since(), None).unwrap();
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

        let (base, seen) = serve_board("200 OK", 7);
        fetch_gitlab(&base, "a/b", "t", false, since(), None).unwrap();
        assert!(seen
            .join()
            .unwrap()
            .iter()
            .filter(|r| !r.contains("/boards ") && !r.contains("/user "))
            .all(|r| r.contains("scope=all")));
    }

    #[test]
    fn gitlab_mine_links_a_colleagues_mr_via_related_merge_requests() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        // Detached: answers until the test process ends.
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let mut sock = sock.unwrap();
                let mut buf = [0u8; 4096];
                let got = sock.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..got]).into_owned();
                let body = if req.starts_with("GET /api/v4/user ") {
                    r#"{"id":5,"username":"fixture-me"}"#
                } else if req.contains("/issues?state=opened") {
                    r#"[{"iid":12,"title":"mine","web_url":"u12"}]"#
                } else if req.contains("/issues/12/related_merge_requests") {
                    // A colleague's merged MR on the `issue/12` branch, and one
                    // that only mentions the issue.
                    r#"[{"iid":40,"title":"fix - #12","web_url":"u40","state":"merged",
                        "source_branch":"issue/12","merged_at":"2026-09-29T11:00:00Z",
                        "author":{"username":"kim"}},
                       {"iid":41,"title":"see #12","web_url":"u41","state":"opened",
                        "source_branch":"other"}]"#
                } else {
                    "[]"
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(reply.as_bytes()).unwrap();
                log.lock().unwrap().push(req);
            }
        });
        let board = fetch_gitlab(&base, "a/b", "t", true, since(), None).unwrap();
        assert_eq!(board.items.len(), 1, "no stray card for the mention");
        let linked = &board.items[0].linked;
        assert_eq!(linked.len(), 1);
        assert_eq!((linked[0].number, linked[0].merged), (40, true));

        // The All board lists every MR already, so it looks nothing up.
        seen.lock().unwrap().clear();
        fetch_gitlab(&base, "a/b", "t", false, since(), None).unwrap();
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.contains("related_merge_requests")));
    }

    #[test]
    fn gitlab_fetch_maps_status() {
        for code in ["401 Unauthorized", "403 Forbidden"] {
            let (base, _h) = serve_board(code, 7);
            assert!(matches!(
                fetch_gitlab(&base, "a/b", "wrong", true, since(), None),
                Err(CoreError::IssuesGitlabAuth)
            ));
        }
        let (base, _h) = serve_board("500 Internal Server Error", 7);
        let err = fetch_gitlab(&base, "a/b", "t", true, since(), None).unwrap_err();
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
            me: Some("octo".into()),
        })
        .unwrap();
        assert_eq!(v["me"], "octo");
        assert_eq!(v["items"][0]["assignees"], serde_json::json!([]));
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
        let items = assemble_gitlab(
            open_issues,
            closed,
            open_mrs,
            merged,
            &lists,
            "grp/proj",
            since(),
            &HashMap::new(),
        )
        .unwrap();
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
        // Project boards always carry review and done, cards or not.
        let ids: Vec<_> = project_columns("Open", lists.clone(), "Closed (14d)")
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            ids,
            ["open", "label:doing", "label:review-me", "review", "done"]
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
        let ids: Vec<_> = project_columns("No status", p.options.clone(), "Done (14d)")
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

    fn item(kind: ItemKind, number: u64, column: &str) -> BoardItem {
        BoardItem {
            kind,
            number,
            title: format!("card {number}"),
            url: format!("u{number}"),
            body: String::new(),
            labels: vec!["bug".into(), "doing".into()],
            updated_at: String::new(),
            column: column.into(),
            draft: false,
            author: String::new(),
            movable: movable(kind, column),
            linked: Vec::new(),
            assignees: vec!["fixture-me".into()],
            approvals: None,
        }
    }

    fn gl_board() -> Board {
        let (_, lists) = parse_gl_board(GL_BOARDS).unwrap();
        Board {
            provider: IssueProvider::Gitlab,
            items: vec![
                item(ItemKind::Issue, 1, "label:doing"),
                item(ItemKind::Issue, 2, "open"),
                item(ItemKind::Issue, 3, "done"),
                item(ItemKind::Pr, 10, "review"),
                item(ItemKind::Pr, 11, "done"),
            ],
            columns: project_columns("Open", lists, "Closed (14d)"),
            source: Some("Development".into()),
            hint: None,
            pr_counts: PrCounts::default(),
            me: Some("fixture-me".into()),
        }
    }

    fn accepts(cols: &[Column], id: &str) -> Vec<ItemKind> {
        cols.iter().find(|c| c.id == id).unwrap().accepts.clone()
    }

    #[test]
    fn drop_rules_and_movable() {
        use ItemKind::{Issue, Pr};
        let d = default_columns();
        assert_eq!(accepts(&d, "open"), [Issue]);
        assert_eq!(accepts(&d, "inProgress"), []);
        assert_eq!(accepts(&d, "review"), []);
        assert_eq!(accepts(&d, "done"), [Issue]);
        let b = gl_board();
        assert_eq!(accepts(&b.columns, "open"), [Issue]);
        assert_eq!(accepts(&b.columns, "label:doing"), [Issue, Pr]);
        assert_eq!(accepts(&b.columns, "review"), [Pr]);
        assert_eq!(accepts(&b.columns, "done"), [Issue]);
        let gh = project_columns(
            "No status",
            vec![col("status:o1", "Todo", None)],
            "Done (14d)",
        );
        assert_eq!(accepts(&gh, "status:o1"), [Issue, Pr]);
        // Only a merged PR/MR is fixed in place.
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(
            v["columns"][1]["accepts"],
            serde_json::json!(["issue", "pr"])
        );
        let movable: Vec<_> = b.items.iter().map(|i| i.movable).collect();
        assert_eq!(movable, [true, true, true, true, false]);
        assert_eq!(v["items"][4]["movable"], false);
        // The wire round-trips.
        assert_eq!(serde_json::from_value::<Board>(v).unwrap(), b);
    }

    #[test]
    fn move_validation() {
        use ItemKind::{Issue, Pr};
        let b = gl_board();
        let refusal = |kind, n, to: &str| match validate_move(&b, kind, n, to) {
            Err(CoreError::IssuesMove(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(validate_move(&b, Issue, 2, "label:doing").is_ok());
        assert!(validate_move(&b, Pr, 10, "label:review-me").is_ok());
        assert!(validate_move(&b, Issue, 3, "open").is_ok());
        assert!(refusal(Issue, 99, "open").contains("Refresh"));
        assert!(refusal(Issue, 2, "nowhere").contains("Unknown"));
        assert!(refusal(Issue, 2, "review").contains("does not accept"));
        assert!(refusal(Pr, 10, "done").contains("does not accept"));
        assert!(refusal(Pr, 11, "label:doing").contains("merged"));
        assert!(refusal(Issue, 1, "label:doing").contains("already"));
        // Kind and number are one key: PR #2 is not issue #2.
        assert!(refusal(Pr, 2, "review").contains("Refresh"));
        // Default board: Open is only reachable from Done.
        let mut d = b.clone();
        d.source = None;
        d.columns = default_columns();
        d.items[0].column = "inProgress".into();
        assert!(validate_move(&d, Issue, 3, "open").is_ok());
        assert!(matches!(
            validate_move(&d, Issue, 1, "open"),
            Err(CoreError::IssuesMove(_))
        ));
        assert!(matches!(
            validate_move(&d, Issue, 2, "inProgress"),
            Err(CoreError::IssuesMove(_))
        ));
    }

    #[test]
    fn gitlab_move_params_table() {
        let b = gl_board();
        let p = |from, to| gitlab_move_params(&b, from, to);
        let s = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let got = |v: Vec<(&str, String)>| -> Vec<(String, String)> {
            v.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        assert_eq!(
            got(p("open", "label:doing")),
            s(&[("add_labels", "doing"), ("remove_labels", "review-me")])
        );
        assert_eq!(
            got(p("label:doing", "label:review-me")),
            s(&[("add_labels", "review-me"), ("remove_labels", "doing")])
        );
        assert_eq!(
            got(p("label:doing", "open")),
            s(&[("remove_labels", "doing,review-me")])
        );
        assert_eq!(
            got(p("label:doing", "review")),
            s(&[("remove_labels", "doing,review-me")])
        );
        assert_eq!(got(p("open", "done")), s(&[("state_event", "close")]));
        assert_eq!(
            got(p("done", "label:doing")),
            s(&[
                ("add_labels", "doing"),
                ("remove_labels", "review-me"),
                ("state_event", "reopen")
            ])
        );
        // No label lists (default board): reopen only.
        let mut d = b.clone();
        d.columns = default_columns();
        assert_eq!(
            got(gitlab_move_params(&d, "done", "open")),
            s(&[("state_event", "reopen")])
        );
        // Labels afterwards: list labels swapped, the rest kept.
        let issue = &b.items[0];
        assert_eq!(
            gitlab_labels_after(&b, issue, "label:review-me"),
            ["bug", "review-me"]
        );
        assert_eq!(gitlab_labels_after(&b, issue, "open"), ["bug"]);
        assert_eq!(gitlab_labels_after(&b, issue, "done"), ["bug", "doing"]);
    }

    #[test]
    fn gitlab_move_sends_put_with_params() {
        let b = gl_board();
        let (base, seen) = serve_board("200 OK", 1);
        let labels = move_gitlab(
            &base,
            "grp/sub/proj",
            "fake-token",
            &b,
            &b.items[1],
            "label:doing",
        )
        .unwrap();
        assert_eq!(labels, ["bug", "doing"]);
        let req = seen.join().unwrap().remove(0);
        let line = req.lines().next().unwrap();
        assert!(
            line.starts_with("PUT /api/v4/projects/grp%2Fsub%2Fproj/issues/2?add_labels=doing&remove_labels=review-me "),
            "{line}"
        );
        assert!(req.to_lowercase().contains("private-token: fake-token"));

        // An MR goes to /merge_requests, a close to state_event.
        let (base, seen) = serve_board("200 OK", 1);
        move_gitlab(&base, "a/b", "t", &b, &b.items[1], "done").unwrap();
        assert!(seen.join().unwrap()[0]
            .starts_with("PUT /api/v4/projects/a%2Fb/issues/2?state_event=close "));
        let (base, seen) = serve_board("200 OK", 1);
        move_gitlab(&base, "a/b", "t", &b, &b.items[3], "label:doing").unwrap();
        assert!(
            seen.join().unwrap()[0].starts_with("PUT /api/v4/projects/a%2Fb/merge_requests/10?")
        );

        let (base, _h) = serve_board("403 Forbidden", 1);
        assert!(matches!(
            move_gitlab(&base, "a/b", "t", &b, &b.items[1], "done"),
            Err(CoreError::IssuesGitlabAuth)
        ));
        let (base, _h) = serve_board("500 Internal Server Error", 1);
        let err = move_gitlab(&base, "a/b", "t", &b, &b.items[1], "done").unwrap_err();
        assert!(matches!(err, CoreError::IssuesMove(_)));
        assert!(!err.to_string().contains("secret-ish"));
    }

    #[test]
    fn gitlab_fetch_knows_who_i_am() {
        let (base, _seen) = serve_board("200 OK", 7);
        let b = fetch_gitlab(&base, "a/b", "t", true, since(), None).unwrap();
        assert_eq!(b.me.as_deref(), Some("fixture-me"));
    }

    #[test]
    fn unassign_validation() {
        let mut b = gl_board();
        assert_eq!(
            validate_unassign(&b, ItemKind::Issue, 2).unwrap().1,
            "fixture-me"
        );
        assert!(validate_unassign(&b, ItemKind::Issue, 99).is_err());
        b.items[1].assignees = vec!["kim".into()];
        assert!(validate_unassign(&b, ItemKind::Issue, 2)
            .unwrap_err()
            .to_string()
            .contains("not assigned"));
        b.me = None;
        assert!(validate_unassign(&b, ItemKind::Issue, 1).is_err());
    }

    #[test]
    fn gitlab_unassign_keeps_the_others() {
        let b = gl_board();
        // Co-assignee kim stays.
        let (base, seen) = serve_board("200 OK", 2);
        unassign_gitlab(&base, "a/b", "t", &b.items[1], "fixture-me").unwrap();
        let reqs = seen.join().unwrap();
        assert!(reqs[0].starts_with("GET /api/v4/projects/a%2Fb/issues/2 "));
        assert!(
            reqs[1].starts_with("PUT /api/v4/projects/a%2Fb/issues/2?assignee_ids%5B%5D=6 "),
            "{}",
            reqs[1]
        );
        // Last assignee: 0 means nobody.
        let (base, seen) = serve_board("200 OK", 2);
        unassign_gitlab(&base, "a/b", "t", &b.items[3], "fixture-me").unwrap();
        assert!(seen.join().unwrap()[1]
            .starts_with("PUT /api/v4/projects/a%2Fb/merge_requests/10?assignee_ids=0 "));
        let (base, _h) = serve_board("403 Forbidden", 1);
        assert!(matches!(
            unassign_gitlab(&base, "a/b", "t", &b.items[1], "fixture-me"),
            Err(CoreError::IssuesGitlabAuth)
        ));
    }

    #[test]
    fn gitlab_retries_a_dropped_connection() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            // First connection is dropped unanswered, the retry gets a reply.
            drop(listener.accept().unwrap());
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = sock.read(&mut buf).unwrap();
            sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
                .unwrap();
        });
        assert_eq!(gitlab_get(&base, "t", "/x").unwrap().0, "[]");
        h.join().unwrap();
    }

    /// Answers `/boards` with a 500 and everything else with `[]` (the MR
    /// list gets an `X-Total`).
    fn serve_boards_down(n: usize) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for _ in 0..n {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let got = sock.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..got]);
                let (status, body) = if req.contains("/boards ") {
                    ("500 Internal Server Error", "boom")
                } else {
                    ("200 OK", "[]")
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nX-Total: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(reply.as_bytes()).unwrap();
            }
        });
        base
    }

    #[test]
    fn gitlab_reuses_cached_columns_when_boards_fails() {
        let known = gl_board();
        let fallback = gitlab_fallback(&known);
        assert_eq!(
            fallback.as_ref().map(|(n, l)| (n.as_str(), l.len())),
            Some(("Development", 2))
        );
        let base = serve_boards_down(7);
        let b = fetch_gitlab(&base, "a/b", "t", true, since(), fallback).unwrap();
        assert_eq!(b.source.as_deref(), Some("Development"));
        assert_eq!(b.columns, known.columns);
        // /boards succeeding without a label-list board is not a failure.
        let (base, _seen) = serve_board("200 OK", 7);
        let b = fetch_gitlab(&base, "a/b", "t", true, since(), gitlab_fallback(&known)).unwrap();
        assert_eq!((b.source, b.columns), (None, default_columns()));
        // No cache, no fallback: defaults, as before.
        let base = serve_boards_down(7);
        let b = fetch_gitlab(&base, "a/b", "t", true, since(), None).unwrap();
        assert_eq!(b.columns, default_columns());
        // A GitHub board never seeds GitLab columns.
        let mut gh = known;
        gh.provider = IssueProvider::Github;
        assert_eq!(gitlab_fallback(&gh), None);
    }

    #[test]
    fn closing_references() {
        let mr = |description: &str, branch: &str| GlItem {
            iid: 1,
            title: String::new(),
            web_url: String::new(),
            description: Some(description.into()),
            labels: vec![],
            updated_at: String::new(),
            author: None,
            draft: false,
            merge_requests_count: 0,
            closed_at: None,
            merged_at: None,
            source_branch: Some(branch.into()),
            assignees: vec![],
        };
        let n = |d: &str, b: &str| gl_closing(&mr(d, b), "grp/proj");
        assert_eq!(n("Closes #12", "x"), [12]);
        assert_eq!(n("fixes: #3, #4 and #5.", "x"), [3, 4, 5]);
        assert_eq!(n("RESOLVED Grp/Proj#7", "x"), [7]);
        assert_eq!(n("Implements #8\n\nFixed #9", "x"), [8, 9]);
        assert_eq!(n("", "42-fix-invoice"), [42]);
        assert_eq!(n("closes #1", "2-b"), [1, 2]);
        // A branch segment starting with the number (Connect: `issue/N`).
        assert_eq!(n("", "issue/12661"), [12661]);
        assert_eq!(n("", "issue/12661b"), [12661]);
        assert_eq!(n("", "feature/7-x"), [7]);
        assert_eq!(n("closes #5", "issue/5"), [5]);
        // Other projects, plain mentions, lookalike words and branches.
        assert!(n("closes other/proj#7", "feature/x").is_empty());
        assert!(n("see #7, prefix #8, fixture #9", "x").is_empty());
        assert!(n("closes #", "drink-7up").is_empty());

        let m = |d: &str| gl_mentions(&mr(d, "x"), "grp/proj");
        assert_eq!(m("Closes #16234\nOnderdeel van #16225"), [16234, 16225]);
        assert_eq!(m("(#3) grp/proj#4, #3"), [3, 4]);
        assert!(m("other/proj#7 &#39; issue#8 #").is_empty());
    }

    #[test]
    fn gitlab_folds_a_sub_issue_mr_into_the_parent_it_mentions() {
        // #20 closes sub-issue #5 (not on the board) and mentions parent #4
        // (on it); #21 closes #4 and only mentions #6; #22 mentions nothing
        // on the board.
        let issues = r#"[{"iid":4,"title":"parent","web_url":"u4"},
                         {"iid":6,"title":"other","web_url":"u6"}]"#;
        let mrs = r#"[{"iid":20,"title":"a","web_url":"u20","source_branch":"issue/5",
                       "description":"Closes #5\nOnderdeel van #4"},
                      {"iid":21,"title":"b","web_url":"u21","description":"Closes #4, see #6"},
                      {"iid":22,"title":"c","web_url":"u22","description":"see #99"}]"#;
        let items = assemble_gitlab(
            issues,
            "[]",
            mrs,
            "[]",
            &[],
            "a/b",
            since(),
            &HashMap::new(),
        )
        .unwrap();
        let linked = |n: u64| -> Vec<u64> {
            let i = items
                .iter()
                .find(|i| i.kind == ItemKind::Issue && i.number == n);
            i.unwrap().linked.iter().map(|l| l.number).collect()
        };
        assert_eq!(linked(4), [20, 21]);
        assert!(linked(6).is_empty(), "a mention never beats a close");
        let prs: Vec<u64> = items
            .iter()
            .filter(|i| i.kind == ItemKind::Pr)
            .map(|i| i.number)
            .collect();
        assert_eq!(prs, [22]);
    }

    #[test]
    fn github_folds_linked_prs_into_their_issue() {
        let items = assemble_github(
            r#"[{"number":7,"title":"a","url":"u7"},{"number":8,"title":"b","url":"u8"}]"#,
            "[]",
            r#"[{"number":20,"title":"fix 7 and 8","url":"u20","isDraft":true,
                 "closingIssuesReferences":[{"number":7},{"number":8}],
                 "latestReviews":[{"author":{"login":"kim"},"state":"APPROVED"},
                                  {"author":{"login":"sam"},"state":"CHANGES_REQUESTED"}]},
                {"number":21,"title":"off board","url":"u21",
                 "closingIssuesReferences":[{"number":99}]}]"#,
            r#"[{"number":19,"title":"merged","url":"u19","closingIssuesReferences":[{"number":7}]}]"#,
            None,
        )
        .unwrap();
        let numbers: Vec<_> = items.iter().map(|i| (i.kind, i.number)).collect();
        // #20 and #19 are folded away; #21 links an issue not on the board.
        assert_eq!(items.len(), 3, "{numbers:?}");
        assert!(items
            .iter()
            .any(|i| i.kind == ItemKind::Pr && i.number == 21));
        let issue = |n| {
            items
                .iter()
                .find(|i| i.kind == ItemKind::Issue && i.number == n)
                .unwrap()
        };
        let linked: Vec<_> = issue(7)
            .linked
            .iter()
            .map(|l| (l.number, l.draft, l.merged))
            .collect();
        assert_eq!(linked, [(20, true, false), (19, false, true)]);
        assert_eq!(issue(8).linked.len(), 1);
        assert_eq!(issue(8).linked[0].url, "u20");
        // The default placement still counts the open PR.
        assert_eq!(issue(7).column, "inProgress");
        let v = serde_json::to_value(issue(8)).unwrap();
        assert_eq!(v["linked"][0]["title"], "fix 7 and 8");
        assert_eq!(v["linked"][0]["merged"], false);
        // Only APPROVED reviews count; GitHub does not say how many are needed.
        assert_eq!(
            issue(8).linked[0].approvals,
            Some(Approvals {
                by: vec!["kim".into()],
                required: None
            })
        );
    }

    #[test]
    fn gitlab_approvals_parse_and_land_on_linked_mrs() {
        let map = parse_gl_approvals(
            r#"{"data":{"project":{"mergeRequests":{"nodes":[
                {"iid":"40","approvalsRequired":2,"approvedBy":{"nodes":[{"username":"kim"}]}},
                {"iid":"41","approvalsRequired":null,"approvedBy":{"nodes":[]}}]}}}}"#,
        );
        assert_eq!(map[&41].required, None);
        assert!(parse_gl_approvals(r#"{"errors":[{"message":"nope"}]}"#).is_empty());
        let items = assemble_gitlab(
            r#"[{"iid":12,"title":"a","web_url":"u12"}]"#,
            "[]",
            r#"[{"iid":40,"title":"m","web_url":"u40","source_branch":"issue/12"}]"#,
            "[]",
            &[],
            "grp/proj",
            since(),
            &map,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].linked[0].approvals,
            Some(Approvals {
                by: vec!["kim".into()],
                required: Some(2)
            })
        );
    }

    #[test]
    fn gitlab_folds_linked_mrs_into_their_issue() {
        let items = assemble_gitlab(
            r#"[{"iid":12,"title":"a","web_url":"u12"},{"iid":13,"title":"b","web_url":"u13"}]"#,
            "[]",
            r#"[{"iid":40,"title":"by text","web_url":"u40","description":"Closes #12"},
                {"iid":41,"title":"by branch","web_url":"u41","source_branch":"13-fix-it"},
                {"iid":42,"title":"other issue","web_url":"u42","description":"closes #99"}]"#,
            r#"[{"iid":38,"title":"merged","web_url":"u38","description":"fixes grp/proj#12",
                 "merged_at":"2026-09-21T10:00:00Z"}]"#,
            &[],
            "grp/proj",
            since(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(items.len(), 3);
        let mr = items.iter().find(|i| i.kind == ItemKind::Pr).unwrap();
        assert_eq!(mr.number, 42);
        let issue = |n| {
            items
                .iter()
                .find(|i| i.kind == ItemKind::Issue && i.number == n)
                .unwrap()
        };
        let linked: Vec<_> = issue(12)
            .linked
            .iter()
            .map(|l| (l.number, l.merged))
            .collect();
        assert_eq!(linked, [(40, false), (38, true)]);
        assert_eq!(issue(13).linked[0].number, 41);
        // The linked open MR keeps the default board's issue in progress
        // (via merge_requests_count, which the real API reports).
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
if [ "$2" = user ]; then echo octo-fixture; exit 0; fi
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
            assert_eq!(board.me.as_deref(), Some("octo-fixture"));
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

        const LOOKUP_ON: &str = r#"{"data":{"repository":{"projectsV2":{"nodes":[{"id":"PVT1","field":{"id":"FLD1"}}]},"issueOrPullRequest":{"id":"ISS7","projectItems":{"nodes":[{"id":"OTHER","project":{"id":"PVT9"}},{"id":"ITEM7","project":{"id":"PVT1"}}]}}}}}"#;
        const LOOKUP_OFF: &str = r#"{"data":{"repository":{"projectsV2":{"nodes":[{"id":"PVT1","field":{"id":"FLD1"}}]},"issueOrPullRequest":{"id":"ISS7","projectItems":{"nodes":[]}}}}}"#;

        /// A fake `gh` that logs its args and answers the move queries.
        fn mover(dir: &Path, lookup: &str, fail: &str) -> (String, std::path::PathBuf) {
            let log = dir.join("calls.log");
            let body = format!(
                "echo \"$*\" >> {log}\n{fail}\n\
                 case \"$*\" in\n\
                 *addProjectV2ItemById*) echo '{{\"data\":{{\"addProjectV2ItemById\":{{\"item\":{{\"id\":\"NEWITEM\"}}}}}}}}';;\n\
                 *projectsV2*) echo '{lookup}';;\n\
                 esac",
                log = log.display()
            );
            (script(dir, &body), log)
        }

        fn gh_board() -> Board {
            Board {
                provider: IssueProvider::Github,
                items: vec![
                    item(ItemKind::Issue, 7, "open"),
                    item(ItemKind::Issue, 8, "done"),
                    item(ItemKind::Issue, 9, "status:o1"),
                ],
                columns: project_columns(
                    "No status",
                    vec![
                        col("status:o1", "Todo", None),
                        col("status:o2", "Doing", None),
                    ],
                    "Done (14d)",
                ),
                source: Some("Widgets roadmap".into()),
                hint: None,
                pr_counts: PrCounts::default(),
                me: Some("octo".into()),
            }
        }

        fn calls(log: &Path) -> Vec<String> {
            std::fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn go(gh: &str, board: &Board, n: usize, to: &str) -> CoreResult<()> {
            move_github(
                gh,
                "github.com",
                "acme/widgets",
                None,
                board,
                &board.items[n],
                to,
            )
        }

        #[test]
        fn github_move_sets_status_on_the_project() {
            let dir = tempfile::tempdir().unwrap();
            let b = gh_board();
            // Card already on the project: lookup, then set — no add.
            let (gh, log) = mover(dir.path(), LOOKUP_ON, "");
            go(&gh, &b, 0, "status:o2").unwrap();
            let c = calls(&log);
            assert_eq!(c.len(), 2, "{c:?}");
            assert!(c[0].contains("issueOrPullRequest") && c[0].contains("number=7"));
            assert!(c[0].contains("-F number=7") && c[0].contains("owner=acme"));
            assert!(c[1].contains("updateProjectV2ItemFieldValue"));
            for want in ["project=PVT1", "item=ITEM7", "field=FLD1", "option=o2"] {
                assert!(c[1].contains(want), "{want} in {}", c[1]);
            }

            // Not on the project yet: it is added first (by node id).
            std::fs::remove_file(&log).unwrap();
            let (gh, log) = mover(dir.path(), LOOKUP_OFF, "");
            go(&gh, &b, 0, "status:o1").unwrap();
            let c = calls(&log);
            assert_eq!(c.len(), 3, "{c:?}");
            assert!(c[1].contains("addProjectV2ItemById") && c[1].contains("content=ISS7"));
            assert!(c[2].contains("item=NEWITEM") && c[2].contains("option=o1"));

            // To Open/Review the status is cleared, when there is one.
            std::fs::remove_file(&log).unwrap();
            let (gh, log) = mover(dir.path(), LOOKUP_ON, "");
            go(&gh, &b, 2, "open").unwrap();
            let c = calls(&log);
            assert!(c[1].contains("clearProjectV2ItemFieldValue") && c[1].contains("item=ITEM7"));
            std::fs::remove_file(&log).unwrap();
            let (gh, log) = mover(dir.path(), LOOKUP_OFF, "");
            go(&gh, &b, 2, "open").unwrap();
            assert_eq!(calls(&log).len(), 1);
        }

        #[test]
        fn github_move_closes_and_reopens() {
            let dir = tempfile::tempdir().unwrap();
            let b = gh_board();
            let (gh, log) = mover(dir.path(), LOOKUP_ON, "");
            go(&gh, &b, 0, "done").unwrap();
            assert_eq!(
                calls(&log),
                ["issue close 7 --repo github.com/acme/widgets"]
            );

            // Out of Done: reopen first, then the Status.
            std::fs::remove_file(&log).unwrap();
            let (gh, log) = mover(dir.path(), LOOKUP_ON, "");
            go(&gh, &b, 1, "status:o2").unwrap();
            let c = calls(&log);
            assert_eq!(c[0], "issue reopen 8 --repo github.com/acme/widgets");
            assert!(c[2].contains("updateProjectV2ItemFieldValue"));

            // Default board: reopen is the whole move.
            let mut d = b.clone();
            d.source = None;
            d.columns = default_columns();
            std::fs::remove_file(&log).unwrap();
            let (gh, log) = mover(dir.path(), LOOKUP_ON, "");
            go(&gh, &d, 1, "open").unwrap();
            assert_eq!(
                calls(&log),
                ["issue reopen 8 --repo github.com/acme/widgets"]
            );
        }

        #[test]
        fn github_unassign_deletes_me_from_assignees() {
            let dir = tempfile::tempdir().unwrap();
            let b = gh_board();
            let (gh, log) = mover(dir.path(), "", "");
            unassign_github(&gh, "github.com", "acme/widgets", None, &b.items[0], "octo").unwrap();
            assert_eq!(
                calls(&log),
                ["api --method DELETE --hostname github.com repos/acme/widgets/issues/7/assignees -f assignees[]=octo"]
            );
            let (gh, _log) = mover(dir.path(), "", "echo nope >&2; exit 1");
            let err = unassign_github(&gh, "github.com", "acme/widgets", None, &b.items[0], "octo")
                .unwrap_err();
            assert_eq!(err.to_string(), "gh could not unassign you.");
        }

        #[test]
        fn github_move_errors_are_podium_text() {
            let dir = tempfile::tempdir().unwrap();
            let b = gh_board();
            let (gh, _) = mover(
                dir.path(),
                LOOKUP_ON,
                "case \"$*\" in *Field*) echo 'INSUFFICIENT_SCOPES secret-ish' >&2; exit 1;; esac",
            );
            match go(&gh, &b, 0, "status:o2") {
                Err(CoreError::IssuesMove(m)) => {
                    assert!(m.contains("gh auth refresh -s project"));
                    assert!(!m.contains("secret-ish"));
                }
                other => panic!("{other:?}"),
            }
            let (gh, _) = mover(dir.path(), LOOKUP_ON, "echo 'secret-ish' >&2; exit 1");
            let err = go(&gh, &b, 0, "done").unwrap_err();
            assert!(matches!(err, CoreError::IssuesMove(_)));
            assert!(!err.to_string().contains("secret-ish"));
            // A lookup without a project is a refusal, not a panic.
            let (gh, _) = mover(dir.path(), "{}", "");
            assert!(matches!(
                go(&gh, &b, 0, "status:o1"),
                Err(CoreError::IssuesMove(_))
            ));
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
