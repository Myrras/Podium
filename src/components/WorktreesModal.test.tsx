import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The modal talks to the IPC command layer directly; mock it per file.
vi.mock("../ipc/commands", () => ({
  worktreeList: vi.fn(),
  worktreeRemove: vi.fn(),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "io", message: e instanceof Error ? e.message : String(e) };
  },
}));

vi.mock("../state/toastStore", () => ({ toastError: vi.fn() }));

import type { WorktreeInfo } from "../ipc/types";
import { worktreeList, worktreeRemove } from "../ipc/commands";
import { toastError } from "../state/toastStore";
import { WorktreesModal } from "./WorktreesModal";

const worktreeListMock = vi.mocked(worktreeList);
const worktreeRemoveMock = vi.mocked(worktreeRemove);

const PROJECT = "proj-1";

function wt(name: string, inUse = false): WorktreeInfo {
  return {
    name,
    path: `/repo/.podium/worktrees/${name}`,
    branch: `podium/${name}`,
    inUse,
  };
}

describe("WorktreesModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("lists worktrees fetched on open", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login"), wt("busy", true)]);
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    expect(await screen.findByText("fix-login")).toBeInTheDocument();
    expect(screen.getByText("podium/fix-login")).toBeInTheDocument();
    expect(screen.getByText("in use")).toBeInTheDocument();
    expect(worktreeListMock).toHaveBeenCalledWith(PROJECT);
  });

  it("disables delete for a worktree in use", async () => {
    worktreeListMock.mockResolvedValue([wt("busy", true)]);
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    const btn = await screen.findByRole("button", {
      name: "Delete worktree busy",
    });
    expect(btn).toBeDisabled();
  });

  it("deletes a clean worktree and replaces the list", async () => {
    worktreeListMock.mockResolvedValue([wt("done")]);
    worktreeRemoveMock.mockResolvedValue([]);
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete worktree done" }),
    );

    await waitFor(() =>
      expect(worktreeRemoveMock).toHaveBeenCalledWith(PROJECT, "done", false),
    );
    await waitFor(() => expect(screen.queryByText("done")).toBeNull());
  });

  it("disables the delete button while removal is in flight", async () => {
    worktreeListMock.mockResolvedValue([wt("slow")]);
    // Hold the removal open so we can observe the in-flight state.
    let resolve!: (v: WorktreeInfo[]) => void;
    worktreeRemoveMock.mockReturnValue(
      new Promise<WorktreeInfo[]>((r) => {
        resolve = r;
      }),
    );
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    const btn = await screen.findByRole("button", {
      name: "Delete worktree slow",
    });
    fireEvent.click(btn);

    // Disabled + busy while the remove promise is pending.
    await waitFor(() => expect(btn).toBeDisabled());
    expect(btn.getAttribute("title")).toBe("Removing…");
    // A second click cannot fire another removal.
    fireEvent.click(btn);
    expect(worktreeRemoveMock).toHaveBeenCalledTimes(1);

    resolve([]);
    await waitFor(() => expect(screen.queryByText("slow")).toBeNull());
  });

  it("confirms before force-removing a dirty worktree", async () => {
    worktreeListMock.mockResolvedValue([wt("dirty")]);
    worktreeRemoveMock
      .mockRejectedValueOnce({ kind: "worktreeDirty", message: "dirty" })
      .mockResolvedValueOnce([]);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete worktree dirty" }),
    );

    await waitFor(() =>
      expect(worktreeRemoveMock).toHaveBeenCalledWith(PROJECT, "dirty", true),
    );
    expect(confirmSpy).toHaveBeenCalled();
    expect(toastError).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it("keeps a dirty worktree when the confirm is declined", async () => {
    worktreeListMock.mockResolvedValue([wt("dirty")]);
    worktreeRemoveMock.mockRejectedValueOnce({
      kind: "worktreeDirty",
      message: "dirty",
    });
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<WorktreesModal open projectId={PROJECT} onClose={() => {}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete worktree dirty" }),
    );

    await waitFor(() => expect(worktreeRemoveMock).toHaveBeenCalledTimes(1));
    expect(screen.getByText("dirty")).toBeInTheDocument();
    confirmSpy.mockRestore();
  });
});
