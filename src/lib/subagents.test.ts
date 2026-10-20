import { describe, expect, it } from "vitest";

import type { SubagentInfo } from "../ipc/types";
import {
  formatElapsed,
  isSubagentLive,
  subagentElapsedMs,
  subagentProcess,
  subagentSummary,
} from "./subagents";

function subagent(overrides: Partial<SubagentInfo> = {}): SubagentInfo {
  return {
    id: "sub-1",
    name: "Ticket check",
    kind: "lookup",
    task: "Check the open tickets",
    phase: "running",
    status: { state: "running", pid: 7, since: "2026-01-01T10:00:00Z" },
    createdAt: "2026-01-01T10:00:00Z",
    startedAt: "2026-01-01T10:00:00Z",
    endedAt: null,
    expiresAt: null,
    result: null,
    error: null,
    question: null,
    ...overrides,
  };
}

describe("subagents helpers", () => {
  it("formats running times compactly", () => {
    expect(formatElapsed(0)).toBe("0s");
    expect(formatElapsed(42_900)).toBe("42s");
    expect(formatElapsed(185_000)).toBe("3m 05s");
    expect(formatElapsed(3_720_000)).toBe("1h 02m");
  });

  it("measures live while running, frozen once ended, none before a start", () => {
    const now = Date.parse("2026-01-01T10:01:00Z");
    expect(subagentElapsedMs(subagent(), now)).toBe(60_000);
    expect(
      subagentElapsedMs(
        subagent({ phase: "done", endedAt: "2026-01-01T10:00:30Z" }),
        now,
      ),
    ).toBe(30_000);
    expect(
      subagentElapsedMs(subagent({ phase: "queued", startedAt: null }), now),
    ).toBeNull();
  });

  it("counts only the live ones", () => {
    expect(isSubagentLive(subagent({ phase: "queued" }))).toBe(true);
    expect(isSubagentLive(subagent({ phase: "waiting" }))).toBe(true);
    expect(isSubagentLive(subagent({ phase: "stopped" }))).toBe(false);
    expect(
      subagentSummary([
        subagent(),
        subagent({ phase: "running" }),
        subagent({ phase: "waiting" }),
        subagent({ phase: "queued" }),
        subagent({ phase: "failed" }),
      ]),
    ).toBe("2 running · 1 needs input · 1 queued");
    expect(subagentSummary([subagent({ phase: "done" })])).toBe("");
  });

  it("builds an agent ProcessInfo carrying the sub-agent's meta", () => {
    const info = subagentProcess(
      subagent({ phase: "done", endedAt: "2026-01-01T10:00:30Z" }),
    );
    expect(info).toMatchObject({
      id: "sub-1",
      name: "Ticket check",
      kind: { kind: "agent" },
      subagent: {
        kind: "lookup",
        task: "Check the open tickets",
        phase: "done",
        endedAt: "2026-01-01T10:00:30Z",
      },
    });
  });
});
