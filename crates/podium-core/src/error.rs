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

    #[error("comment not found")]
    CommentNotFound,

    #[error("link not found")]
    LinkNotFound,

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
