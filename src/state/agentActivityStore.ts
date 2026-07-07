/**
 * Centralized agent activity: "working" / "waiting" / "idle".
 *
 * A single polling loop (started once from App) derives each running agent's
 * state from its terminal — recent output means "working"; an otherwise quiet
 * screen showing a permission/confirmation prompt means "waiting" (needs the
 * user); everything else is "idle" (done or paused). Computing this in one
 * place — rather than per row/pane — means the state is derived once and the
 * "needs input" alert fires exactly once per transition, no matter how many
 * views show the same agent.
 *
 * Detection is a frontend heuristic (see `detectInputPrompt`): coding-agent
 * CLIs emit no structured PTY events, and the existing "working" signal is
 * already output-timestamp based, so this only reads what the agent has
 * printed — no extra IPC. Like that signal, it only covers agents whose
 * terminal has been opened at least once (otherwise there's no buffer to read).
 */

import { create } from "zustand";

import type { ProcessId } from "../ipc/types";
import { detectInputPrompt } from "../lib/agentPrompt";
import { notifyAgentWaiting } from "../lib/notify";
import { getLastOutputAt, readViewportText } from "../lib/terminalRegistry";
import { useProcessStore } from "./processStore";

/** The three states an agent surfaces to the user. */
export type AgentActivity = "working" | "waiting" | "idle";

/** Output younger than this counts as "working". */
const WORKING_WINDOW_MS = 2500;
/** Poll cadence for the monitor loop. */
const POLL_INTERVAL_MS = 1000;

function computeActivity(processId: ProcessId): AgentActivity {
  const last = getLastOutputAt(processId);
  if (last !== null && Date.now() - last <= WORKING_WINDOW_MS) return "working";
  const screen = readViewportText(processId);
  if (screen !== null && detectInputPrompt(screen)) return "waiting";
  return "idle";
}

interface AgentActivityState {
  /** Latest activity per running-agent process id. */
  activity: Record<string, AgentActivity>;
  /** One monitor step: recompute, fire alerts on new "waiting" transitions. */
  tick: () => void;
}

export const useAgentActivityStore = create<AgentActivityState>((set, get) => ({
  activity: {},
  tick: () => {
    const { processes } = useProcessStore.getState();
    const prev = get().activity;
    const next: Record<string, AgentActivity> = {};
    let changed = false;

    for (const p of processes) {
      if (p.kind.kind !== "agent" || p.status.state !== "running") continue;
      const activity = computeActivity(p.id);
      next[p.id] = activity;
      const before = prev[p.id];
      if (before !== activity) {
        changed = true;
        // Alert only on the edge into "waiting" so we don't re-notify each
        // poll while the agent keeps sitting on the same prompt.
        if (activity === "waiting") notifyAgentWaiting(p.name);
      }
    }
    // Agents that stopped/were removed drop out of the map.
    if (!changed) {
      for (const id of Object.keys(prev)) {
        if (!(id in next)) {
          changed = true;
          break;
        }
      }
    }

    if (changed) set({ activity: next });
  },
}));

/**
 * Start the activity monitor. Call once (App mount); returns a stop function.
 */
export function startAgentActivityMonitor(): () => void {
  const timer = setInterval(
    () => useAgentActivityStore.getState().tick(),
    POLL_INTERVAL_MS,
  );
  return () => clearInterval(timer);
}
