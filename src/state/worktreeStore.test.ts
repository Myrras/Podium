import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC command layer so the store talks to fixtures, not Tauri.
vi.mock("../ipc/commands", () => ({
  worktreeList: vi.fn(() => Promise.resolve([])),
}));

import type { WorktreeInfo } from "../ipc/types";
import { worktreeList } from "../ipc/commands";
import { useWorktreeStore } from "./worktreeStore";

const worktreeListMock = vi.mocked(worktreeList);

function wt(name: string): WorktreeInfo {
  return {
    name,
    path: `/repo/.podium/worktrees/${name}`,
    branch: `podium/${name}`,
    inUse: false,
  };
}

describe("worktreeStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorktreeStore.setState({ worktreesByProject: {} }, false);
  });

  it("refresh fills the cache for a project", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login")]);

    await useWorktreeStore.getState().refresh("proj-a");

    expect(worktreeListMock).toHaveBeenCalledWith("proj-a");
    expect(useWorktreeStore.getState().worktreesByProject["proj-a"]).toEqual([
      wt("fix-login"),
    ]);
  });

  it("refresh caches an empty list silently on failure", async () => {
    // Background refreshes run on every project open — a non-git project
    // must not toast, and the modal must get an empty state, not "Loading…".
    worktreeListMock.mockRejectedValue({ kind: "notAGitRepo", message: "no" });

    await useWorktreeStore.getState().refresh("proj-a");

    expect(useWorktreeStore.getState().worktreesByProject["proj-a"]).toEqual(
      [],
    );
  });

  it("setWorktrees replaces one project's cache without touching others", () => {
    useWorktreeStore.setState({
      worktreesByProject: { "proj-a": [wt("mine")], "proj-b": [wt("theirs")] },
    });

    useWorktreeStore.getState().setWorktrees("proj-a", []);

    expect(useWorktreeStore.getState().worktreesByProject).toEqual({
      "proj-a": [],
      "proj-b": [wt("theirs")],
    });
  });

  it("dropProject removes only that project's cache", () => {
    useWorktreeStore.setState({
      worktreesByProject: { "proj-a": [wt("mine")], "proj-b": [wt("theirs")] },
    });

    useWorktreeStore.getState().dropProject("proj-a");

    expect(useWorktreeStore.getState().worktreesByProject).toEqual({
      "proj-b": [wt("theirs")],
    });
  });
});
