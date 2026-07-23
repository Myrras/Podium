//! The Podium desktop shell: a thin Tauri 2 IPC adapter over `podium-core`.
//!
//! This crate owns no orchestration logic — it (de)serializes command
//! arguments, holds the live application state, and forwards work to
//! `podium-core`. High-volume terminal data streams to the frontend over
//! per-attach `tauri::ipc::Channel`s.
//!
//! ## Logging & secrets
//! Logging is configured via `tauri-plugin-log` (a global `log` logger at
//! `INFO`); `tracing` events fall through to it (the `tracing/log` feature).
//! Terminal output and the MCP bearer token are never logged.

#![forbid(unsafe_code)]

mod commands;
mod error;
mod events;
mod state;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_log::{Target, TargetKind};

/// Global event the webview listens for to raise the "active processes" close
/// warning. Emitted when a close/exit is blocked because agents or terminals
/// are still running.
const CLOSE_REQUESTED_EVENT: &str = "window:close-requested";

use crate::state::AppState;

/// Build and run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Native folder picker for opening projects.
        .plugin(tauri_plugin_dialog::init())
        // Native OS notifications (e.g. when an agent stalls needing input).
        .plugin(tauri_plugin_notification::init())
        // Persist window size/position across runs.
        .plugin(tauri_plugin_window_state::Builder::new().build())
        // Opens to-do links and markdown links in the OS default browser.
        .plugin(tauri_plugin_opener::init())
        // Logging sinks: stdout (dev), the per-OS log directory (support), and
        // the webview console (in-app diagnostics).
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .manage(AppState::new())
        // Warn before the red close button quits Podium while agents or
        // terminals are still running (exiting SIGKILLs every process group).
        // `prevent_close` keeps the window; the webview shows the warning and
        // calls `window_confirm_close` to go through with it. Cmd+Q / app-quit
        // takes the `RunEvent::ExitRequested` path below instead.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.force_close.load(Ordering::SeqCst)
                    || !state.orchestrator.has_active_agents_or_terminals()
                {
                    return;
                }
                api.prevent_close();
                if let Err(e) = window.emit(CLOSE_REQUESTED_EVENT, ()) {
                    tracing::warn!("failed to emit close-requested to webview: {e}");
                }
            }
        })
        .setup(|app| {
            // macOS: native menu bar with Settings… (⌘,) in the app menu; the
            // other platforms use the in-app title-bar gear instead. The menu
            // item emits a Tauri event the frontend listens for.
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};

                let settings = MenuItem::with_id(
                    app,
                    "settings",
                    "Settings\u{2026}", // "Settings…"
                    true,
                    Some("CmdOrCtrl+,"),
                )?;
                let app_menu = SubmenuBuilder::new(app, "Podium")
                    .about(None)
                    .separator()
                    .item(&settings)
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                // Setting a custom menu replaces the default one, so Edit must
                // be re-added or Cmd+C/V/X stop working in the webview.
                let edit_menu = SubmenuBuilder::new(app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .separator()
                    .select_all()
                    .build()?;
                let menu = MenuBuilder::new(app)
                    .item(&app_menu)
                    .item(&edit_menu)
                    .build()?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| {
                    if event.id() == "settings" {
                        let _ = app.emit("menu:open-settings", ());
                    }
                });
            }

            // Forward core lifecycle events to the webview for the app's
            // whole lifetime.
            let state = app.state::<AppState>();
            events::spawn_forwarder(app.handle().clone(), Arc::clone(&state.orchestrator));
            // Per-project to-dos persist in the app data dir (keyed by
            // project root, so they survive restarts).
            state
                .orchestrator
                .set_todos_path(app.path().app_data_dir()?.join("todos.json"));
            // Per-project scratchpads persist the same way, keyed by project
            // root.
            state
                .orchestrator
                .set_scratchpads_path(app.path().app_data_dir()?.join("scratchpads.json"));
            // Global agent settings (command override + default args per
            // adapter, merge mode) persist in the app data dir too.
            state
                .orchestrator
                .set_agent_settings_path(app.path().app_data_dir()?.join("agents.json"));
            // Start the built-in MCP server (ephemeral localhost port, per-run
            // bearer token). Agents get the URL + token via 0600 config files
            // under `{app_data_dir}/mcp`, which is wiped on every start. A
            // failure is logged but not fatal: Podium works without MCP.
            let orchestrator = Arc::clone(&state.orchestrator);
            let handle = app.handle().clone();
            let config_dir = app.path().app_data_dir()?.join("mcp");
            tauri::async_runtime::spawn(async move {
                match podium_core::mcp::start(orchestrator, config_dir).await {
                    Ok(server) => {
                        let state = handle.state::<AppState>();
                        *state.mcp.lock().expect("mcp state lock poisoned") = Some(server);
                    }
                    Err(e) => tracing::error!("failed to start mcp server: {e}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::project::project_open,
            commands::project::project_close,
            commands::project::project_list,
            commands::project::project_config_reload,
            commands::project::project_rename,
            commands::project::project_reorder,
            commands::recents::recents_list,
            commands::recents::recents_remove,
            commands::workspace::workspace_list,
            commands::workspace::workspace_remove,
            commands::process::process_add,
            commands::process::process_remove,
            commands::process::process_list,
            commands::process::process_git_branch,
            commands::process::process_rename,
            commands::process::process_start,
            commands::process::process_stop,
            commands::process::process_restart,
            commands::process::process_write,
            commands::process::process_resize,
            commands::process::process_attach,
            commands::agent::adapters_list,
            commands::agent::agent_spawn,
            commands::agent::agent_settings_get,
            commands::agent::agent_settings_set_adapter,
            commands::agent::agent_settings_set_default_adapter,
            commands::agent::agent_settings_set_merge_mode,
            commands::agent::agent_settings_set_suggest_worktree,
            commands::mcp::mcp_status,
            commands::mcp::mcp_clients_status,
            commands::mcp::mcp_client_install,
            commands::todo::todo_list,
            commands::todo::todo_list_archived,
            commands::todo::todo_set_archived,
            commands::todo::todo_add,
            commands::todo::todo_set_done,
            commands::todo::todo_update,
            commands::todo::todo_comment,
            commands::todo::todo_comment_update,
            commands::todo::todo_comment_remove,
            commands::todo::todo_add_link,
            commands::todo::todo_remove_link,
            commands::todo::todo_remove,
            commands::todo::todo_unassign,
            commands::scratchpad::scratchpad_list,
            commands::scratchpad::scratchpad_list_archived,
            commands::scratchpad::scratchpad_add,
            commands::scratchpad::scratchpad_update_content,
            commands::scratchpad::scratchpad_update_title,
            commands::scratchpad::scratchpad_add_tag,
            commands::scratchpad::scratchpad_remove_tag,
            commands::scratchpad::scratchpad_set_archived,
            commands::scratchpad::scratchpad_remove,
            commands::scratchpad::scratchpad_unassign,
            commands::window::window_confirm_close,
            commands::worktree::worktree_list,
            commands::worktree::worktree_remove,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| match event {
        // Cmd+Q / app-quit path (the window's CloseRequested guard covers the
        // red close button). Warn instead of quitting while agents or
        // terminals run, unless the user already confirmed via
        // `window_confirm_close`.
        RunEvent::ExitRequested { api, .. } => {
            let state = app_handle.state::<AppState>();
            if !state.force_close.load(Ordering::SeqCst)
                && state.orchestrator.has_active_agents_or_terminals()
            {
                api.prevent_exit();
                if let Some(window) = app_handle.get_webview_window("main") {
                    if let Err(e) = window.emit(CLOSE_REQUESTED_EVENT, ()) {
                        tracing::warn!("failed to emit close-requested to webview: {e}");
                    }
                }
            }
        }
        RunEvent::Exit => {
            // NOTE: `{app_data_dir}/mcp` (server.json + per-agent configs)
            // is intentionally left in place — it is wiped and rewritten on
            // the next start, and the stdio bridge polls it to reconnect.
            // Best effort: stop the MCP server and SIGTERM every managed
            // process group before the app dies (the SIGKILL escalation may
            // not get to run).
            let state = app_handle.state::<AppState>();
            if let Ok(guard) = state.mcp.lock() {
                if let Some(server) = guard.as_ref() {
                    server.stop();
                }
            }
            let orchestrator = Arc::clone(&state.orchestrator);
            tauri::async_runtime::block_on(orchestrator.shutdown());
        }
        _ => {}
    });
}

/// Run the `mcp-bridge` subcommand: a headless stdio ↔ streamable-HTTP proxy
/// external MCP clients launch instead of connecting to the built-in server
/// directly (its port and bearer token rotate every app run). Connection
/// details come from the 0600 `mcp/server.json` in the app data dir;
/// `PODIUM_APP_DATA_DIR` overrides that dir for non-standard setups.
pub fn run_mcp_bridge() -> std::process::ExitCode {
    use std::process::ExitCode;

    // Mirror Tauri's `app_data_dir` ({data_dir}/{identifier}) without
    // booting a Tauri app; the identifier matches `tauri.conf.json`.
    let data_dir = std::env::var_os("PODIUM_APP_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::data_dir().map(|dir| dir.join("com.podium.app")));
    let Some(data_dir) = data_dir else {
        eprintln!("podium mcp-bridge: could not resolve the app data directory");
        return ExitCode::FAILURE;
    };
    let server_json = data_dir.join("mcp").join("server.json");

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("podium mcp-bridge: failed to start async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    // stdout is the MCP protocol channel — only stderr may carry text.
    match runtime.block_on(podium_core::mcp::bridge::run_stdio(server_json)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("podium mcp-bridge: {e}");
            ExitCode::FAILURE
        }
    }
}
