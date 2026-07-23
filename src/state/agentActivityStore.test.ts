import { beforeEach, describe, expect, it, vi } from "vitest";

// The process store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

const lastOutput = vi.fn<(id: string) => number | null>();
const viewport = vi.fn<(id: string) => string | null>();
vi.mock("../lib/terminalRegistry", () => ({
  disposeTerminal: vi.fn(),
  getLastOutputAt: (id: string) => lastOutput(id),
  readViewportText: (id: string) => viewport(id),
}));

const notifyAgentWaiting = vi.fn();
vi.mock("../lib/notify", () => ({
  notifyAgentWaiting: (name: string) => notifyAgentWaiting(name),
}));

import type { ProcessInfo, ProcessKind, ProcessStatus } from "../ipc/types";
import { useProcessStore } from "./processStore";
import { useAgentActivityStore } from "./agentActivityStore";

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
  };
}

const RUNNING: ProcessStatus = { state: "running", pid: 1, since: "now" };
const AGENT: ProcessKind = { kind: "agent", adapter: "claude-code" };

function seed(processes: ProcessInfo[]) {
  useProcessStore.setState({ processes });
}

function tick() {
  useAgentActivityStore.getState().tick();
}

describe("agentActivityStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAgentActivityStore.setState({ activity: {}, notified: {} });
    useProcessStore.setState({ activeProcessId: null });
    lastOutput.mockReturnValue(null);
    viewport.mockReturnValue(null);
    // Default: the user is not looking at any agent, so waiting agents ping.
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
  });

  it("marks recent output as working", () => {
    lastOutput.mockReturnValue(Date.now());
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("working");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("marks a quiet prompt screen as waiting and notifies once", () => {
    lastOutput.mockReturnValue(Date.now() - 10_000);
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    seed([proc("a1", AGENT, RUNNING, "claude")]);

    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");
    expect(notifyAgentWaiting).toHaveBeenCalledOnce();
    expect(notifyAgentWaiting).toHaveBeenCalledWith("claude");

    // Still on the same prompt next poll — no repeat alert.
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
  });

  it("marks a quiet non-prompt screen as idle", () => {
    lastOutput.mockReturnValue(null);
    viewport.mockReturnValue("Compiled successfully.");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("idle");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("ignores non-agent and non-running processes", () => {
    seed([
      proc("svc", { kind: "service" }, RUNNING),
      proc("a2", AGENT, { state: "notStarted" }),
    ]);
    tick();
    expect(useAgentActivityStore.getState().activity).toEqual({});
  });

  it("drops agents that are no longer running", () => {
    viewport.mockReturnValue("(y/n)");
    lastOutput.mockReturnValue(Date.now() - 10_000);
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");

    seed([
      proc("a1", AGENT, {
        state: "exited",
        code: 0,
        crashed: false,
        at: "now",
      }),
    ]);
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBeUndefined();
  });

  it("does not re-alert on working↔waiting flicker while unattended", () => {
    lastOutput.mockReturnValue(Date.now() - 10_000);
    viewport.mockReturnValue("Continue? (y/n)");
    seed([proc("a1", AGENT, RUNNING)]);
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);

    // A live prompt repaints (spinner/cursor), briefly reading as "working",
    // then reads as waiting again — the user must not be pinged again.
    lastOutput.mockReturnValue(Date.now());
    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("working");

    lastOutput.mockReturnValue(Date.now() - 10_000);
    tick();
    expect(notifyAgentWaiting).toHaveBeenCalledTimes(1);
  });

  it("does not ping while the user is viewing the agent", () => {
    lastOutput.mockReturnValue(Date.now() - 10_000);
    viewport.mockReturnValue("Do you want to proceed?\n❯ 1. Yes");
    vi.mocked(document.hasFocus).mockReturnValue(true);
    useProcessStore.setState({ activeProcessId: "a1" });
    seed([proc("a1", AGENT, RUNNING)]);

    tick();
    expect(useAgentActivityStore.getState().activity.a1).toBe("waiting");
    expect(notifyAgentWaiting).not.toHaveBeenCalled();
  });

  it("does not re-ping after the user viewed the prompt and switches away", () => {
    lastOutput.mockReturnValue(Date.now() - 10_000);
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
    lastOutput.mockReturnValue(Date.now() - 10_000);
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
    lastOutput.mockReturnValue(Date.now() - 10_000);
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
});
