import { beforeEach, describe, expect, it, vi } from "vitest";

import type { WorktreeInfo } from "../ipc/types";

const m = vi.hoisted(() => ({ remove: vi.fn(), toast: vi.fn() }));
vi.mock("../ipc/commands", () => ({
  worktreeRemove: m.remove,
  toIpcError: (e: { kind?: string; message?: string }) => ({
    kind: e.kind ?? "other",
    message: e.message ?? "x",
  }),
}));
vi.mock("../state/toastStore", () => ({ toastError: m.toast }));

import { useWorktreeStore } from "../state/worktreeStore";
import { dirtyWorktreeQuestion, removeWorktree } from "./worktrees";

const left: WorktreeInfo = {
  name: "wt-b",
  path: "/x/b",
  branch: "b",
  inUse: false,
};
const dirty = { kind: "worktreeDirty", message: "dirty" };

beforeEach(() => {
  vi.clearAllMocks();
  useWorktreeStore.setState({ worktreesByProject: {} });
});

describe("removeWorktree", () => {
  it("removes and stores the fresh list without asking", async () => {
    m.remove.mockResolvedValueOnce([left]);
    const ask = vi.fn();
    expect(await removeWorktree("p1", "wt-a", ask)).toBe(true);
    expect(m.remove).toHaveBeenCalledWith("p1", "wt-a", false);
    expect(ask).not.toHaveBeenCalled();
    expect(useWorktreeStore.getState().worktreesByProject.p1).toEqual([left]);
  });

  it("asks on a dirty worktree and retries with force on yes", async () => {
    m.remove.mockRejectedValueOnce(dirty).mockResolvedValueOnce([]);
    const ask = vi.fn(() => Promise.resolve(true));
    expect(await removeWorktree("p1", "wt-a", ask)).toBe(true);
    expect(ask).toHaveBeenCalledWith("wt-a");
    expect(m.remove).toHaveBeenLastCalledWith("p1", "wt-a", true);
  });

  it("keeps a dirty worktree on no", async () => {
    m.remove.mockRejectedValueOnce(dirty);
    expect(
      await removeWorktree("p1", "wt-a", () => Promise.resolve(false)),
    ).toBe(false);
    expect(m.remove).toHaveBeenCalledTimes(1);
    expect(m.toast).not.toHaveBeenCalled();
  });

  it("toasts other errors, also from the forced retry", async () => {
    m.remove.mockRejectedValueOnce({ message: "locked" });
    expect(await removeWorktree("p1", "wt-a", vi.fn())).toBe(false);
    expect(m.toast).toHaveBeenLastCalledWith(
      "Could not remove worktree",
      "locked",
    );
    m.remove
      .mockRejectedValueOnce(dirty)
      .mockRejectedValueOnce({ message: "busy" });
    expect(
      await removeWorktree("p1", "wt-a", () => Promise.resolve(true)),
    ).toBe(false);
    expect(m.toast).toHaveBeenLastCalledWith(
      "Could not remove worktree",
      "busy",
    );
  });

  it("phrases the dirty question", () => {
    expect(dirtyWorktreeQuestion("wt-a")).toBe(
      'Worktree "wt-a" has uncommitted changes. Discard them and remove it anyway?',
    );
  });
});
