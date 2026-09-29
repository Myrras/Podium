//! Issues assigned to the user in a project's repo, for the sidebar's Issues
//! section. The repo is auto-detected from the git `origin` remote; which
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

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// One open issue assigned to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub labels: Vec<String>,
    pub updated_at: String,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhIssue {
    number: u64,
    title: String,
    url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    labels: Vec<GhLabel>,
    #[serde(default)]
    updated_at: String,
}

#[derive(Deserialize)]
struct GhLabel {
    name: String,
}

/// Map `gh issue list --json …` output to [`IssueInfo`]s.
pub fn parse_gh_issues(json: &str) -> CoreResult<Vec<IssueInfo>> {
    let raw: Vec<GhIssue> = serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesFetch("unexpected response from gh".to_string()))?;
    Ok(raw
        .into_iter()
        .map(|i| IssueInfo {
            number: i.number,
            title: i.title,
            url: i.url,
            body: i.body,
            labels: i.labels.into_iter().map(|l| l.name).collect(),
            updated_at: i.updated_at,
        })
        .collect())
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

/// Open issues assigned to `@me` in `owner_repo` on `host`, via the `gh`
/// binary at `gh` (injectable so tests can point it at a fake script).
/// `account` picks a logged-in `gh` account; `None` uses the active one.
pub fn fetch_github(
    gh: &str,
    host: &str,
    owner_repo: &str,
    account: Option<&str>,
) -> CoreResult<Vec<IssueInfo>> {
    let mut cmd = Command::new(gh);
    cmd.args(["issue", "list", "--repo", &format!("{host}/{owner_repo}")])
        .args(["--assignee", "@me", "--state", "open", "--limit", "100"])
        .args(["--json", "number,title,url,body,labels,updatedAt"])
        .stdin(Stdio::null());
    if let Some(account) = account {
        let token = gh_account_token(gh, host, account)?;
        let var = if host == "github.com" {
            "GH_TOKEN"
        } else {
            "GH_ENTERPRISE_TOKEN"
        };
        cmd.env(var, token);
    }
    let output = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CoreError::IssuesGhUnavailable,
        _ => CoreError::IssuesFetch("could not run gh".to_string()),
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        // stderr is only inspected, never forwarded.
        return Err(
            if stderr.contains("auth login") || stderr.contains("not logged") {
                CoreError::IssuesGhUnavailable
            } else {
                CoreError::IssuesFetch("gh could not fetch the issues".to_string())
            },
        );
    }
    parse_gh_issues(&String::from_utf8_lossy(&output.stdout))
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
// `list`, the two errors, the Settings fields) go in one sweep.

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
struct GitlabIssue {
    iid: u64,
    title: String,
    web_url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    updated_at: String,
}

/// Map GitLab's `/issues` JSON to [`IssueInfo`]s.
pub fn parse_gitlab_issues(json: &str) -> CoreResult<Vec<IssueInfo>> {
    let raw: Vec<GitlabIssue> = serde_json::from_str(json)
        .map_err(|_| CoreError::IssuesFetch("unexpected response from GitLab".to_string()))?;
    Ok(raw
        .into_iter()
        .map(|i| IssueInfo {
            number: i.iid,
            title: i.title,
            url: i.web_url,
            body: i.description.unwrap_or_default(),
            labels: i.labels,
            updated_at: i.updated_at,
        })
        .collect())
}

