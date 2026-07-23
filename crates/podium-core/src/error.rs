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

    #[error("agent limit reached: a project can run at most 8 agents at once")]
    AgentLimitReached,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("pty error: {0}")]
    Pty(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}
