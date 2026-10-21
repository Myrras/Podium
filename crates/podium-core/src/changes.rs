//! The changes panel's data: which files differ in a checkout (the project
//! root or one of its Podium worktrees) and what changed in each.
//!
//! Two bases: [`ChangesBase::Uncommitted`] compares the working tree with
//! `HEAD` (staged, unstaged and untracked together — what a commit would
//! pick up), [`ChangesBase::Branch`] compares it with the merge-base of
//! `HEAD` and the repository's default branch, so a feature branch shows
//! everything it changes, committed or not. Both read the working tree, so
//! an edit an agent is making right now shows up on the next refresh.
//!
//! Git runs through [`crate::worktree::git`] (the project's backend, no
//! shell, stderr never in an error). An untracked file has no diff in git's
//! eyes, so its content is read from disk and shown as all-added. Every list
//! and diff is capped so a vendored directory or a generated file cannot
//! flood the webview.

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::platform::WslBackend;
use crate::worktree::git;

/// Git's well-known empty tree: the base for a repository without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Most files one listing returns; the rest is reported as `truncated`.
const MAX_FILES: usize = 1000;

/// Most bytes of one file's diff returned to the UI.
const MAX_DIFF_BYTES: usize = 512 * 1024;

/// Untracked files larger than this get no line count and no inline content.
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;

/// Default-branch refs tried, in order, when `origin/HEAD` is not set.
const DEFAULT_BRANCH_CANDIDATES: &[&str] = &["origin/main", "origin/master", "main", "master"];

/// What a checkout is compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangesBase {
    /// The working tree against `HEAD`, untracked files included.
    Uncommitted,
    /// The working tree against the merge-base with the default branch.
    Branch,
}

/// How one file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
}

/// One changed file in a [`ChangeSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// Repository-relative path, `/`-separated (the new path for a rename).
    pub path: String,
    /// The path before a rename or copy.
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// Lines added/removed; `None` for a binary (or oversized untracked) file.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub binary: bool,
}

/// The changed files of one checkout against one base.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    /// The branch checked out, `None` when detached.
    pub branch: Option<String>,
    pub base: ChangesBase,
    /// The ref the branch is compared with (`origin/main`, …); only for
    /// [`ChangesBase::Branch`], and `None` when no default branch was found
    /// (the listing then falls back to the uncommitted changes).
    pub base_ref: Option<String>,
    /// Short hash of the commit compared with; `None` before the first commit.
    pub base_commit: Option<String>,
    pub files: Vec<ChangedFile>,
    /// More than [`MAX_FILES`] files changed; only the first ones are listed.
    pub truncated: bool,
}

/// One file's unified diff.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// Git's unified diff (for an untracked file, one synthesized all-added
    /// hunk); empty for a binary file or a pure rename.
    pub diff: String,
    pub binary: bool,
    /// The diff was cut at [`MAX_DIFF_BYTES`].
    pub truncated: bool,
}

/// The commit `base` compares with in `dir`, plus the ref it came from.
/// Falls back to `HEAD` (or the empty tree) when a branch base cannot be
/// resolved, so the panel still shows something useful.
fn resolve_base(
    wsl: Option<&WslBackend>,
    dir: &Path,
    base: ChangesBase,
) -> (String, Option<String>) {
    let head = git(wsl, dir, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let Some(head) = head else {
        return (EMPTY_TREE.to_string(), None);
    };
    if base == ChangesBase::Uncommitted {
        return (head, None);
    }
    let Some(default_ref) = default_branch(wsl, dir) else {
        return (head, None);
    };
    match git(wsl, dir, &["merge-base", "HEAD", &default_ref]) {
        Some(mb) if !mb.trim().is_empty() => (mb.trim().to_string(), Some(default_ref)),
        _ => (head, None),
    }
}

/// The repository's default branch as a ref name: `origin/HEAD`'s target
/// when set, else the first of [`DEFAULT_BRANCH_CANDIDATES`] that exists.
fn default_branch(wsl: Option<&WslBackend>, dir: &Path) -> Option<String> {
    if let Some(sym) = git(
        wsl,
        dir,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let sym = sym.trim();
        if !sym.is_empty() {
            return Some(sym.to_string());
        }
    }
    DEFAULT_BRANCH_CANDIDATES
        .iter()
        .find(|r| {
            git(
                wsl,
                dir,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{r}^{{commit}}"),
                ],
            )
            .is_some()
        })
        .map(|r| (*r).to_string())
}

