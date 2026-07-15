/**
 * Typed wrappers over the global Tauri events emitted by `src-tauri`'s event
 * forwarder (`events.rs`). These are low-volume lifecycle notifications;
 * terminal output streams over per-attach channels instead (see
 * `processAttach` in `commands.ts`).
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ProcessRefEvent,
  ProcessStatusEvent,
  ProjectRefEvent,
} from "./types";

export function onProcessAdded(
  handler: (event: ProcessRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProcessRefEvent>("process:added", (e) => handler(e.payload));
}

export function onProcessRemoved(
  handler: (event: ProcessRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProcessRefEvent>("process:removed", (e) => handler(e.payload));
}

export function onProcessStatus(
  handler: (event: ProcessStatusEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProcessStatusEvent>("process:status", (e) =>
    handler(e.payload),
  );
}

/** A process's metadata changed (e.g. it was renamed). */
export function onProcessUpdated(
  handler: (event: ProcessRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProcessRefEvent>("process:updated", (e) => handler(e.payload));
}

export function onProjectOpened(
  handler: (event: ProjectRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProjectRefEvent>("project:opened", (e) => handler(e.payload));
}

/** Project metadata changed (e.g. `podium.yml` was reloaded). */
export function onProjectUpdated(
  handler: (event: ProjectRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProjectRefEvent>("project:updated", (e) => handler(e.payload));
}

export function onProjectClosed(
  handler: (event: ProjectRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProjectRefEvent>("project:closed", (e) => handler(e.payload));
}

/** A project's to-do list changed (from this UI or an agent over MCP). */
export function onTodosChanged(
  handler: (event: ProjectRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProjectRefEvent>("todo:changed", (e) => handler(e.payload));
}

/** A project's scratchpads changed (from this UI or an agent over MCP). */
export function onScratchpadsChanged(
  handler: (event: ProjectRefEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProjectRefEvent>("scratchpad:changed", (e) =>
    handler(e.payload),
  );
}

/** The native macOS menu's Settings… item (⌘,) was activated (`lib.rs`). */
export function onMenuOpenSettings(handler: () => void): Promise<UnlistenFn> {
  return listen("menu:open-settings", () => handler());
}

/**
 * A close/quit was blocked because agents or terminals are still running
 * (`lib.rs`). The frontend responds by showing the close-warning dialog.
 */
export function onWindowCloseRequested(
  handler: () => void,
): Promise<UnlistenFn> {
  return listen("window:close-requested", () => handler());
}
