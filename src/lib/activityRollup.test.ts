import { describe, expect, it } from "vitest";

import type { ProcessInfo } from "../ipc/types";
import type { ProcessActivity } from "../state/agentActivityStore";
import { activityRollup } from "./activityRollup";

function proc(
  id: string,
  kind: "agent" | "terminal",
  state: "running" | "notStarted" = "running",
): ProcessInfo {
  return {
    id,
    projectId: "p",
    name: id,
    kind: { kind } as ProcessInfo["kind"],
    status:
      state === "running"
        ? { state: "running", pid: 1, since: "2026-08-20T10:00:00Z" }
        : { state: "notStarted" },
    restartPolicy: "never",
    command: "x",
    worktree: null,
    color: null,
    viewing: false,
  } as ProcessInfo;
}

const act = (
  activity: ProcessActivity["activity"],
  reported = true,
): ProcessActivity => ({
  activity,
  reported,
  waitingFor: null,
  sinceMs: null,
  subagents: 0,
  shells: 0,
});

describe("activityRollup", () => {
  it("counts working and needs by pill state", () => {
    const procs = [
      proc("a1", "agent"),
      proc("a2", "agent"),
      proc("a3", "agent"),
      proc("t1", "terminal"),
      proc("t2", "terminal"),
      proc("t3", "terminal", "notStarted"),
    ];
    const details = {
      a1: act("working"),
      a2: act("waiting"),
      a3: act("idle"),
      t1: act("working", false), // unreported terminal: just "up"
      t3: act("waiting"), // not running: ignored
    };
    expect(activityRollup(procs, details)).toEqual({ working: 1, needs: 1 });
  });

  it("is zero for no processes or untracked ones", () => {
    expect(activityRollup([], {})).toEqual({ working: 0, needs: 0 });
    expect(activityRollup([proc("a", "agent")], {})).toEqual({
      working: 0,
      needs: 0,
    });
  });
});
