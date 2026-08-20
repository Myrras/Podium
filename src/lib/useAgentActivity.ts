/**
 * Read one agent's activity ("working" / "waiting" / "idle") and its
 * background-shell count for rendering.
 *
 * The state is derived centrally by the activity monitor (see
 * `agentActivityStore`); this hook just subscribes to a single process's slot,
 * so a component only re-renders when that agent's activity actually changes.
 * Non-agent / not-yet-tracked processes read as "idle".
 */

import type { ProcessId } from "../ipc/types";
import {
  type AgentActivity,
  useAgentActivityStore,
} from "../state/agentActivityStore";

export type { AgentActivity };

/** The current activity for `processId` (defaults to "idle" when untracked). */
export function useAgentActivity(processId: ProcessId): AgentActivity {
  return useAgentActivityStore((s) => s.activity[processId] ?? "idle");
}

/**
 * How many background shells `processId` currently reports (0 when none or
 * untracked). Lets an "idle" agent still show that work is running behind it.
 */
export function useAgentShells(processId: ProcessId): number {
  return useAgentActivityStore((s) => s.shells[processId] ?? 0);
}
