import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectInfo, WorktreeInfo } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { newTerminal } from "./terminals";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const project = { id: "p1", nativeInWslMode: false } as ProjectInfo;
const addProcess = vi.fn();
const startProcess = vi.fn();
const setActiveProject = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  addProcess.mockResolvedValue({ id: "t9" });
  useProcessStore.setState({ processes: [], addProcess, startProcess }, false);
  useProjectStore.setState({ setActiveProject }, false);
});

describe("newTerminal", () => {
  it("adds, starts and activates a numbered project terminal", async () => {
    await newTerminal(project);
    expect(setActiveProject).toHaveBeenCalledWith("p1");
    expect(addProcess).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({ name: "Terminal 1", kind: "terminal" }),
    );
    expect(startProcess).toHaveBeenCalledWith("t9");
  });

  it("roots a worktree terminal in the worktree", async () => {
    await newTerminal(project, { name: "fix" } as WorktreeInfo);
    expect(addProcess).toHaveBeenCalledWith(
      "p1",
      expect.objectContaining({
        name: "Terminal (fix)",
        cwd: ".podium/worktrees/fix",
      }),
    );
  });
});
