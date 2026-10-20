import { beforeEach, describe, expect, it, vi } from "vitest";

// The store reaches Tauri only through these; neither is exercised here.
vi.mock("../ipc/commands", () => ({ toIpcError: vi.fn() }));
vi.mock("../lib/terminalRegistry", () => ({ disposeTerminal: vi.fn() }));

import type { ProcessInfo } from "../ipc/types";
import { disposeTerminal } from "../lib/terminalRegistry";
import { useProcessStore } from "./processStore";

function agent(id: string): ProcessInfo {
  return {
    id,
    projectId: "proj-a",
    name: "fix-login",
    kind: { kind: "agent", adapter: "claude-code" },
    status: {
      state: "exited",
      code: 0,
      crashed: false,
      stopped: false,
      at: "2026-01-01T00:00:00Z",
    },
    restartPolicy: "never",
    command: "claude",
    worktree: null,
    color: null,
  };
}

describe("processStore.applyRemoved", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProcessStore.setState(
      { processes: [agent("a1"), agent("a2")], activeProcessId: "a1" },
      false,
    );
  });

  // An exited agent the cleanup sweep removes may be the one open in the
  // work area; the work area then falls back to its empty state.
  it("clears the focus and disposes the terminal of the removed process", () => {
    useProcessStore.getState().applyRemoved("a1");

    const state = useProcessStore.getState();
    expect(state.activeProcessId).toBeNull();
    expect(state.processes.map((p) => p.id)).toEqual(["a2"]);
    expect(disposeTerminal).toHaveBeenCalledWith("a1");
  });

  it("keeps the focus when another process is removed", () => {
    useProcessStore.getState().applyRemoved("a2");

    expect(useProcessStore.getState().activeProcessId).toBe("a1");
  });
});
