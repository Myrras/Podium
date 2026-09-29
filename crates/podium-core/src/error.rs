//! Error types shared across podium-core.

use thiserror::Error;

/// Convenience alias used by all fallible core APIs.
pub type CoreResult<T> = Result<T, CoreError>;

/// All errors surfaced by the podium-core public API.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("project not found")]
    ProjectNotFound,

    #[error("process not found")]
    ProcessNotFound,

    #[error("to-do not found")]
    TodoNotFound,

    #[error("to-do is already assigned to another session")]
    TodoAlreadyAssigned,

    #[error("comment not found")]
    CommentNotFound,

    #[error("link not found")]
    LinkNotFound,

    #[error("scratchpad not found")]
    ScratchpadNotFound,

    #[error("scratchpad conflict: it was updated by someone else since you last loaded it")]
    ScratchpadConflict,

    #[error("not a git repository: worktrees need the project to be a git repo")]
    NotAGitRepo,

    #[error("worktree not found")]
    WorktreeNotFound,

    #[error("worktree has uncommitted changes — force to remove it anyway")]
    WorktreeDirty,

    #[error("a process is still running in this worktree; stop it first")]
    WorktreeInUse,

    /// A git invocation failed; the message is Podium-owned text (git's own
    /// output is never captured into errors).
    #[error("git error: {0}")]
    Git(String),

    #[error("process is already running")]
    ProcessAlreadyRunning,

    #[error("process is not running")]
    ProcessNotRunning,

    #[error("proposal not found: it was already confirmed, rejected or lost on restart")]
    ProposalNotFound,

    #[error("queued message not found: it was already delivered, removed, or lost when the session ended")]
    QueuedMessageNotFound,

    #[error("agent limit reached: a project can run at most 8 agents at once")]
    AgentLimitReached,

    #[error("No GitHub or GitLab remote found.")]
    IssuesNoRemote,

    #[error("The GitHub CLI (gh) is missing or not logged in.")]
    IssuesGhUnavailable,

    #[error("No setup in Settings → Issues matches this remote's host.")]
    IssuesUnsupportedHost,

    #[error("The selected setup \"{setup}\" is not for {remote}, this project's remote host.")]
    IssuesHostMismatch { setup: String, remote: String },

    #[error("This GitLab setup has no token: add one in Settings → Issues.")]
    IssuesGitlabNotConfigured,

    #[error("GitLab rejected the token: check Settings → Issues.")]
    IssuesGitlabAuth,

    /// Podium-owned text only; the tool's own output is never included.
    #[error("Could not fetch issues: {0}")]
    IssuesFetch(String),

    /// A refused or failed board move; Podium-owned text only.
    #[error("{0}")]
    IssuesMove(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pty error: {0}")]
    Pty(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}
