import { beforeEach, describe, expect, it, vi } from "vitest";

// The process store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

const viewport = vi.fn<(id: string) => string | null>();
vi.mock("../lib/terminalRegistry", () => ({
  disposeTerminal: vi.fn(),
  readViewportText: (id: string) => viewport(id),
}));

const notifyAgentWaiting = vi.fn();
vi.mock("../lib/notify", () => ({
  notifyAgentWaiting: (name: string, isAgent?: boolean) =>
    notifyAgentWaiting(name, isAgent),
}));

const processReportWaiting = vi.fn<(id: string) => Promise<void>>();
const processReportViewing = vi.fn<(id: string | null) => Promise<void>>();
const processActivity = vi.fn<() => Promise<Record<string, OutputActivity>>>();
vi.mock("../ipc/commands", () => ({
  processReportWaiting: (id: string) => processReportWaiting(id),
  processReportViewing: (id: string | null) => processReportViewing(id),
  processActivity: () => processActivity(),
}));

import type {
  OutputActivity,
  ProcessInfo,
  ProcessKind,
  ProcessStatus,
} from "../ipc/types";
import { useAssistantStore } from "./assistantStore";
import { useLayoutStore } from "./layoutStore";
import { useProcessStore } from "./processStore";
import {
  startAgentActivityMonitor,
  useAgentActivityStore,
  viewedAgentId,
} from "./agentActivityStore";

function proc(
  id: string,
  kind: ProcessKind,
  status: ProcessStatus,
  name = id,
): ProcessInfo {
  return {
    id,
    projectId: "p",
    name,
    kind,
    status,
    restartPolicy: "never",
    command: "run",
    worktree: null,
    color: null,
    viewing: false,
  };
}

const RUNNING: ProcessStatus = { state: "running", pid: 1, since: "now" };
const AGENT: ProcessKind = { kind: "agent", adapter: "claude-code" };
/** Output timing alone: the process reported nothing itself. */
function timing(quietMs: number, burstMs: number): OutputActivity {
  return {
    quietMs,
    burstMs,
    state: null,
    waitingFor: null,
    stateMs: null,
    subagents: 0,
    shells: 0,
  };
}
/** A spinner that has been animating for a while: recent, sustained output. */
const WORKING = timing(100, 1200);
/** One redraw just now (a repaint after the pane was fitted): not work. */
const REDRAW = timing(50, 0);
/** A long-silent screen, with what the process reported itself on top. */
function reported(
  state: OutputActivity["state"],
  rest: Partial<OutputActivity> = {},
): OutputActivity {
  return { ...timing(60_000, 0), state, stateMs: 0, ...rest };
}

function seed(processes: ProcessInfo[]) {
  useProcessStore.setState({ processes });
}

/** One monitor step with the backend's output timing (none by default). */
function tick(output: Record<string, OutputActivity> = {}) {
  useAgentActivityStore.getState().tick(output);
}

