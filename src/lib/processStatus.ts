/** Presentation helpers for `ProcessStatus` (shared by dot, rows, pane). */

import type { ProcessStatus } from "../ipc/types";
import type { AgentActivity } from "./useAgentActivity";

/**
 * Short human label for an agent's activity (sidebar row / pane hint).
 * `shells` qualifies it with the agent's background shells — an idle agent
 * with a build still running reads "idle · 1 shell", not just "idle".
 */
export function activityLabel(activity: AgentActivity, shells = 0): string {
  const base =
    activity === "working"
      ? "working…"
      : activity === "waiting"
        ? "needs input"
        : "idle";
  if (shells === 0) return base;
  return `${base} · ${shells} ${shells === 1 ? "shell" : "shells"}`;
}

/** Human-readable one-liner for a process status. */
export function statusLabel(status: ProcessStatus): string {
  switch (status.state) {
    case "notStarted":
      return "Not started";
    case "running":
      return `Running · pid ${status.pid}`;
    case "stopping":
      return "Stopping…";
    case "exited":
      return status.crashed
        ? `Crashed · ${status.code === null ? "signal" : `code ${status.code}`}`
        : `Exited · ${status.code === null ? "signal" : `code ${status.code}`}`;
  }
}

/** Whether the process can currently receive input / be stopped. */
export function isActive(status: ProcessStatus): boolean {
  return status.state === "running" || status.state === "stopping";
}
