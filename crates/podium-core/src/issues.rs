//! Issues assigned to the user in a project's repo, for the sidebar's Issues
//! section. The repo is auto-detected from the git `origin` remote; GitHub
//! issues come from the user's own `gh` login (Podium stores no token).
//!
//! Everything here shells out, so call it from a blocking-friendly context.
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

/// Detect the repo behind `root` and list the user's open issues there.
pub fn list(root: &Path) -> CoreResult<Vec<IssueInfo>> {
    let (host, path) = detect(root)?;
    match host.as_str() {
        "github.com" => {
            let gh = crate::platform::gh_binary().ok_or(CoreError::IssuesGhUnavailable)?;
            fetch_github(gh, &path)
        }
        _ => Err(CoreError::IssuesUnsupportedHost),
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