describe("agentActivityStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAgentActivityStore.setState({
      activity: {},
      details: {},
      notified: {},
    });
    useProcessStore.setState({ activeProcessId: null });
    useAssistantStore.setState({ subagents: [] });
    useLayoutStore.setState({
      openTodo: null,
      openScratchpad: null,
      openBoard: null,
      openSubagent: null,
    });
    processReportViewing.mockResolvedValue(undefined);
    viewport.mockReturnValue(null);
    // Default: the user is not looking at any agent, so waiting agents ping.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
  });

  it("covers the assistant's running sub-agents, which the process store never holds", () => {
    seed([]);
    const since = "2026-01-01T10:00:00Z";
    const base = {
      kind: "full" as const,
      createdAt: since,
      endedAt: null,
      expiresAt: null,
      result: null,
      error: null,
      question: null,
    };
    useAssistantStore.setState({
      subagents: [
        {
          ...base,
          id: "sub-1",
          name: "Repo sweep",
          task: "Repo sweep",
          phase: "running",
          status: RUNNING,
          startedAt: since,
        },
        {
          ...base,
          id: "sub-2",
          name: "Queued sweep",
          task: "Queued sweep",
          phase: "queued",
          status: { state: "notStarted" },
          startedAt: null,
        },
      ],
    });
    viewport.mockImplementation((id) =>
      id === "sub-1" ? "Do you want to proceed?\n❯ 1. Yes" : null,
    );

    tick();
    expect(useAgentActivityStore.getState().activity).toEqual({
      "sub-1": "waiting",
    });
    expect(notifyAgentWaiting).toHaveBeenCalledWith("Repo sweep", true);
    // Reported like any agent, so the assistant hears of it at once.
    expect(processReportWaiting).toHaveBeenCalledWith("sub-1");
  });

  it("does not ping for a sub-agent whose terminal the user is watching", () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    useLayoutStore.setState({ openSubagent: "sub-1" });
    useAssistantStore.setState({
      subagents: [
        {
          id: "sub-1",
          name: "Repo sweep",
          kind: "full",
          task: "Repo sweep",
          phase: "running",
          status: RUNNING,
          createdAt: "2026-01-01T10:00:00Z",
          startedAt: "2026-01-01T10:00:00Z",
          endedAt: null,
          expiresAt: null,
          result: null,
          error: null,
          question: null,
        },
      ],
    });
    seed([]);
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");

    tick();
    expect(useAgentActivityStore.getState().activity["sub-1"]).toBe("waiting");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("marks recent sustained output as working", () => {
    seed([proc("a1", AGENT, RUNNING)]);
    tick({ a1: WORKING });
    expect(useAgentActivityStore.getState().activity.a1).toBe("working");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("does not mistake a lone redraw for work", () => {
    // Opening a terminal fits the pane, and the agent repaints once: a burst
    // of ~0 ms. That used to show "working…" for seconds on every open.
    seed([proc("a1", AGENT, RUNNING)]);
    tick({ a1: REDRAW });
    expect(useAgentActivityStore.getState().activity.a1).toBe("idle");
  });

  it("marks stale output as idle", () => {
    seed([proc("a1", AGENT, RUNNING)]);
    tick({ a1: timing(10_000, 5000) });
    expect(useAgentActivityStore.getState().activity.a1).toBe("idle");
  });

  it("polls the backend for output timing on each tick", async () => {
    vi.useFakeTimers();
    try {
      processActivity.mockResolvedValue({ a1: WORKING });
      seed([proc("a1", AGENT, RUNNING)]);
      const stop = startAgentActivityMonitor();
      await vi.advanceTimersByTimeAsync(1000);
      expect(processActivity).toHaveBeenCalledOnce();
      expect(useAgentActivityStore.getState().activity.a1).toBe("working");
      stop();
    } finally {
      vi.useRealTimers();
    }
  });

  it("marks a quiet prompt screen as waiting and notifies once", () => {
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    seed([proc("a1", AGENT, RUNNING, "claude")]);

    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");
    expect(notifyAgentWaiting).toHaveBeenCalledOnce();
    expect(notifyAgentWaiting).toHaveBeenCalledWith("claude", true);

    // The assistant is told too, so it can say what the agent is asking for
    // instead of waiting out the watcher's silence timer.
    expect(processReportWaiting).toHaveBeenCalledWith("a1");

    // Still on the same prompt next poll — no repeat alert.
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
    expect(processReportWaiting).toHaveBeenCalledTimes(1);
  });

  it("marks a quiet non-prompt screen as idle", () => {
    viewport.mockReturnValue("Compiled successfully.");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("idle");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("ignores non-running processes", () => {
    seed([proc("a2", AGENT, { state: "notStarted" })]);
    tick();
    expect(useAgentActivityStore.getState().activity).toEqual({});
  });

  it("flags a terminal stalled on a sudo password prompt", () => {
    viewport.mockReturnValue("$ sudo systemctl restart api\nPassword:");
    seed([proc("t1", { kind: "terminal" }, RUNNING, "shell")]);

    tick();
    expect(useAgentActivityStore.getState().activity.t1).toBe("waiting");
    expect(notifyAgentWaiting).toHaveBeenCalledWith("shell", false);
    // Only agents are reported to the assistant — it reasons about those.
    expect(processReportWaiting).not.toHaveBeenCalled();
  });

  it("leaves a quiet terminal idle", () => {
    viewport.mockReturnValue("tester@host ~/podium %");
    seed([proc("t1", { kind: "terminal" }, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.t1).toBe("idle");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("drops agents that are no longer running", () => {
    viewport.mockReturnValue("(y/n)");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");

    seed([
      proc("a1", AGENT, {
        state: "exited",
        code: 0,
        crashed: false,
        stopped: false,
        at: "now",
      }),
    ]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBeUndefined();
  });

  it("does not re-alert on working↔waiting flicker while unattended", () => {
    viewport.mockReturnValue("Continue? (y/n)");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

    // The agent briefly reads as "working" (a spinner while it re-checks),
    // then sits on the same prompt again — the user must not be pinged again.
    tick({ a1: WORKING });
    expect(useAgentActivityStore.getState().activity.a1).toBe("working");

    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
  });

  it("does not ping while the user is viewing the agent", () => {
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    vi.mocked(document.hasFocus).mockReturnValue(true);
    useProcessStore.setState({ activeProcessId: "a1" });
    seed([proc("a1", AGENT, RUNNING)]);

    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("does not re-ping after the user viewed the prompt and switches away", () => {
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    seed([proc("a1", AGENT, RUNNING)]);

    // Not looking → one ping.
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

    // User views the agent (focus + active): episode acknowledged.
    vi.mocked(document.hasFocus).mockReturnValue(true);
    useProcessStore.setState({ activeProcessId: "a1" });
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

    // Switching to another agent while the same prompt is still up must NOT
    // ping again — that was the repeatable spurious-notification bug.
    vi.mocked(document.hasFocus).mockReturnValue(false);
    useProcessStore.setState({ activeProcessId: null });
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
  });

  it("viewing the prompt before any ping also acknowledges the episode", () => {
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    vi.mocked(document.hasFocus).mockReturnValue(true);
    useProcessStore.setState({ activeProcessId: "a1" });
    seed([proc("a1", AGENT, RUNNING)]);

    tick();
    // Looking away afterwards: same prompt, already seen → no ping.
    vi.mocked(document.hasFocus).mockReturnValue(false);
    tick();
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("pings again for a fresh prompt after the previous one cleared", () => {
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

    // Prompt answered: screen no longer shows one → episode ends.
    viewport.mockReturnValue("Working on it…");
    tick();

    // A new prompt appears → a new episode pings once more.
    viewport.mockReturnValue("Overwrite file? (y/n)");
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(2);
  });

  it("tracks the background-shell count from the footer", () => {
    viewport.mockReturnValue(
      "⏵⏵ auto mode on (shift+tab to cycle) · 2 shells · ↓ to manage",
    );
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    // Idle agent, but work is still running behind its prompt.
    expect(useAgentActivityStore.getState().activity.a1).toBe("idle");
    expect(useAgentActivityStore.getState().details.a1.shells).toBe(2);
  });

  it("clears the shell count once the footer stops reporting one", () => {
    viewport.mockReturnValue("⏵⏵ auto mode on · 1 shell · ↓ to manage");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().details.a1.shells).toBe(1);

    viewport.mockReturnValue("⏵⏵ auto mode on (shift+tab to cycle)");
    tick();
    expect(useAgentActivityStore.getState().details.a1.shells).toBe(0);
  });
  describe("reported state", () => {
    const details = () => useAgentActivityStore.getState().details;

    it("wins over the output-timing heuristic", () => {
      seed([proc("a1", AGENT, RUNNING), proc("a2", AGENT, RUNNING)]);
      tick({
        // Thinking quietly: nothing printed for a minute, still working.
        a1: reported("working"),
        // A spinner still animating, but the agent says it is done.
        a2: { ...WORKING, state: "idle", stateMs: 0 },
      });
      const { activity } = useAgentActivityStore.getState();
      expect(activity).toEqual({ a1: "working", a2: "idle" });
      expect(details().a1).toMatchObject({
        activity: "working",
        reported: true,
      });
    });

    it("wins over the prompt on screen, also for the notification", () => {
      // Scrollback that merely looks like a prompt; the agent is working.
      viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
      seed([proc("a1", AGENT, RUNNING, "claude")]);
      tick({ a1: reported("working") });
      expect(useAgentActivityStore.getState().activity.a1).toBe("working");
      expect(notifyAgentWaiting).not.toHaveBeenCalled();
    });

    it("pings once per reported waiting episode, without an opened terminal", () => {
      seed([proc("a1", AGENT, RUNNING, "claude")]);
      const waiting = reported("waiting", { waitingFor: "permission" });

      tick({ a1: waiting });
      expect(details().a1).toMatchObject({
        activity: "waiting",
        waitingFor: "permission",
      });
      expect(notifyAgentWaiting).toHaveBeenCalledWith("claude", true);
      expect(processReportWaiting).toHaveBeenCalledWith("a1");

      tick({ a1: waiting });
      expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

      // Answered, then asking again: a new episode.
      tick({ a1: reported("working") });
      tick({ a1: waiting });
      expect(notifyAgentWaiting).toHaveBeenCalledTimes(2);
    });

    it("falls back to the heuristics when nothing is reported", () => {
      viewport.mockReturnValue("Continue? (y/n)");
      seed([proc("a1", AGENT, RUNNING), proc("a2", AGENT, RUNNING)]);
      tick({ a1: REDRAW, a2: WORKING });
      expect(details().a1).toEqual({
        activity: "waiting",
        reported: false,
        waitingFor: null,
        sinceMs: null,
        subagents: 0,
        shells: 0,
      });
      expect(details().a2).toMatchObject({
        activity: "working",
        sinceMs: null,
      });
    });

    it("reads a terminal reported working on an input prompt as waiting, once", () => {
      viewport.mockReturnValue("$ sudo systemctl restart api\nPassword:");
      seed([proc("t1", { kind: "terminal" }, RUNNING)]);
      tick({ t1: reported("working") });
      expect(details().t1).toMatchObject({ activity: "waiting" });
      expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
      tick({ t1: reported("working") });
      expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
    });

    it("keeps a terminal reported idle idle, even with a prompt on screen", () => {
      viewport.mockReturnValue("Continue? [y/N]");
      seed([proc("t1", { kind: "terminal" }, RUNNING)]);
      tick({ t1: reported("idle") });
      expect(details().t1.activity).toBe("idle");
      expect(notifyAgentWaiting).not.toHaveBeenCalled();
    });

    it("prefers the backend's counts over the on-screen shell count", () => {
      viewport.mockReturnValue("? for shortcuts · 2 shells");
      seed([proc("a1", AGENT, RUNNING), proc("a2", AGENT, RUNNING)]);
      tick({ a1: reported("idle", { subagents: 3, shells: 5 }) });
      expect(details().a1).toMatchObject({ subagents: 3, shells: 5 });
      // Nothing from the backend: the footer's count stands.
      expect(details().a2).toMatchObject({ subagents: 0, shells: 2 });
    });

    it("turns the state's age into a start time that holds between polls", () => {
      vi.useFakeTimers();
      try {
        vi.setSystemTime(1_000_000);
        seed([proc("a1", AGENT, RUNNING)]);
        tick({ a1: reported("working", { stateMs: 4000 }) });
        expect(details().a1.sinceMs).toBe(996_000);
        const first = details().a1;

        // The next poll lands 30 ms late: same start, same object.
        vi.setSystemTime(1_001_030);
        tick({ a1: reported("working", { stateMs: 5000 }) });
        expect(details().a1).toBe(first);

        // A new state starts its own clock.
        vi.setSystemTime(1_002_000);
        tick({ a1: reported("idle", { stateMs: 200 }) });
        expect(details().a1.sinceMs).toBe(1_001_800);

        // Same state, but the backend says it began again (a new turn).
        vi.setSystemTime(1_010_000);
        tick({ a1: reported("idle", { stateMs: 100 }) });
        expect(details().a1.sinceMs).toBe(1_009_900);
      } finally {
        vi.useRealTimers();
      }
    });

    it("drops a process that stopped", () => {
      seed([proc("a1", AGENT, RUNNING)]);
      tick({ a1: reported("working") });
      seed([]);
      tick();
      expect(details()).toEqual({});
    });
  });

  describe("viewing", () => {
    beforeEach(() => {
      seed([
        proc("a1", AGENT, RUNNING),
        proc("t1", { kind: "terminal" }, RUNNING),
      ]);
    });

    it("is the active agent while the window is focused", () => {
      vi.mocked(document.hasFocus).mockReturnValue(true);
      useProcessStore.setState({ activeProcessId: "a1" });
      expect(viewedAgentId()).toBe("a1");
    });

    it("is nothing while the window is not in front", () => {
      useProcessStore.setState({ activeProcessId: "a1" });
      expect(viewedAgentId()).toBeNull();
    });

    it("is nothing for a terminal, or with no active process", () => {
      vi.mocked(document.hasFocus).mockReturnValue(true);
      useProcessStore.setState({ activeProcessId: "t1" });
      expect(viewedAgentId()).toBeNull();
      useProcessStore.setState({ activeProcessId: null });
      expect(viewedAgentId()).toBeNull();
    });

    it.each([
      ["a to-do", { openTodo: { projectId: "p", todoId: "t" } }],
      [
        "a scratchpad",
        { openScratchpad: { projectId: "p", scratchpadId: "s" } },
      ],
      ["the Git board", { openBoard: { projectId: "p" } }],
    ])("is nothing while %s covers the work area", (_, layout) => {
      vi.mocked(document.hasFocus).mockReturnValue(true);
      useProcessStore.setState({ activeProcessId: "a1" });
      useLayoutStore.setState(layout);
      expect(viewedAgentId()).toBeNull();
    });

    it("is nothing while one of the assistant's sub-agents fills the work area", () => {
      // Sub-agents are never in the process store: not reported as viewed.
      vi.mocked(document.hasFocus).mockReturnValue(true);
      useLayoutStore.setState({ openSubagent: "s1" });
      expect(viewedAgentId()).toBeNull();
    });

    it("reports a change to the backend once, not every tick", async () => {
      vi.useFakeTimers();
      try {
        processActivity.mockResolvedValue({});
        const stop = startAgentActivityMonitor();

        // Nothing in view at the start: nothing to report.
        await vi.advanceTimersByTimeAsync(1000);
        expect(processReportViewing).not.toHaveBeenCalled();

        vi.mocked(document.hasFocus).mockReturnValue(true);
        useProcessStore.setState({ activeProcessId: "a1" });
        await vi.advanceTimersByTimeAsync(1000);
        await vi.advanceTimersByTimeAsync(1000);
        expect(processReportViewing).toHaveBeenCalledOnce();
        expect(processReportViewing).toHaveBeenCalledWith("a1");

        // Opening a to-do covers the terminal: no agent in view any more.
        useLayoutStore.setState({
          openTodo: { projectId: "p", todoId: "t" },
        });
        await vi.advanceTimersByTimeAsync(1000);
        expect(processReportViewing).toHaveBeenCalledTimes(2);
        expect(processReportViewing).toHaveBeenLastCalledWith(null);
        stop();
      } finally {
        vi.useRealTimers();
      }
    });

    it("reports a window focus change at once", async () => {
      vi.useFakeTimers();
      try {
        processActivity.mockResolvedValue({});
        const stop = startAgentActivityMonitor();
        vi.mocked(document.hasFocus).mockReturnValue(true);
        useProcessStore.setState({ activeProcessId: "a1" });

        window.dispatchEvent(new Event("focus"));
        expect(processReportViewing).toHaveBeenCalledWith("a1");

        vi.mocked(document.hasFocus).mockReturnValue(false);
        window.dispatchEvent(new Event("blur"));
        expect(processReportViewing).toHaveBeenLastCalledWith(null);
        stop();
      } finally {
        vi.useRealTimers();
      }
    });

    it("retries a report that failed", async () => {
      vi.useFakeTimers();
      try {
        processActivity.mockResolvedValue({});
        vi.mocked(document.hasFocus).mockReturnValue(true);
        useProcessStore.setState({ activeProcessId: "a1" });
        processReportViewing.mockRejectedValueOnce(new Error("bridge down"));
        const stop = startAgentActivityMonitor();

        await vi.advanceTimersByTimeAsync(1000);
        await vi.advanceTimersByTimeAsync(1000);
        expect(processReportViewing).toHaveBeenCalledTimes(2);
        expect(processReportViewing).toHaveBeenLastCalledWith("a1");
        stop();
      } finally {
        vi.useRealTimers();
      }
    });

    it("tells the backend nothing is in view when the monitor stops", async () => {
      vi.useFakeTimers();
      try {
        processActivity.mockResolvedValue({});
        vi.mocked(document.hasFocus).mockReturnValue(true);
        useProcessStore.setState({ activeProcessId: "a1" });
        const stop = startAgentActivityMonitor();
        await vi.advanceTimersByTimeAsync(1000);
        expect(processReportViewing).toHaveBeenLastCalledWith("a1");

        stop();
        expect(processReportViewing).toHaveBeenLastCalledWith(null);
      } finally {
        vi.useRealTimers();
      }
    });
  });
});
