import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The subsection talks to the IPC command layer (via processStore and
// worktreeStore); mock it per file.
vi.mock("../ipc/commands", () => ({
  worktreeList: vi.fn(),
  worktreeRemove: vi.fn(),
  worktreeOpenInWarp: vi.fn(),
  processAdd: vi.fn(),
  processStart: vi.fn(),
  toIpcError: (e: unknown) => {
    if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
      return e as { kind: string; message: string };
    }
    return { kind: "io", message: e instanceof Error ? e.message : String(e) };
  },
}));

vi.mock("../state/toastStore", () => ({ toastError: vi.fn() }));

import type { ProcessInfo, WorktreeInfo } from "../ipc/types";
import {
  processAdd,
  processStart,
  worktreeList,
  worktreeOpenInWarp,
  worktreeRemove,
} from "../ipc/commands";
import { toastError } from "../state/toastStore";
import { useProcessStore } from "../state/processStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { WorktreeSubsection } from "./WorktreeSubsection";

const worktreeListMock = vi.mocked(worktreeList);
const worktreeRemoveMock = vi.mocked(worktreeRemove);
const processAddMock = vi.mocked(processAdd);
const processStartMock = vi.mocked(processStart);

const PROJECT = "proj-1";
const initialProcess = useProcessStore.getState();

function wt(name: string): WorktreeInfo {
  return {
    name,
    path: `/repo/.podium/worktrees/${name}`,
    branch: `podium/${name}`,
    inUse: false,
  };
}

function terminal(
  id: string,
  name: string,
  worktree: string | null,
): ProcessInfo {
  return {
    id,
    projectId: PROJECT,
    name,
    kind: { kind: "terminal" },
    status: { state: "running", pid: 1, since: "2026-08-20T10:00:00Z" },
    restartPolicy: "never",
    command: "zsh",
    worktree,
    color: null,
  };
}

function renderSubsection(
  terminals: ProcessInfo[] = [],
  onOpenCommands = vi.fn(),
) {
  render(
    <WorktreeSubsection
      projectId={PROJECT}
      projectRoot="/fake/root"
      terminals={terminals}
      onOpenCommands={onOpenCommands}
    />,
  );
  return { onOpenCommands };
}

describe("WorktreeSubsection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useWorktreeStore.setState({ worktreesByProject: {} }, false);
    useProcessStore.setState(initialProcess, true);
  });

  it("renders nothing when the project has no worktrees", async () => {
    worktreeListMock.mockResolvedValue([]);
    const { container } = render(
      <WorktreeSubsection
        projectId={PROJECT}
        projectRoot="/fake/root"
        terminals={[]}
        onOpenCommands={vi.fn()}
      />,
    );
    await waitFor(() => expect(worktreeListMock).toHaveBeenCalledWith(PROJECT));
    expect(container).toBeEmptyDOMElement();
  });

  it("opens a terminal rooted in the worktree", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login")]);
    processAddMock.mockResolvedValue({
      ...terminal("t1", "Terminal (fix-login)", "fix-login"),
    });
    processStartMock.mockResolvedValue();
    renderSubsection();

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Open terminal in fix-login",
      }),
    );

    await waitFor(() =>
      expect(processAddMock).toHaveBeenCalledWith(PROJECT, {
        name: "Terminal (fix-login)",
        kind: "terminal",
        cwd: ".podium/worktrees/fix-login",
      }),
    );
    expect(processStartMock).toHaveBeenCalledWith("t1");
  });

  it("opens the worktree in Warp", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login")]);
    vi.mocked(worktreeOpenInWarp).mockResolvedValue();
    renderSubsection();

    fireEvent.click(
      await screen.findByRole("button", { name: "Open fix-login in Warp" }),
    );

    await waitFor(() =>
      expect(worktreeOpenInWarp).toHaveBeenCalledWith(PROJECT, "fix-login"),
    );
  });

  it("opens the Commands modal targeted at the worktree", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login")]);
    const { onOpenCommands } = renderSubsection();

    fireEvent.click(
      await screen.findByRole("button", { name: "Commands for fix-login" }),
    );

    expect(onOpenCommands).toHaveBeenCalledWith(wt("fix-login"));
  });

  it("removes a clean worktree", async () => {
    worktreeListMock.mockResolvedValue([wt("done")]);
    worktreeRemoveMock.mockResolvedValue([]);
    renderSubsection();

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete worktree done" }),
    );

    await waitFor(() =>
      expect(worktreeRemoveMock).toHaveBeenCalledWith(PROJECT, "done", false),
    );
    await waitFor(() => expect(screen.queryByText("done")).toBeNull());
  });

  it("confirms before force-removing a dirty worktree", async () => {
    worktreeListMock.mockResolvedValue([wt("dirty")]);
    worktreeRemoveMock
      .mockRejectedValueOnce({ kind: "worktreeDirty", message: "dirty" })
      .mockResolvedValueOnce([]);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderSubsection();

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
    renderSubsection();

    fireEvent.click(
      await screen.findByRole("button", { name: "Delete worktree dirty" }),
    );

    await waitFor(() => expect(worktreeRemoveMock).toHaveBeenCalledTimes(1));
    expect(screen.getByText("dirty")).toBeInTheDocument();
    confirmSpy.mockRestore();
  });

  it("disables delete while a live process runs in the worktree, derived from the process store", async () => {
    worktreeListMock.mockResolvedValue([wt("busy")]);
    useProcessStore.setState({
      processes: [terminal("t1", "shell", "busy")],
    });
    renderSubsection();

    const btn = await screen.findByRole("button", {
      name: "Delete worktree busy",
    });
    expect(btn).toBeDisabled();
  });

  it("nests a worktree's terminals under its row", async () => {
    worktreeListMock.mockResolvedValue([wt("fix-login")]);
    renderSubsection([
      terminal("t1", "nested-shell", "fix-login"),
      terminal("t2", "root-shell", null),
    ]);

    await screen.findByText("fix-login");
    expect(screen.getByText("nested-shell")).toBeInTheDocument();
    expect(screen.queryByText("root-shell")).not.toBeInTheDocument();
  });
});
