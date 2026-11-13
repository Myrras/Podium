/** Presentation helpers for `ProcessStatus` (shared by dot, rows, pane). */

import type { ProcessKind, ProcessStatus } from "../ipc/types";
import type { AgentActivity, ProcessActivity } from "./useAgentActivity";

/** What a status pill shows (see `StatusPill`). */
export type PillState =
  | "working"
  | "needs"
  | "idle"
  | "up"
  | "stopping"
  | "crashed"
  | "exited"
  | "notStarted";

/**
 * The pill for a process: its lifecycle when not running; a running agent's
 * activity; a running terminal or service is Running/Idle only when it
 * reported its state, Needs you when waiting, and otherwise just Up.
 */
export function pillState(
  status: ProcessStatus,
  kind: ProcessKind["kind"],
  activity: ProcessActivity,
): PillState {
  if (status.state === "exited") return status.crashed ? "crashed" : "exited";
  if (status.state !== "running") return status.state;
  if (activity.activity === "waiting") return "needs";
  if (kind !== "agent" && !activity.reported) return "up";
  return activity.activity === "working" ? "working" : "idle";
}

/** The pill's word; a working terminal or service reads "Running". */
export function pillWord(
  state: PillState,
  kind: ProcessKind["kind"] = "agent",
): string {
  switch (state) {
    case "working":
      return kind === "agent" ? "Working" : "Running";
    case "needs":
      return "Needs you";
    case "idle":
      return "Idle";
    case "up":
      return "Up";
    case "stopping":
      return "Stopping";
    case "crashed":
      return "Crashed";
    case "exited":
      return "Exited";
    case "notStarted":
      return "Not started";
  }
}

/** Time in the current state: "3m", "1h 4m"; "" under a minute or unknown. */
export function formatSince(sinceMs: number | null, now: number): string {
  if (sinceMs === null) return "";
  const min = Math.floor((now - sinceMs) / 60_000);
  if (min < 1) return "";
  const h = Math.floor(min / 60);
  return h > 0 ? `${h}h ${min % 60}m` : `${min}m`;
}

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
