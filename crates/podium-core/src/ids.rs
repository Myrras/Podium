//! Strongly-typed UUID newtypes for projects, processes, and to-dos.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a fresh random (v4) id.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::from_str(s)?))
            }
        }
    };
}

uuid_newtype!(
    /// Identifies an open project.
    ProjectId
);
uuid_newtype!(
    /// Identifies a managed process within a project.
    ProcessId
);

/// Namespace for [`ProjectId::for_root`]'s v5 ids. Fixed forever: changing it
/// would change every project's id, breaking the ids agents remember.
const PROJECT_NAMESPACE: Uuid = Uuid::from_u128(0x6f1c_2b7e_4d0a_4c8e_9b52_3a7d_e1f0_c4a9);

impl ProjectId {
    /// The project id for a (canonical) root path: a v5 UUID of the path, so
    /// the same folder gets the same id on every app run. A resumed agent
    /// session still knows the `project_id` from its earlier turns, and that
    /// id must keep working against Podium's MCP tools after a restart.
    pub fn for_root(root: &Path) -> Self {
        Self(Uuid::new_v5(
            &PROJECT_NAMESPACE,
            root.to_string_lossy().as_bytes(),
        ))
    }
}

impl ProcessId {
    /// The UUID behind this id, for reusing it as an agent's CLI session id
    /// (and back, see [`ProcessId::from_session_id`]).
    pub fn from_session_id(session_id: &str) -> Option<Self> {
        Uuid::parse_str(session_id).ok().map(Self)
    }
}
uuid_newtype!(
    /// Identifies a to-do item within a project.
    TodoId
);
uuid_newtype!(
    /// Identifies a single comment on a to-do (for editing / removing it).
    CommentId
);
uuid_newtype!(
    /// Identifies a single issue/PR link on a to-do (for removing it).
    LinkId
);
uuid_newtype!(
    /// Identifies a scratchpad within a project.
    ScratchpadId
);
uuid_newtype!(
    /// Identifies one pending assistant proposal (a gated tool call awaiting
    /// the user's confirmation).
    ProposalId
);
uuid_newtype!(
    /// Identifies one message waiting in the assistant's send queue (a user
    /// turn, a proactivity nudge, or a proposal-outcome notice) so it can be
    /// edited or withdrawn before it is delivered.
    PendingId
);
uuid_newtype!(
    /// Identifies one item on the assistant's Responses list.
    ResponseId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_plain_uuid_string() {
        let id = ProcessId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn round_trips_through_from_str() {
        let id = ProjectId::new();
        let parsed: ProjectId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn project_id_for_root_is_stable_and_distinct_per_root() {
        let a = ProjectId::for_root(Path::new("/tmp/fixture-a"));
        assert_eq!(a, ProjectId::for_root(Path::new("/tmp/fixture-a")));
        assert_ne!(a, ProjectId::for_root(Path::new("/tmp/fixture-b")));
    }

    #[test]
    fn process_id_from_session_id_round_trips() {
        let id = ProcessId::new();
        assert_eq!(ProcessId::from_session_id(&id.to_string()), Some(id));
        assert_eq!(ProcessId::from_session_id("not-a-uuid"), None);
    }

    #[test]
    fn scratchpad_id_serializes_as_plain_uuid_string() {
        let id = ScratchpadId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
    }

    #[test]
    fn scratchpad_id_round_trips_through_from_str() {
        let id = ScratchpadId::new();
        let parsed: ScratchpadId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }
}
