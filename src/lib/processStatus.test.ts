import { describe, expect, it } from "vitest";

import type { ProcessStatus } from "../ipc/types";
import {
  type ProcessActivity,
  UNTRACKED_ACTIVITY,
} from "../state/agentActivityStore";
import { formatSince, pillState, pillWord } from "./processStatus";

describe("pillState", () => {
  const act = (patch: Partial<ProcessActivity>): ProcessActivity => ({
    ...UNTRACKED_ACTIVITY,
    ...patch,
  });
  const running: ProcessStatus = { state: "running", pid: 7, since: "now" };

  it("uses the lifecycle word when not running", () => {
    const idle = act({});
    expect(pillState({ state: "notStarted" }, "agent", idle)).toBe(
      "notStarted",
    );
    expect(pillState({ state: "stopping" }, "service", idle)).toBe("stopping");
    const exited = { code: 1, stopped: false } as const;
    expect(
      pillState(
        { state: "exited", ...exited, crashed: true } as ProcessStatus,
        "agent",
        idle,
      ),
    ).toBe("crashed");
    expect(
      pillState(
        { state: "exited", ...exited, crashed: false } as ProcessStatus,
        "terminal",
        idle,
      ),
    ).toBe("exited");
  });

  it("follows a running agent's activity", () => {
    expect(pillState(running, "agent", act({ activity: "working" }))).toBe(
      "working",
    );
    expect(pillState(running, "agent", act({ activity: "waiting" }))).toBe(
      "needs",
    );
    expect(pillState(running, "agent", act({ activity: "idle" }))).toBe("idle");
  });

  it("reads a terminal or service as Up unless it reports or waits", () => {
    expect(pillState(running, "terminal", act({ activity: "working" }))).toBe(
      "up",
    );
    expect(pillState(running, "service", act({ activity: "idle" }))).toBe("up");
    expect(pillState(running, "terminal", act({ activity: "waiting" }))).toBe(
      "needs",
    );
    expect(
      pillState(
        running,
        "terminal",
        act({ activity: "working", reported: true }),
      ),
    ).toBe("working");
    expect(
      pillState(running, "terminal", act({ activity: "idle", reported: true })),
    ).toBe("idle");
  });
});

describe("pillWord", () => {
  it("says Working for agents and Running for terminals", () => {
    expect(pillWord("working", "agent")).toBe("Working");
    expect(pillWord("working", "terminal")).toBe("Running");
    expect(pillWord("needs")).toBe("Needs you");
    expect(pillWord("notStarted")).toBe("Not started");
  });
});

describe("formatSince", () => {
  const now = 10_000_000;
  it("shows minutes, then hours and minutes", () => {
    expect(formatSince(now - 3 * 60_000, now)).toBe("3m");
    expect(formatSince(now - 64 * 60_000, now)).toBe("1h 4m");
  });

  it("shows nothing under a minute or when unknown", () => {
    expect(formatSince(now - 59_000, now)).toBe("");
    expect(formatSince(null, now)).toBe("");
  });
});