/// List the files changed in `dir` against `base`.
pub fn list(wsl: Option<&WslBackend>, dir: &Path, base: ChangesBase) -> CoreResult<ChangeSet> {
    if git(wsl, dir, &["rev-parse", "--git-dir"]).is_none() {
        return Err(CoreError::NotAGitRepo);
    }
    let (commit, base_ref) = resolve_base(wsl, dir, base);
    let name_status = git(
        wsl,
        dir,
        &[
            "diff",
            "--no-ext-diff",
            "-M",
            "--name-status",
            "-z",
            &commit,
        ],
    )
    .ok_or_else(|| CoreError::Git("git diff failed".to_string()))?;
    let numstat = git(
        wsl,
        dir,
        &["diff", "--no-ext-diff", "-M", "--numstat", "-z", &commit],
    )
    .ok_or_else(|| CoreError::Git("git diff failed".to_string()))?;
    let untracked = git(
        wsl,
        dir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .ok_or_else(|| CoreError::Git("git ls-files failed".to_string()))?;

    let mut files = parse_name_status(&name_status);
    let counts = parse_numstat(&numstat);
    for f in &mut files {
        if let Some(&(add, del)) = counts.get(&f.path) {
            match (add, del) {
                (Some(a), Some(d)) => {
                    f.additions = Some(a);
                    f.deletions = Some(d);
                }
                _ => f.binary = true,
            }
        }
    }
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        let (lines, binary) = untracked_stats(&dir.join(path));
        files.push(ChangedFile {
            path: path.to_string(),
            old_path: None,
            status: ChangeStatus::Untracked,
            additions: lines,
            deletions: lines.map(|_| 0),
            binary,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);

    let base_commit = (commit != EMPTY_TREE).then(|| commit.chars().take(8).collect());
    Ok(ChangeSet {
        branch: crate::worktree::current_branch(wsl, dir),
        base,
        base_ref,
        base_commit,
        files,
        truncated,
    })
}

/// The unified diff of one file in `dir` against `base`. `old_path` is the
/// pre-rename path, so a renamed file diffs against its old content.
pub fn file_diff(
    wsl: Option<&WslBackend>,
    dir: &Path,
    base: ChangesBase,
    path: &str,
    old_path: Option<&str>,
) -> CoreResult<FileDiff> {
    validate_rel_path(path)?;
    if let Some(old) = old_path {
        validate_rel_path(old)?;
    }
    if git(wsl, dir, &["rev-parse", "--git-dir"]).is_none() {
        return Err(CoreError::NotAGitRepo);
    }
    let (commit, _) = resolve_base(wsl, dir, base);
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        commit.as_str(),
        "--",
        path,
    ];
    if let Some(old) = old_path {
        args.push(old);
    }
    let diff = git(wsl, dir, &args).ok_or_else(|| CoreError::Git("git diff failed".to_string()))?;
    if diff.trim().is_empty() {
        // Git knows nothing about an untracked file: show it as all-added.
        let full = dir.join(path);
        if full.is_file() {
            return Ok(untracked_diff(&full, path));
        }
    }
    let binary = diff
        .lines()
        .any(|l| l.starts_with("Binary files ") && l.ends_with(" differ"));
    let (diff, truncated) = cap(diff);
    Ok(FileDiff {
        path: path.to_string(),
        diff: if binary { String::new() } else { diff },
        binary,
        truncated,
    })
}

/// A path from the UI must stay inside the checkout: relative, no `..`.
fn validate_rel_path(path: &str) -> CoreResult<()> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if ok {
        Ok(())
    } else {
        Err(CoreError::InvalidInput(
            "path must be relative to the checkout".to_string(),
        ))
    }
}

/// Cut `diff` to [`MAX_DIFF_BYTES`] on a line boundary.
fn cap(diff: String) -> (String, bool) {
    if diff.len() <= MAX_DIFF_BYTES {
        return (diff, false);
    }
    let mut end = MAX_DIFF_BYTES;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    let cut = diff[..end].rfind('\n').map_or(end, |i| i + 1);
    (diff[..cut].to_string(), true)
}

/// Line count and binary flag of an untracked file; no count when it is
/// binary, too large or unreadable.
fn untracked_stats(full: &Path) -> (Option<u32>, bool) {
    let Ok(meta) = std::fs::metadata(full) else {
        return (None, false);
    };
    if meta.len() > MAX_UNTRACKED_BYTES {
        return (None, false);
    }
    match std::fs::read(full) {
        Ok(bytes) if is_binary(&bytes) => (None, true),
        Ok(bytes) => (Some(count_lines(&bytes)), false),
        Err(_) => (None, false),
    }
}

/// Git's own heuristic: a NUL byte in the first 8000 bytes means binary.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

