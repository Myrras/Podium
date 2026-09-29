//! Issues assigned to the user in a project's repo, for the sidebar's Issues
//! section. The repo is auto-detected from the git `origin` remote; GitHub
//! issues come from the user's own `gh` login (Podium stores no token).
//!
//! Self-hosted GitLab (temporary, see [`fetch_gitlab`]) uses a personal access
//! token kept only in the OS keychain: it is never returned, logged or put in
//! an error message.
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

/// Open issues assigned to `@me` in `owner_repo`, via the `gh` binary at `gh`
/// (injectable so tests can point it at a fake script).
pub fn fetch_github(gh: &str, owner_repo: &str) -> CoreResult<Vec<IssueInfo>> {
    let output = Command::new(gh)
        .args(["issue", "list", "--repo", owner_repo])
        .args(["--assignee", "@me", "--state", "open", "--limit", "100"])
        .args(["--json", "number,title,url,body,labels,updatedAt"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
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

// ponytail: temporary, remove once Connect is on GitHub — the GitLab parts
// of this module (this block, `Route::GitLab`, the dispatch arm in `list`,
// `IssueSettings`, the two errors, the Settings tab) go in one sweep.

const KEYCHAIN_SERVICE: &str = "com.podium.app.gitlab";

/// Reduce user input (`https://Git.Example.test/`) to a bare hostname; `None`
/// when nothing usable is left.
pub fn normalize_host(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    let host = s.split('/').next().unwrap_or("").trim();
    (!host.is_empty() && !host.contains(char::is_whitespace)).then(|| host.to_ascii_lowercase())
}

fn keychain_entry(host: &str) -> CoreResult<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, host)
        .map_err(|_| CoreError::IssuesFetch("could not open the keychain".to_string()))
}

/// Store the token for `host` in the OS keychain; a blank token deletes it.
pub fn gitlab_token_set(host: &str, token: &str) -> CoreResult<()> {
    let entry = keychain_entry(host)?;
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

fn gitlab_token_get(host: &str) -> Option<String> {
    keychain_entry(host).ok()?.get_password().ok()
}

/// Whether a token is stored for `host` (the token itself never leaves here).
pub fn gitlab_token_present(host: &str) -> bool {
    gitlab_token_get(host).is_some()
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

/// Where a remote's issues come from.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    GitHub,
    GitLab,
    Unsupported,
}

fn route(remote_host: &str, gitlab_host: Option<&str>) -> Route {
    if remote_host == "github.com" {
        Route::GitHub
    } else if gitlab_host == Some(remote_host) {
        // ponytail: temporary, remove once Connect is on GitHub
        Route::GitLab
    } else {
        Route::Unsupported
    }
}

/// Detect the repo behind `root` and list the user's open issues there.
/// `gitlab_host` is the configured self-hosted GitLab, if any.
pub fn list(root: &Path, gitlab_host: Option<&str>) -> CoreResult<Vec<IssueInfo>> {
    let (host, path) = detect(root)?;
    match route(&host, gitlab_host) {
        Route::GitHub => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            fetch_github(gh, &path)
        }
        Route::GitLab => {
            let token = gitlab_token_get(&host).ok_or(CoreError::IssuesGitlabNotConfigured)?;
            fetch_gitlab(&format!("https://{host}"), &path, &token)
        }
        Route::Unsupported => Err(CoreError::IssuesUnsupportedHost),
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
    fn normalizes_host() {
        for (input, want) in [
            ("git.example.test", Some("git.example.test")),
            ("https://Git.Example.test/", Some("git.example.test")),
            (
                "  http://git.example.test/group/proj ",
                Some("git.example.test"),
            ),
            ("git.example.test:8443", Some("git.example.test:8443")),
            ("", None),
            ("   ", None),
            ("https://", None),
            ("two words", None),
        ] {
            assert_eq!(normalize_host(input).as_deref(), want, "{input:?}");
        }
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

    #[test]
    fn dispatches_by_host() {
        assert_eq!(route("github.com", None), Route::GitHub);
        assert_eq!(route("github.com", Some("git.example.test")), Route::GitHub);
        assert_eq!(
            route("git.example.test", Some("git.example.test")),
            Route::GitLab
        );
        assert_eq!(route("git.example.test", None), Route::Unsupported);
        assert_eq!(
            route("other.test", Some("git.example.test")),
            Route::Unsupported
        );
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
            assert_eq!(fetch_github(&ok, "acme/widgets").unwrap()[0].number, 3);

            let auth = script(dir.path(), "echo 'run: gh auth login' >&2; exit 4");
            assert!(matches!(
                fetch_github(&auth, "acme/widgets"),
                Err(CoreError::IssuesGhUnavailable)
            ));

            let other = script(dir.path(), "echo 'secret-ish stderr' >&2; exit 1");
            let err = fetch_github(&other, "acme/widgets").unwrap_err();
            assert!(matches!(err, CoreError::IssuesFetch(_)));
            assert!(!err.to_string().contains("secret-ish"));

            assert!(matches!(
                fetch_github("/nonexistent/gh", "acme/widgets"),
                Err(CoreError::IssuesGhUnavailable)
            ));
        }
    }
}
