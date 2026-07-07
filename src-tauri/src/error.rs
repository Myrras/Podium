//! The IPC-facing error type.
//!
//! Every command returns `Result<T, IpcError>`. `IpcError` is a flat,
//! serializable shape so the frontend can branch on a stable `kind` string and
//! show the (already-sanitized) `message`. Messages come from
//! [`CoreError`](podium_core::CoreError)'s `Display` (Podium-owned text) or
//! from IPC-layer validation here — never from terminal output or secrets.

use serde::Serialize;

use podium_core::CoreError;

/// A flat, serializable error returned by every IPC command.
///
/// `kind` is a stable machine-readable category (camelCase, matching the
/// [`CoreError`] variant, plus a few IPC-only kinds); `message` is a
/// human-readable, secret-free description.
#[derive(Debug, Clone, Serialize)]
pub struct IpcError {
    /// Human-readable, sanitized description (never contains secrets).
    pub message: String,
    /// Stable machine-readable category.
    pub kind: String,
}

impl IpcError {
    /// Build an `IpcError` from a `kind` discriminant and a (secret-free)
    /// message. Used for IPC-layer conditions that have no [`CoreError`]
    /// counterpart (e.g. malformed base64 or a path-traversal attempt).
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    /// Invalid command input detected at the IPC layer.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalidInput", message)
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.kind)
    }
}

impl std::error::Error for IpcError {}

impl From<CoreError> for IpcError {
    fn from(err: CoreError) -> Self {
        // The `message` is `CoreError`'s own `Display`, which is built from
        // Podium-owned text only — safe to forward verbatim.
        let kind = match &err {
            CoreError::ProjectNotFound => "projectNotFound",
            CoreError::ProcessNotFound => "processNotFound",
            CoreError::TodoNotFound => "todoNotFound",
            CoreError::CommentNotFound => "commentNotFound",
            CoreError::LinkNotFound => "linkNotFound",
            CoreError::ProcessAlreadyRunning => "processAlreadyRunning",
            CoreError::ProcessNotRunning => "processNotRunning",
            CoreError::AgentLimitReached => "agentLimitReached",
            CoreError::Io(_) => "io",
            CoreError::Pty(_) => "pty",
            CoreError::Config(_) => "config",
            CoreError::InvalidInput(_) => "invalidInput",
        };
        Self {
            kind: kind.to_string(),
            message: err.to_string(),
        }
    }
}
