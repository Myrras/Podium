//! IPC command modules. Each command is a thin (de)serialization shim over
//! [`podium_core::Orchestrator`]; no orchestration logic lives here.

pub mod agent;
pub mod mcp;
pub mod process;
pub mod project;
pub mod recents;
pub mod scratchpad;
pub mod todo;
pub mod window;
pub mod workspace;
