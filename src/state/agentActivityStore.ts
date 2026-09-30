/**
 * Centralized process activity: "working" / "waiting" / "idle".
 *
 * A single polling loop (started once from App) derives each running
 * process's state once a second. "Working" comes from the backend
 * (`processActivity`): how long ago the process last printed and how long
 * that burst of output has lasted, read off its scrollback — so it covers
 * every running process, whether or not its terminal was ever opened here.
 * An otherwise quiet screen showing a permission/confirmation prompt means
 * "waiting" (needs the user); everything else is "idle" (done or paused).
 * Computing this in one place — rather than per row/pane — means the state
 * is derived once and the "needs input" alert fires exactly once per
 * transition, no matter how many views show the same process. Every running
 * process is tracked, not just agents: a terminal stalled on a `sudo`
 * password or a `[y/N]` confirmation needs the user exactly as much as an
 * agent does (the UI only surfaces "waiting" for non-agents —
 * "working"/"idle" is noise for a plain shell). The same screen read also
 * yields an agent's background-shell count, so "idle" can be qualified with
 * the work still running behind the prompt.
 *
 * Prompt detection is a frontend heuristic (see `detectInputPrompt`): neither
 * coding-agent CLIs nor ordinary commands emit structured PTY events, so it
 * reads what is drawn on the terminal. Unlike "working", it only covers
 * processes whose terminal has been opened at least once (otherwise there is
 * no screen to read).
 */

import { create } from "zustand";

import type { OutputActivity, ProcessId } from "../ipc/types";
import { detectBackgroundShells, detectInputPrompt } from "../lib/agentPrompt";
import { processActivity, processReportWaiting } from "../ipc/commands";
import { notifyAgentWaiting } from "../lib/notify";
import { readViewportText } from "../lib/terminalRegistry";
import { useLayoutStore } from "./layoutStore";
import { useProcessStore } from "./processStore";

/** The three states an agent surfaces to the user. */
export type AgentActivity = "working" | "waiting" | "idle";

/** Output younger than this counts as "working". */
const WORKING_WINDOW_MS = 2500;
/**
 * A burst shorter than this is a lone redraw — Claude Code repainting after
 * the pane was fitted, a prompt being drawn — not work. A working agent
 * animates its spinner for as long as it works, so its burst keeps growing.
 */
const MIN_BURST_MS = 300;
/** Poll cadence for the monitor loop. */
const POLL_INTERVAL_MS = 1000;

/** Whether the backend's output timing reads as recent, sustained output. */
export function isWorking(output: OutputActivity | undefined): boolean {
  return (
    output !== undefined &&
    output.quietMs <= WORKING_WINDOW_MS &&
    output.burstMs >= MIN_BURST_MS
  );
}

function computeActivity(
  processId: ProcessId,
  output: OutputActivity | undefined,
): {
  activity: AgentActivity;
  /** Whether the screen shows an input prompt right now, regardless of
   * recent output — one prompt on screen = one notification episode. */
  prompting: boolean;
  /** Background shells the agent reports on screen (0 when none). */
  shells: number;
} {
  const screen = readViewportText(processId);
  const prompting = screen !== null && detectInputPrompt(screen);
  const shells = screen === null ? 0 : detectBackgroundShells(screen);
  if (isWorking(output)) return { activity: "working", prompting, shells };
  return { activity: prompting ? "waiting" : "idle", prompting, shells };
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
   * Background shells per running-agent process id — an agent reading "idle"
   * may still have a build or test run going behind its prompt. Replaced
   * wholesale every tick: consumers select a plain number out of it, so the
   * map's identity churn never costs a re-render.
   */
  shells: Record<string, number>;
  /**
   * Agents already alerted (pinged, or seen by the user) for the prompt
   * currently on their screen. Kept out of `activity` (which drives UI) —
   * this only gates the notification. The flag lives as long as the prompt
   * does: viewing the agent acknowledges the episode (no ping on a later
   * switch-away), and only a fresh prompt after this one clears pings again.
   */
  notified: Record<string, true>;
  /**
   * One monitor step: recompute from the backend's output timing (keyed by
   * process id), fire alerts on new "waiting" transitions.
   */
  tick: (output: Record<ProcessId, OutputActivity>) => void;
}

export const useAgentActivityStore = create<AgentActivityState>((set, get) => ({
  activity: {},
  shells: {},
  notified: {},
  tick: (output) => {
    const { processes } = useProcessStore.getState();
    const prev = get().activity;
    const prevNotified = get().notified;
    const next: Record<string, AgentActivity> = {};
    const shells: Record<string, number> = {};
    const notified: Record<string, true> = {};
    let changed = false;

    for (const p of processes) {
      if (p.status.state !== "running") continue;
      const {
        activity,
        prompting,
        shells: count,
      } = computeActivity(p.id, output[p.id]);
      next[p.id] = activity;
      if (count > 0) shells[p.id] = count;
      if (prev[p.id] !== activity) changed = true;

      if (!prompting) {
        // No prompt on screen: the episode (if any) is over; the flag drops
        // so the next prompt alerts again.
        continue;
      }
      if (prevNotified[p.id] || isViewingAgent(p.id)) {
        // Already pinged, or the user is looking at the prompt right now —
        // either way this episode is acknowledged and never pings again
        // (also covers the working↔waiting flicker of a live prompt).
        notified[p.id] = true;
      } else if (activity === "waiting") {
        notifyAgentWaiting(p.name, p.kind.kind === "agent");
        // The assistant only ever heard about a stuck agent through the
        // watcher's five-minute silence timer, which cannot tell "waiting for
        // you" from "done thinking". This is the same one-per-episode
        // transition the notification uses, so it says so at once — and only
        // for agents, since the assistant reasons about those.
        if (p.kind.kind === "agent") void processReportWaiting(p.id);
        notified[p.id] = true;
      }
    }
    // Processes that stopped/were removed drop out of both maps.
    for (const id of Object.keys(prev)) {
      if (!(id in next)) {
        changed = true;
        break;
      }
    }

    set((s) => ({
      activity: changed ? next : s.activity,
      shells,
      notified,
    }));
  },
}));

/**
 * Start the activity monitor. Call once (App mount); returns a stop function.
 * Each tick asks the backend for output timing first and schedules the next
 * tick only after applying it, so a slow answer can never let two ticks
 * overlap and race each other's writes.
 */
export function startAgentActivityMonitor(): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;
  const poll = async () => {
    // A failed poll is one idle tick, not worth a log line every second.
    const output = await processActivity().catch(() => ({}));
    if (stopped) return;
    useAgentActivityStore.getState().tick(output);
    timer = setTimeout(poll, POLL_INTERVAL_MS);
  };
  timer = setTimeout(poll, POLL_INTERVAL_MS);
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}