fn count_lines(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let last_open = usize::from(!bytes.ends_with(b"\n"));
    u32::try_from(newlines + last_open).unwrap_or(u32::MAX)
}

/// An untracked file as one all-added hunk, in git's unified format.
fn untracked_diff(full: &Path, path: &str) -> FileDiff {
    let too_big = std::fs::metadata(full).map_or(true, |m| m.len() > MAX_UNTRACKED_BYTES);
    let bytes = if too_big {
        None
    } else {
        std::fs::read(full).ok()
    };
    let Some(bytes) = bytes else {
        return FileDiff {
            path: path.to_string(),
            diff: String::new(),
            binary: false,
            truncated: true,
        };
    };
    if is_binary(&bytes) {
        return FileDiff {
            path: path.to_string(),
            diff: String::new(),
            binary: true,
            truncated: false,
        };
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in &lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    let (diff, truncated) = cap(diff);
    FileDiff {
        path: path.to_string(),
        diff,
        binary: false,
        truncated,
    }
}

/// Parse `git diff --name-status -z`: `X\0path\0`, or `Rnnn\0old\0new\0`
/// for renames and copies.
fn parse_name_status(out: &str) -> Vec<ChangedFile> {
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    let mut files = Vec::new();
    while let Some(code) = parts.next() {
        let kind = code.chars().next().unwrap_or('M');
        let (status, two_paths) = match kind {
            'A' => (ChangeStatus::Added, false),
            'D' => (ChangeStatus::Deleted, false),
            'R' => (ChangeStatus::Renamed, true),
            'C' => (ChangeStatus::Copied, true),
            'T' => (ChangeStatus::TypeChanged, false),
            _ => (ChangeStatus::Modified, false),
        };
        let (old_path, path) = if two_paths {
            let old = parts.next().map(str::to_string);
            (old, parts.next())
        } else {
            (None, parts.next())
        };
        let Some(path) = path else { break };
        files.push(ChangedFile {
            path: path.to_string(),
            old_path,
            status,
            additions: None,
            deletions: None,
            binary: false,
        });
    }
    files
}

/// Parse `git diff --numstat -z` into added/removed counts keyed by the
/// (new) path; `None` counts mean binary (`-\t-`). A rename is
/// `a\td\t\0old\0new\0`.
fn parse_numstat(out: &str) -> std::collections::HashMap<String, (Option<u32>, Option<u32>)> {
    let mut map = std::collections::HashMap::new();
    let mut parts = out.split('\0');
    while let Some(rec) = parts.next() {
        if rec.is_empty() {
            continue;
        }
        let mut cols = rec.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            // Rename: the old and new paths follow as their own records.
            let _old = parts.next();
            match parts.next() {
                Some(new) => new,
                None => break,
            }
        } else {
            path
        };
        map.insert(path.to_string(), (add.parse().ok(), del.parse().ok()));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status_parses_plain_and_renamed_entries() {
        let out = "M\0src/a.rs\0R087\0old/b.rs\0new/b.rs\0D\0gone.txt\0A\0fresh.md\0";
        let files = parse_name_status(out);
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].status, ChangeStatus::Modified);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[1].status, ChangeStatus::Renamed);
        assert_eq!(files[1].old_path.as_deref(), Some("old/b.rs"));
        assert_eq!(files[1].path, "new/b.rs");
        assert_eq!(files[2].status, ChangeStatus::Deleted);
        assert_eq!(files[3].status, ChangeStatus::Added);
    }

    #[test]
    fn numstat_parses_counts_binary_and_renames() {
        let out = "3\t1\tsrc/a.rs\0-\t-\timg.png\x000\t0\t\0old/b.rs\0new/b.rs\0";
        let map = parse_numstat(out);
        assert_eq!(map["src/a.rs"], (Some(3), Some(1)));
        assert_eq!(map["img.png"], (None, None));
        assert_eq!(map["new/b.rs"], (Some(0), Some(0)));
    }

    #[test]
    fn rel_paths_cannot_escape_the_checkout() {
        assert!(validate_rel_path("src/a.rs").is_ok());
        assert!(validate_rel_path("../secret").is_err());
        assert!(validate_rel_path("a/../../b").is_err());
        assert!(validate_rel_path("").is_err());
        #[cfg(unix)]
        assert!(validate_rel_path("/etc/passwd").is_err());
        #[cfg(windows)]
        assert!(validate_rel_path("C:\\Windows").is_err());
    }

    #[test]
    fn cap_cuts_on_a_line_boundary() {
        let line = "+".repeat(99) + "\n";
        let big = line.repeat(MAX_DIFF_BYTES / 100 + 10);
        let (cut, truncated) = cap(big);
        assert!(truncated);
        assert!(cut.len() <= MAX_DIFF_BYTES);
        assert!(cut.ends_with('\n'));
        let (small, truncated) = cap("x\n".to_string());
        assert!(!truncated);
        assert_eq!(small, "x\n");
    }

    #[test]
    fn line_counts_handle_a_missing_final_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }

    // --- real git ---

    fn run(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }

    /// A throwaway repo on `main` with one commit, identity pinned.
    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        run(dir.path(), &["init", "-b", "main"]);
        run(dir.path(), &["config", "user.name", "Test"]);
        run(dir.path(), &["config", "user.email", "test@example.com"]);
        run(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("README.md"), "sample\n").unwrap();
        std::fs::write(dir.path().join("keep.txt"), "one\ntwo\n").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "initial"]);
        dir
    }

    fn by_path<'a>(set: &'a ChangeSet, path: &str) -> &'a ChangedFile {
        set.files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} not listed"))
    }

    #[test]
    fn uncommitted_lists_modified_staged_deleted_and_untracked() {
        let repo = init_repo();
        let root = repo.path();
        std::fs::write(root.join("README.md"), "sample\nmore\n").unwrap();
        std::fs::write(root.join("staged.txt"), "x\n").unwrap();
        run(root, &["add", "staged.txt"]);
        std::fs::remove_file(root.join("keep.txt")).unwrap();
        std::fs::write(root.join("new.txt"), "a\nb\nc\n").unwrap();

        let set = list(None, root, ChangesBase::Uncommitted).unwrap();
        assert_eq!(set.branch.as_deref(), Some("main"));
        assert!(set.base_ref.is_none());
        let readme = by_path(&set, "README.md");
        assert_eq!(readme.status, ChangeStatus::Modified);
        assert_eq!((readme.additions, readme.deletions), (Some(1), Some(0)));
        assert_eq!(by_path(&set, "staged.txt").status, ChangeStatus::Added);
        assert_eq!(by_path(&set, "keep.txt").status, ChangeStatus::Deleted);
        let untracked = by_path(&set, "new.txt");
        assert_eq!(untracked.status, ChangeStatus::Untracked);
        assert_eq!(untracked.additions, Some(3));
    }

    #[test]
    fn branch_base_includes_commits_since_the_default_branch() {
        let repo = init_repo();
        let root = repo.path();
        run(root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("feature.txt"), "f\n").unwrap();
        run(root, &["add", "."]);
        run(root, &["commit", "-m", "feature work"]);
        std::fs::write(root.join("README.md"), "sample\nedit\n").unwrap();

        let uncommitted = list(None, root, ChangesBase::Uncommitted).unwrap();
        assert!(uncommitted.files.iter().all(|f| f.path != "feature.txt"));

        let branch = list(None, root, ChangesBase::Branch).unwrap();
        assert_eq!(branch.base_ref.as_deref(), Some("main"));
        assert_eq!(by_path(&branch, "feature.txt").status, ChangeStatus::Added);
        assert_eq!(by_path(&branch, "README.md").status, ChangeStatus::Modified);
    }

    #[test]
    fn file_diff_shows_tracked_and_untracked_content() {
        let repo = init_repo();
        let root = repo.path();
        std::fs::write(root.join("README.md"), "sample\nmore\n").unwrap();
        std::fs::write(root.join("new.txt"), "hello\nworld\n").unwrap();

        let tracked = file_diff(None, root, ChangesBase::Uncommitted, "README.md", None).unwrap();
        assert!(tracked.diff.contains("@@"));
        assert!(tracked.diff.contains("\n+more"));
        assert!(!tracked.binary);

        let fresh = file_diff(None, root, ChangesBase::Uncommitted, "new.txt", None).unwrap();
        assert!(fresh.diff.contains("@@ -0,0 +1,2 @@"));
        assert!(fresh.diff.contains("\n+hello\n+world\n"));

        assert!(file_diff(None, root, ChangesBase::Uncommitted, "../x", None).is_err());
    }

    #[test]
    fn a_repo_without_commits_lists_everything_as_untracked() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-b", "main"]);
        std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
        let set = list(None, dir.path(), ChangesBase::Branch).unwrap();
        assert!(set.base_commit.is_none());
        assert_eq!(by_path(&set, "a.txt").status, ChangeStatus::Untracked);
    }

    #[test]
    fn non_git_dir_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            list(None, dir.path(), ChangesBase::Uncommitted),
            Err(CoreError::NotAGitRepo)
        ));
    }
}