/// Open issues assigned to the token's owner in `project_path`, from the
/// instance at `base` (`https://host`; injectable so tests can use plain HTTP).
pub fn fetch_gitlab(base: &str, project_path: &str, token: &str) -> CoreResult<Vec<IssueInfo>> {
    let url = format!(
        "{base}/api/v4/projects/{}/issues?scope=assigned_to_me&state=opened&per_page=100",
        project_path.replace('/', "%2F")
    );
    let response = reqwest::blocking::Client::new()
        .get(url)
        .header("PRIVATE-TOKEN", token)
        .send()
        .map_err(|_| CoreError::IssuesFetch("could not reach GitLab".to_string()))?;
    match response.status().as_u16() {
        200 => {
            let body = response
                .text()
                .map_err(|_| CoreError::IssuesFetch("could not read GitLab's response".into()))?;
            parse_gitlab_issues(&body)
        }
        401 | 403 => Err(CoreError::IssuesGitlabAuth),
        _ => Err(CoreError::IssuesFetch(
            "GitLab could not fetch the issues".to_string(),
        )),
    }
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

/// Detect the repo behind `root` and list the user's open issues there,
/// using the project's `selected` setup or the first matching one.
pub fn list(
    root: &Path,
    setups: &[IssueSetup],
    selected: Option<&IssueSetup>,
) -> CoreResult<Vec<IssueInfo>> {
    let (host, path) = detect(root)?;
    let setup = pick(&host, setups, selected)?;
    match setup {
        Some(IssueSetup {
            provider: IssueProvider::Gitlab,
            id,
            url,
            ..
        }) => {
            // ponytail: temporary, remove once Connect is on GitHub
            let token = gitlab_token_get(id).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            fetch_gitlab(url, &path, &token)
        }
        _ => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            let account = setup.and_then(|s| s.account.as_deref());
            fetch_github(gh, &host, &path, account)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn maps_gh_json() {
        let json = r#"[
          {"number":7,"title":"Fix widget","url":"https://github.com/acme/widgets/issues/7",
           "body":"It breaks.","labels":[{"name":"bug","color":"d73a4a"},{"name":"p1"}],
           "updatedAt":"2026-09-01T10:00:00Z"},
          {"number":9,"title":"No body","url":"https://github.com/acme/widgets/issues/9",
           "labels":[],"updatedAt":"2026-09-02T10:00:00Z"}
        ]"#;
        let issues = parse_gh_issues(json).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 7);
        assert_eq!(issues[0].labels, vec!["bug", "p1"]);
        assert_eq!(issues[0].updated_at, "2026-09-01T10:00:00Z");
        assert_eq!(issues[1].body, "");
        assert!(matches!(
            parse_gh_issues("not json"),
            Err(CoreError::IssuesFetch(_))
        ));
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
    fn maps_gitlab_json() {
        let json = r#"[
          {"id":501,"iid":12,"title":"Fix invoice","web_url":"https://git.example.test/grp/proj/-/issues/12",
           "description":"Broken.","labels":["bug","p1"],"updated_at":"2026-09-01T10:00:00Z"},
          {"id":502,"iid":13,"title":"No body","web_url":"https://git.example.test/grp/proj/-/issues/13",
           "description":null,"labels":[],"updated_at":"2026-09-02T10:00:00Z"}
        ]"#;
        let issues = parse_gitlab_issues(json).unwrap();
        assert_eq!(issues[0].number, 12);
        assert_eq!(
            issues[0].url,
            "https://git.example.test/grp/proj/-/issues/12"
        );
        assert_eq!(issues[0].body, "Broken.");
        assert_eq!(issues[0].labels, vec!["bug", "p1"]);
        assert_eq!(issues[1].body, "");
        assert!(matches!(
            parse_gitlab_issues("{}"),
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

    /// One-shot local HTTP server answering with `status` + `body`; returns
    /// the base URL and the request text it saw.
    fn serve_once(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (base, handle)
    }

    #[test]
    fn gitlab_fetch_sends_token_and_maps_status() {
        let (base, seen) = serve_once(
            "200 OK",
            r#"[{"iid":4,"title":"t","web_url":"u","labels":[],"updated_at":"x"}]"#,
        );
        let issues = fetch_gitlab(&base, "grp/sub/proj", "fake-token").unwrap();
        assert_eq!(issues[0].number, 4);
        let req = seen.join().unwrap();
        assert!(req.starts_with(
            "GET /api/v4/projects/grp%2Fsub%2Fproj/issues?scope=assigned_to_me&state=opened"
        ));
        assert!(req.to_lowercase().contains("private-token: fake-token"));

        for code in ["401 Unauthorized", "403 Forbidden"] {
            let (base, _h) = serve_once(code, "{}");
            assert!(matches!(
                fetch_gitlab(&base, "a/b", "wrong"),
                Err(CoreError::IssuesGitlabAuth)
            ));
        }
        let (base, _h) = serve_once("500 Internal Server Error", "secret-ish body");
        let err = fetch_gitlab(&base, "a/b", "t").unwrap_err();
        assert!(matches!(err, CoreError::IssuesFetch(_)));
        assert!(!err.to_string().contains("secret-ish"));
    }

    #[test]
    fn serializes_camel_case() {
        let v = serde_json::to_value(
            &parse_gh_issues(r#"[{"number":1,"title":"t","url":"u","updatedAt":"x"}]"#).unwrap()[0],
        )
        .unwrap();
        assert_eq!(v["updatedAt"], "x");
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

        #[test]
        fn success_and_error_classification() {
            let dir = tempfile::tempdir().unwrap();
            let ok = script(dir.path(), r#"echo '[{"number":3,"title":"t","url":"u"}]'"#);
            assert_eq!(
                fetch_github(&ok, "github.com", "acme/widgets", None).unwrap()[0].number,
                3
            );

            let auth = script(dir.path(), "echo 'run: gh auth login' >&2; exit 4");
            assert!(matches!(
                fetch_github(&auth, "github.com", "acme/widgets", None),
                Err(CoreError::IssuesGhUnavailable)
            ));

            let other = script(dir.path(), "echo 'secret-ish stderr' >&2; exit 1");
            let err = fetch_github(&other, "github.com", "acme/widgets", None).unwrap_err();
            assert!(matches!(err, CoreError::IssuesFetch(_)));
            assert!(!err.to_string().contains("secret-ish"));

            assert!(matches!(
                fetch_github("/nonexistent/gh", "github.com", "acme/widgets", None),
                Err(CoreError::IssuesGhUnavailable)
            ));
        }

        #[test]
        fn account_token_is_passed_to_gh_only_via_env() {
            let dir = tempfile::tempdir().unwrap();
            let gh = script(
                dir.path(),
                r#"if [ "$1" = auth ]; then [ "$6" = work ] && echo fake-tok && exit 0; exit 1; fi
echo "[{\"number\":1,\"title\":\"$GH_TOKEN $4\",\"url\":\"u\"}]""#,
            );
            let issues = fetch_github(&gh, "github.com", "acme/widgets", Some("work")).unwrap();
            assert_eq!(issues[0].title, "fake-tok github.com/acme/widgets");
            let unset = fetch_github(&gh, "github.com", "acme/widgets", None).unwrap();
            assert_eq!(unset[0].title, " github.com/acme/widgets");
            assert!(matches!(
                fetch_github(&gh, "github.com", "acme/widgets", Some("nobody")),
                Err(CoreError::IssuesGhUnavailable)
            ));
        }
    }
}
