/**
 * Small helpers around the assistant's sub-agents (`SubagentInfo`).
 *
 * A sub-agent is an ordinary agent process, but it belongs to the assistant's
 * internal scope and never appears in `process_list` — so the process store
 * never holds it. The few views that need a `ProcessInfo` for one (the work
 * area's terminal pane, the activity monitor) take it from here instead.
 */

import type { ProcessInfo, SubagentInfo } from "../ipc/types";

/** Whether the sub-agent still holds (or waits for) a slot. */
export function isSubagentLive(s: SubagentInfo): boolean {
  return s.phase === "queued" || s.phase === "running" || s.phase === "waiting";
}

/**
 * A `ProcessInfo` for a sub-agent, built from its snapshot. Only the fields
 * the terminal pane and the activity monitor read are meaningful: there is
 * no user project behind it (`projectId` is empty), and the adapter is the
 * one every sub-agent launches with (`SubagentConfig::adapter_id`'s default).
 */
export function subagentProcess(s: SubagentInfo): ProcessInfo {
  return {
    id: s.id,
    projectId: "",
    name: s.name,
    kind: { kind: "agent", adapter: "claude-code" },
    status: s.status,
    restartPolicy: "never",
    command: "",
    worktree: null,
    color: null,
    // Never reported as viewed: sub-agents are not in the process store.
    viewing: false,
    subagent: {
      kind: s.kind,
      task: s.task,
      phase: s.phase,
      endedAt: s.endedAt,
    },
  };
}

/**
 * How long the sub-agent has run (live while running, frozen once ended);
 * `null` when it never started (queued, or cancelled while queued).
 */
export function subagentElapsedMs(
  s: SubagentInfo,
  now: number = Date.now(),
): number | null {
  if (!s.startedAt) return null;
  const start = Date.parse(s.startedAt);
  const end = s.endedAt ? Date.parse(s.endedAt) : now;
  return Math.max(0, end - start);
}

/** Compact running time: "42s", "3m 05s", "1h 02m". */
export function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (h > 0) return `${h}h ${pad(m)}m`;
  if (m > 0) return `${m}m ${pad(sec)}s`;
  return `${sec}s`;
}

/**
 * "2 running · 1 needs input · 1 queued", leaving out the zeros; "" when
 * nothing is live.
 */
export function subagentSummary(subagents: SubagentInfo[]): string {
  const running = subagents.filter((a) => a.phase === "running").length;
  const queued = subagents.filter((a) => a.phase === "queued").length;
  const waiting = subagents.filter((a) => a.phase === "waiting").length;
  return [
    running > 0 ? `${running} running` : "",
    waiting > 0 ? `${waiting} needs input` : "",
    queued > 0 ? `${queued} queued` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}
