/**
 * Thin, safe wrapper over `@tauri-apps/plugin-log`.
 *
 * SECURITY: we only ever log already-sanitized text — error messages and UI
 * lifecycle notes. We NEVER log command output, file contents, or secrets.
 * The plugin's writes go to the same sinks the Rust side configured.
 *
 * Logging failures are swallowed: diagnostics must never break the UI.
 */

import { error as logError, warn as logWarn } from "@tauri-apps/plugin-log";

/** Record a sanitized error to the app log. */
export function logErr(context: string, message: string): void {
  void logError(`${context}: ${message}`).catch(() => undefined);
}

/** Record a sanitized warning to the app log. */
export function logWarning(context: string, message: string): void {
  void logWarn(`${context}: ${message}`).catch(() => undefined);
}
