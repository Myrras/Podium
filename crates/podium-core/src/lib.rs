//! podium-core — the UI-agnostic heart of Podium.
//!
//! Owns projects, processes (PTYs), agent adapters, and the built-in MCP
//! server. Has **zero** Tauri dependency: the Tauri shell (`src-tauri`) and
//! the MCP tools are both thin adapters over [`Orchestrator`]'s public API.

#![forbid(unsafe_code)]

pub mod agent;
pub mod assignment;
pub mod config;
pub mod error;
pub mod events;
pub mod ids;
pub mod mcp;
pub mod orchestrator;
pub mod platform;
pub mod process;
pub mod project;
pub mod scratchpad;
pub mod todo;
pub mod worktree;

pub use agent::settings::{AdapterOverride, AgentSettings, MergeMode};
pub use agent::{
    AdapterInfo, AdapterRegistry, AgentAdapter, AgentLaunchCtx, LaunchPlan, McpConnectInfo,
};
pub use assignment::AssignedAgent;
pub use error::{CoreError, CoreResult};
pub use events::{EventBus, PodiumEvent};
pub use ids::{CommentId, LinkId, ProcessId, ProjectId, ScratchpadId, TodoId};
pub use mcp::McpServer;
pub use orchestrator::{Orchestrator, ProjectInfo};
pub use process::pty::TermChunk;
pub use process::supervisor::SupervisorConfig;
pub use process::{ProcessInfo, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy};
pub use scratchpad::ScratchpadInfo;
pub use todo::{TodoComment, TodoInfo, TodoLink};
pub use worktree::WorktreeInfo;
