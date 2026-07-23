//! Podium-managed git worktrees: isolated checkouts under
//! `<project_root>/.podium/worktrees/<name>`, each on its own
//! `podium/<name>` branch, so an agent's changes never touch the user's
//! working tree.
//!
//! Git is driven through the login shell (`platform::run_shell_stdout`) —
//! the same idiom as adapter probing, so PATH edits from shell profiles are
//! honoured. stderr is always discarded so git output can never leak into
//! `CoreError` messages. There is no persistence and no events:
//! `git worktree list --porcelain` is the source of truth.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CoreError, CoreResult};

/// Read-only snapshot of one Podium-managed worktree.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    /// The slugified directory/branch name (unique within the project).
    pub name: String,
    /// Absolute path of the checkout, under `.podium/worktrees/`.
    pub path: PathBuf,
    /// The branch checked out in it (normally `podium/<name>`).
    pub branch: String,
    /// Whether a Podium-managed process is currently running in it. Always
    /// `false` from [`list`]; the orchestrator fills it in.
    pub in_use: bool,
}

/// Longest slug we generate — keeps paths and branch names readable.
const MAX_SLUG_LEN: usize = 40;

/// Run `git -C <root> <args…>` through the login shell, discarding stderr;
/// `Some(stdout)` on success, `None` on any failure. Output never reaches an
/// error message.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = format!(
        "git -C {}",
        crate::platform::quote_arg(&root.to_string_lossy()).ok()?
    );
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&crate::platform::quote_arg(arg).ok()?);
    }
    crate::platform::run_shell_stdout(&cmd)
}

/// The current branch checked out at `cwd`, or `None` when it is not a git
/// repository or is in a detached HEAD (`rev-parse` yields the literal
/// `HEAD`). Shells out to git — call from a blocking-friendly context.
pub fn current_branch(cwd: &Path) -> Option<String> {
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }
    Some(branch)
}

/// `root` must be (inside) a git repository; also covers "git not
/// installed", which probes the same way.
fn ensure_git_repo(root: &Path) -> CoreResult<()> {
    if git(root, &["rev-parse", "--git-dir"]).is_none() {
        return Err(CoreError::NotAGitRepo);
    }
    Ok(())
}

/// Reduce a free-form name to a directory/branch-safe slug: lowercase,
/// `[a-z0-9]` runs joined by single `-`, trimmed, capped at
/// [`MAX_SLUG_LEN`]. An empty result is invalid input.
fn slugify(name: &str) -> CoreResult<String> {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(MAX_SLUG_LEN).collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        return Err(CoreError::InvalidInput(
            "worktree name must contain at least one letter or digit".to_string(),
        ));
    }
    Ok(slug)
}

/// The directory Podium worktrees live under.
fn worktrees_dir(root: &Path) -> PathBuf {
    root.join(".podium").join("worktrees")
}

/// Make git ignore `.podium/` via `.git/info/exclude` (never `.gitignore` —
/// that file belongs to the user). Idempotent: appends only when no
/// `.podium` line is present yet.
fn ensure_excluded(root: &Path) -> CoreResult<()> {
    let common = git(root, &["rev-parse", "--git-common-dir"])
        .ok_or(CoreError::NotAGitRepo)?
        .trim()
        .to_string();
    let common = if Path::new(&common).is_absolute() {
        PathBuf::from(common)
    } else {
        root.join(common)
    };
    let info_dir = common.join("info");
    let exclude = info_dir.join("exclude");
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    let already = existing
        .lines()
        .map(str::trim)
        .any(|l| l == ".podium/" || l == ".podium");
    if already {
        return Ok(());
    }
    std::fs::create_dir_all(&info_dir)?;
    let mut contents = existing;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str("# Podium-managed worktrees\n.podium/\n");
    std::fs::write(&exclude, contents)?;
    Ok(())
}

