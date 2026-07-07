// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // `Podium mcp-bridge` runs the headless stdio ↔ HTTP MCP bridge instead
    // of the GUI: external MCP clients launch it with a config line that
    // never changes across app restarts.
    if std::env::args().nth(1).as_deref() == Some("mcp-bridge") {
        return podium_lib::run_mcp_bridge();
    }
    podium_lib::run();
    std::process::ExitCode::SUCCESS
}
