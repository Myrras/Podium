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
    /// Subtle UI colour identifying an agent session (assigned at spawn from a
    /// palette, avoiding colours other live agents already use). `None` for
    /// services/terminals. Surfaced to the UI via a to-do's `AssignedAgent`.
    #[serde(default)]
    pub color: Option<String>,
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
    /// Name of the Podium-managed worktree the process runs in, derived from
    /// its cwd (`…/.podium/worktrees/<name>`); `None` for the project root.
    pub worktree: Option<String>,
    /// Subtle UI colour of an agent session (from its spec), so the sidebar
    /// row can be tinted to match the to-dos this session owns. `None` for
    /// services/terminals.
    pub color: Option<String>,
}

/// The worktree name a cwd implies: the directory name when its parent is
/// `worktrees` under `.podium` (how [`crate::worktree::create`] lays paths
/// out), else `None`. Derived rather than stored so it can never go stale.
// ponytail: a worktree created mid-session over MCP doesn't change the
// process's cwd, so it won't get a badge or removal guard this way.
pub fn worktree_name_from_cwd(cwd: &std::path::Path) -> Option<String> {
    let name = cwd.file_name()?.to_str()?.to_string();
    let parent = cwd.parent()?;
    if parent.file_name()? != "worktrees" || parent.parent()?.file_name()? != ".podium" {
        return None;
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn worktree_name_is_derived_from_the_cwd_layout() {
        assert_eq!(
            worktree_name_from_cwd(Path::new("/repo/.podium/worktrees/fix-login")),
            Some("fix-login".to_string())
        );
        assert_eq!(worktree_name_from_cwd(Path::new("/repo")), None);
        assert_eq!(
            worktree_name_from_cwd(Path::new("/repo/.podium/other/fix-login")),
            None
        );
        assert_eq!(
            worktree_name_from_cwd(Path::new("/repo/worktrees/fix-login")),
            None
        );
    }
}