/// Whether the checkout at `path` has uncommitted changes (untracked files
/// count as dirty).
fn is_dirty(path: &Path) -> bool {
    git(path, &["status", "--porcelain"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

/// Create a worktree named after `name` (slugified, de-duplicated with
/// `-2`, `-3`, … when the directory or a leftover `podium/<name>` branch
/// already exists) at `.podium/worktrees/<name>` on a fresh
/// `podium/<name>` branch from HEAD.
pub fn create(root: &Path, name: &str) -> CoreResult<WorktreeInfo> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    ensure_git_repo(&root)?;
    let slug = slugify(name)?;
    let dir = worktrees_dir(&root);
    let mut candidate = slug.clone();
    let mut n = 1usize;
    loop {
        let dir_taken = dir.join(&candidate).exists();
        let branch_taken = git(
            &root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/podium/{candidate}"),
            ],
        )
        .is_some();
        if !dir_taken && !branch_taken {
            break;
        }
        n += 1;
        candidate = format!("{slug}-{n}");
    }
    ensure_excluded(&root)?;
    let path = dir.join(&candidate);
    let branch = format!("podium/{candidate}");
    std::fs::create_dir_all(&dir)?;
    if git(
        &root,
        &["worktree", "add", &path.to_string_lossy(), "-b", &branch],
    )
    .is_none()
    {
        return Err(CoreError::Git(
            "git worktree add failed (does the repository have at least one commit?)".to_string(),
        ));
    }
    Ok(WorktreeInfo {
        name: candidate,
        path,
        branch,
        in_use: false,
    })
}

/// List the Podium-managed worktrees (paths under `.podium/worktrees/`),
/// parsed out of `git worktree list --porcelain`. `in_use` is always
/// `false` here — the orchestrator fills it in from its process table.
pub fn list(root: &Path) -> CoreResult<Vec<WorktreeInfo>> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    ensure_git_repo(&root)?;
    let out = git(&root, &["worktree", "list", "--porcelain"])
        .ok_or_else(|| CoreError::Git("git worktree list failed".to_string()))?;
    let base = worktrees_dir(&root);
    let mut infos = Vec::new();
    // Porcelain output is blank-line-separated blocks, each starting with
    // `worktree <path>` and carrying `branch refs/heads/<b>` or `detached`.
    for block in out.split("\n\n") {
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if line == "detached" {
                branch = Some("(detached)".to_string());
            }
        }
        let Some(path) = path else { continue };
        if !path.starts_with(&base) {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        infos.push(WorktreeInfo {
            name,
            path,
            branch: branch.unwrap_or_else(|| "(detached)".to_string()),
            in_use: false,
        });
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

/// Remove the worktree named `name`. A manually deleted directory is pruned
/// and counts as removed; uncommitted changes are refused unless `force`.
/// The `podium/<name>` branch is intentionally kept — commits are never
/// thrown away here.
pub fn remove(root: &Path, name: &str, force: bool) -> CoreResult<()> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let entry = list(&root)?
        .into_iter()
        .find(|w| w.name == name)
        .ok_or(CoreError::WorktreeNotFound)?;
    if !entry.path.exists() {
        // The checkout is already gone; drop git's stale bookkeeping.
        let _ = git(&root, &["worktree", "prune"]);
        return Ok(());
    }
    if !force && is_dirty(&entry.path) {
        return Err(CoreError::WorktreeDirty);
    }
    if git(
        &root,
        &[
            "worktree",
            "remove",
            "--force",
            &entry.path.to_string_lossy(),
        ],
    )
    .is_none()
    {
        return Err(CoreError::Git("git worktree remove failed".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway git repo with one commit, identity pinned so commits
    /// work on machines without global git config.
    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        run(dir.path(), &["init", "-b", "main"]);
        run(dir.path(), &["config", "user.name", "Test"]);
        run(dir.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(dir.path().join("README.md"), "sample\n").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "initial"]);
        dir
    }

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

    fn git_stdout(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn slugify_normalizes_names() {
        assert_eq!(slugify("Fix Login Bug").unwrap(), "fix-login-bug");
        assert_eq!(slugify("  weird__Name!! ").unwrap(), "weird-name");
        assert_eq!(slugify("émigré café").unwrap(), "migr-caf");
        let long = "x".repeat(80);
        assert_eq!(slugify(&long).unwrap().len(), MAX_SLUG_LEN);
        assert!(slugify("!!!").is_err());
        assert!(slugify("").is_err());
    }

    #[test]
    fn create_makes_worktree_branch_and_exclude() {
        let repo = init_repo();
        let wt = create(repo.path(), "Fix Login").expect("create");
        assert_eq!(wt.name, "fix-login");
        assert_eq!(wt.branch, "podium/fix-login");
        assert!(wt.path.is_dir());
        assert!(wt.path.ends_with(".podium/worktrees/fix-login"));
        // Branch exists.
        let branches = git_stdout(repo.path(), &["branch", "--list", "podium/fix-login"]);
        assert!(branches.contains("podium/fix-login"));
        // `.podium/` is excluded via info/exclude, and `.gitignore` untouched.
        let root = std::fs::canonicalize(repo.path()).unwrap();
        let exclude = std::fs::read_to_string(root.join(".git/info/exclude")).unwrap();
        assert!(exclude.lines().any(|l| l.trim() == ".podium/"));
        assert!(!root.join(".gitignore").exists());
        let status = git_stdout(repo.path(), &["status", "--porcelain"]);
        assert!(status.trim().is_empty(), "worktree dir must be ignored");
    }

    #[test]
    fn exclude_append_is_idempotent() {
        let repo = init_repo();
        create(repo.path(), "one").unwrap();
        create(repo.path(), "two").unwrap();
        let root = std::fs::canonicalize(repo.path()).unwrap();
        let exclude = std::fs::read_to_string(root.join(".git/info/exclude")).unwrap();
        let hits = exclude.lines().filter(|l| l.trim() == ".podium/").count();
        assert_eq!(hits, 1);
    }

    #[test]
    fn create_dedupes_against_dir_and_leftover_branch() {
        let repo = init_repo();
        let first = create(repo.path(), "task").unwrap();
        assert_eq!(first.name, "task");
        let second = create(repo.path(), "task").unwrap();
        assert_eq!(second.name, "task-2");
        // Remove task-2's checkout; its branch stays → next create skips it.
        remove(repo.path(), "task-2", false).unwrap();
        let third = create(repo.path(), "task").unwrap();
        assert_eq!(third.name, "task-3");
    }

    #[test]
    fn list_only_reports_podium_worktrees() {
        let repo = init_repo();
        create(repo.path(), "mine").unwrap();
        // A non-Podium worktree elsewhere must not show up.
        let other = tempfile::tempdir().unwrap();
        run(
            repo.path(),
            &[
                "worktree",
                "add",
                other.path().join("elsewhere").to_str().unwrap(),
                "-b",
                "not-podium",
            ],
        );
        let listed = list(repo.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "mine");
        assert_eq!(listed[0].branch, "podium/mine");
        assert!(!listed[0].in_use);
    }

    #[test]
    fn remove_refuses_dirty_unless_forced() {
        let repo = init_repo();
        let wt = create(repo.path(), "dirty").unwrap();
        // An untracked file counts as dirty.
        std::fs::write(wt.path.join("scratch.txt"), "wip\n").unwrap();
        assert!(matches!(
            remove(repo.path(), "dirty", false),
            Err(CoreError::WorktreeDirty)
        ));
        remove(repo.path(), "dirty", true).unwrap();
        assert!(!wt.path.exists());
        // The branch survives removal.
        let branches = git_stdout(repo.path(), &["branch", "--list", "podium/dirty"]);
        assert!(branches.contains("podium/dirty"));
    }

    #[test]
    fn remove_clean_worktree_and_missing_name() {
        let repo = init_repo();
        create(repo.path(), "clean").unwrap();
        remove(repo.path(), "clean", false).unwrap();
        assert!(list(repo.path()).unwrap().is_empty());
        assert!(matches!(
            remove(repo.path(), "clean", false),
            Err(CoreError::WorktreeNotFound)
        ));
    }

    #[test]
    fn remove_prunes_a_manually_deleted_checkout() {
        let repo = init_repo();
        let wt = create(repo.path(), "gone").unwrap();
        std::fs::remove_dir_all(&wt.path).unwrap();
        remove(repo.path(), "gone", false).unwrap();
        assert!(list(repo.path()).unwrap().is_empty());
    }

    #[test]
    fn current_branch_reports_head_and_worktree_branches() {
        let repo = init_repo();
        assert_eq!(current_branch(repo.path()).as_deref(), Some("main"));
        let wt = create(repo.path(), "feature").unwrap();
        assert_eq!(current_branch(&wt.path).as_deref(), Some("podium/feature"));
        // A non-git dir has no branch.
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(current_branch(plain.path()), None);
    }

    #[test]
    fn non_git_dir_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            create(dir.path(), "x"),
            Err(CoreError::NotAGitRepo)
        ));
        assert!(matches!(list(dir.path()), Err(CoreError::NotAGitRepo)));
    }
}
