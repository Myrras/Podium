//! Process domain types plus the PTY and scrollback machinery.

pub mod pty;
pub mod scrollback;
pub mod supervisor;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ProcessId, ProjectId};

/// What kind of process this is, which drives UI affordances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProcessKind {
    Service,
    Terminal,
    Agent { adapter: String },
}

/// Lifecycle state of a managed process.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ProcessStatus {
    NotStarted,
    Running {
        pid: u32,
        since: DateTime<Utc>,
    },
    Stopping,
    Exited {
        code: Option<i32>,
        crashed: bool,
        at: DateTime<Utc>,
    },
}

/// Whether the supervisor should restart the process after it exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    #[default]
    Never,
    OnCrash,
    Always,
}

/// Everything needed to launch a process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSpec {
    pub name: String,
    pub command: String,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub kind: ProcessKind,
    pub restart_policy: RestartPolicy,
}

/// Read-only snapshot of a managed process, for listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub id: ProcessId,
    pub project_id: ProjectId,
    pub name: String,
    pub kind: ProcessKind,
    pub status: ProcessStatus,
    pub restart_policy: RestartPolicy,
    pub command: String,
}
