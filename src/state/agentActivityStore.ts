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
import { useLayoutStore } from "./layoutStore";
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

/**
 * Is the user currently looking at this agent's terminal? True only when the
 * app window is focused, the agent is the active process, and no to-do pane is
 * covering the work area. Used to suppress the "needs input" ping — you don't
 * want a notification for the window you're already watching.
 */
function isViewingAgent(processId: ProcessId): boolean {
  if (!document.hasFocus()) return false;
  if (useLayoutStore.getState().openTodo !== null) return false;
  return useProcessStore.getState().activeProcessId === processId;
}

interface AgentActivityState {
  /** Latest activity per running-agent process id. */
  activity: Record<string, AgentActivity>;
  /**
   * Agents already pinged for their current unattended "waiting" episode. Kept
   * out of `activity` (which drives UI) — this only gates the notification.
   * Re-armed when the user views the agent, so a fresh look-away pings again.
   */
  notified: Record<string, true>;
  /** One monitor step: recompute, fire alerts on new "waiting" transitions. */
  tick: () => void;
}

export const useAgentActivityStore = create<AgentActivityState>((set, get) => ({
  activity: {},
  notified: {},
  tick: () => {
    const { processes } = useProcessStore.getState();
    const prev = get().activity;
    const prevNotified = get().notified;
    const next: Record<string, AgentActivity> = {};
    const notified: Record<string, true> = {};
    let changed = false;

    for (const p of processes) {
      if (p.kind.kind !== "agent" || p.status.state !== "running") continue;
      const activity = computeActivity(p.id);
      next[p.id] = activity;
      if (prev[p.id] !== activity) changed = true;

      if (isViewingAgent(p.id)) {
        // The user is watching this agent — no ping, and re-arm so a later
        // look-away while it's waiting alerts once more.
        continue;
      }
      if (prevNotified[p.id]) {
        // Already pinged for this unattended episode; keep the flag so the
        // working↔waiting flicker of a live prompt doesn't re-notify.
        notified[p.id] = true;
      } else if (activity === "waiting") {
        notifyAgentWaiting(p.name);
        notified[p.id] = true;
      }
    }
    // Agents that stopped/were removed drop out of both maps.
    for (const id of Object.keys(prev)) {
      if (!(id in next)) {
        changed = true;
        break;
      }
    }

    set((s) => ({
      activity: changed ? next : s.activity,
      notified,
    }));
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
