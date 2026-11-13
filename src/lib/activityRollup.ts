/** How many of some processes are working / need the user, by pill state. */

import type { ProcessInfo } from "../ipc/types";
import { UNTRACKED_ACTIVITY } from "../state/agentActivityStore";
import type { ProcessActivity } from "./useAgentActivity";
import { pillState } from "./processStatus";

export interface ActivityRollup {
  working: number;
  needs: number;
}

export function activityRollup(
  processes: readonly ProcessInfo[],
  details: Readonly<Record<string, ProcessActivity>>,
): ActivityRollup {
  const out = { working: 0, needs: 0 };
  for (const p of processes) {
    const state = pillState(
      p.status,
      p.kind.kind,
      details[p.id] ?? UNTRACKED_ACTIVITY,
    );
    if (state === "working" || state === "needs") out[state]++;
  }
  return out;
}
