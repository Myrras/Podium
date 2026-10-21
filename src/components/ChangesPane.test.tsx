import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({
  changesList: vi.fn(),
  changesFileDiff: vi.fn(),
  toIpcError: (e: unknown) => e,
}));

import type {
  ChangeSet,
  ProcessInfo,
  ProjectInfo,
  WorktreeInfo,
} from "../ipc/types";
import { changesFileDiff, changesList } from "../ipc/commands";
import { useChangesStore } from "../state/changesStore";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { ChangesPane } from "./ChangesPane";

const listMock = vi.mocked(changesList);
const diffMock = vi.mocked(changesFileDiff);

const initialChanges = useChangesStore.getState();
const initialProcess = useProcessStore.getState();
const initialProject = useProjectStore.getState();
const initialWorktree = useWorktreeStore.getState();
const initialLayout = useLayoutStore.getState();

function project(id: string, name: string): ProjectInfo {
  return {
    id,
    name,
    root: `/fake/${name.toLowerCase()}`,
    iconInitials: name.slice(0, 2).toUpperCase(),
    configError: null,
    renamed: false,
    sections: {
      todos: true,
      terminals: true,
      worktrees: true,
      scratchpads: true,
      issues: false,
    },
    color: null,
    nativeInWslMode: false,
  };
}

function agent(projectId: string, worktree: string | null): ProcessInfo {
  return {
    id: "agent-1",
    projectId,
    name: "agent",
    kind: { kind: "agent", adapter: "claude-code" },
    status: { state: "running", pid: 1, since: "2026-08-20T10:00:00Z" },
    restartPolicy: "never",
    command: "claude",
    worktree,
    color: null,
    viewing: false,
  };
}

function wt(name: string): WorktreeInfo {
  return {
    name,
    path: `/fake/webshop/.podium/worktrees/${name}`,
    branch: `podium/${name}`,
    inUse: false,
  };
}

const SET: ChangeSet = {
  branch: "podium/fix-cart",
  base: "uncommitted",
  baseRef: null,
  baseCommit: "abc12345",
  files: [
    {
      path: "src/cart.ts",
      oldPath: null,
      status: "modified",
      additions: 2,
      deletions: 1,
      binary: false,
    },
    {
      path: "notes.md",
      oldPath: null,
      status: "untracked",
      additions: 3,
      deletions: 0,
      binary: false,
    },
  ],
  truncated: false,
};

describe("ChangesPane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useChangesStore.setState(initialChanges, true);
    useProcessStore.setState(initialProcess, true);
    useProjectStore.setState(initialProject, true);
    useWorktreeStore.setState(initialWorktree, true);
    useLayoutStore.setState(initialLayout, true);
    useProjectStore.setState({
      projects: [project("proj-1", "Webshop"), project("proj-2", "Billing")],
    });
    useWorktreeStore.setState({
      worktreesByProject: { "proj-1": [wt("fix-cart")] },
    });
    listMock.mockResolvedValue(SET);
  });

  it("follows the focused agent's worktree and lists its changes", async () => {
    useProcessStore.setState({
      processes: [agent("proj-1", "fix-cart")],
      activeProcessId: "agent-1",
    });

    render(<ChangesPane />);

    await screen.findByText("cart.ts");
    expect(listMock).toHaveBeenCalledWith("proj-1", "fix-cart", "uncommitted");
    expect(screen.getByText("2 files")).toBeInTheDocument();
    expect(screen.getByText(/vs HEAD \(abc12345\)/)).toBeInTheDocument();
  });

  it("falls back to the first project's root with nothing focused", async () => {
    render(<ChangesPane />);
    await screen.findByText("cart.ts");
    expect(listMock).toHaveBeenCalledWith("proj-1", null, "uncommitted");
  });

  it("switches to the branch base", async () => {
    render(<ChangesPane />);
    await screen.findByText("cart.ts");

    listMock.mockResolvedValue({
      ...SET,
      base: "branch",
      baseRef: "origin/main",
    });
    fireEvent.click(screen.getByRole("button", { name: "Branch" }));

    await screen.findByText(/vs origin\/main/);
    expect(listMock).toHaveBeenLastCalledWith("proj-1", null, "branch");
  });

  it("pins a checkout picked from the dropdown", async () => {
    render(<ChangesPane />);
    await screen.findByText("cart.ts");

    fireEvent.change(screen.getByLabelText("Checkout"), {
      target: { value: "proj-2|" },
    });

    await waitFor(() =>
      expect(listMock).toHaveBeenLastCalledWith("proj-2", null, "uncommitted"),
    );
    expect(useChangesStore.getState().pinned).toEqual({
      projectId: "proj-2",
      worktree: null,
    });
  });

  it("expands a file into its diff", async () => {
    diffMock.mockResolvedValue({
      path: "src/cart.ts",
      diff: "@@ -1,2 +1,3 @@\n keep\n-old total\n+new total\n+extra\n",
      binary: false,
      truncated: false,
    });
    render(<ChangesPane />);

    fireEvent.click(await screen.findByText("cart.ts"));

    await screen.findByText("new total");
    expect(screen.getByText("old total")).toBeInTheDocument();
    expect(diffMock).toHaveBeenCalledWith(
      "proj-1",
      null,
      "uncommitted",
      "src/cart.ts",
      null,
    );
  });

  it("shows a readable error outside a git repository", async () => {
    listMock.mockRejectedValue({ kind: "notAGitRepo", message: "x" });
    render(<ChangesPane />);
    await screen.findByText("Not a git repository.");
  });

  it("closes through the layout store", async () => {
    useLayoutStore.setState({ changesOpen: true });
    render(<ChangesPane />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Close changes" }));
    });
    expect(useLayoutStore.getState().changesOpen).toBe(false);
  });
});
