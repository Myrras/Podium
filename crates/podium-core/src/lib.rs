//! podium-core — the UI-agnostic heart of Podium.
//!
//! Owns projects, processes (PTYs), agent adapters, and the built-in MCP
//! server. Has **zero** Tauri dependency: the Tauri shell (`src-tauri`) and
//! the MCP tools are both thin adapters over [`Orchestrator`]'s public API.

#![forbid(unsafe_code)]

pub mod agent;
pub mod assignment;
pub mod assistant;
pub mod backend_settings;
pub mod boards;
pub mod config;
pub mod error;
pub mod events;
pub mod ids;
pub mod issues;
pub mod justfile;
pub mod mcp;
pub mod orchestrator;
pub mod platform;
pub mod proactivity;
pub mod process;
pub mod project;
pub mod responses;
pub mod scratchpad;
pub mod todo;
pub mod worktree;

pub use agent::settings::{AdapterOverride, AgentSettings, MergeMode};
pub use agent::{
    AdapterInfo, AdapterRegistry, AgentAdapter, AgentLaunchCtx, LaunchPlan, LiveProcess,
    McpConnectInfo, RestartCommands,
};
pub use assignment::AssignedAgent;
pub use assistant::{
    AssistantConfig, AssistantEvent, AssistantSettings, Attachment, AttachmentInfo, AutonomyMode,
    PendingKind, ProposalInfo, QueuedMessage, TranscriptEntry, TranscriptRole,
};
pub use boards::CachedBoard;
pub use error::{CoreError, CoreResult};
pub use events::{EventBus, PodiumEvent};
pub use ids::{
    CommentId, LinkId, PendingId, ProcessId, ProjectId, ProposalId, ResponseId, ScratchpadId,
    TodoId,
};
pub use issues::{Board, IssueProvider, IssuesOverview, ItemKind};
pub use mcp::McpServer;
pub use orchestrator::{normalize_project_name, Orchestrator, ProjectInfo};
pub use proactivity::ProactivityConfig;
pub use process::pty::TermChunk;
pub use process::scrollback::OutputActivity;
pub use process::supervisor::SupervisorConfig;
pub use process::{ProcessInfo, ProcessKind, ProcessSpec, ProcessStatus, RestartPolicy};
pub use project::ProjectSections;
pub use responses::{LabelledResponse, ResponseItem, ResponseKind};
pub use scratchpad::ScratchpadInfo;
pub use todo::{TodoComment, TodoInfo, TodoLink, TodoSession, TodoSessionInfo};
pub use worktree::WorktreeInfo;
