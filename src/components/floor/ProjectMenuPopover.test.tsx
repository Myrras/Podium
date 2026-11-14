import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProcessInfo, ProjectInfo, WorktreeInfo } from "../../ipc/types";

const m = vi.hoisted(() => ({
  worktreeRemove: vi.fn(),
  newTerminal: vi.fn(),
  rename: vi.fn(),
  color: vi.fn(),
  close: vi.fn(),
  reload: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../ipc/commands", () => ({
  worktreeRemove: m.worktreeRemove,
  toIpcError: (e: { kind?: string; message?: string }) => ({
    kind: e.kind ?? "other",
    message: e.message ?? "x",
  }),
}));
vi.mock("../../lib/terminals", () => ({ newTerminal: m.newTerminal }));
vi.mock("../../state/toastStore", () => ({ toastError: vi.fn() }));

import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useWorktreeStore } from "../../state/worktreeStore";
import { ProjectMenuPopover } from "./ProjectMenuPopover";

const project = {
  id: "p1",
  name: "Acme",
  color: "#112233",
  nativeInWslMode: false,
  sections: {
    todos: true,
    terminals: true,
    worktrees: true,
    scratchpads: true,
    issues: true,
  },
} as unknown as ProjectInfo;
const free: WorktreeInfo = {
  name: "wt-a",
  path: "/x/a",
  branch: "b",
  inUse: false,
};
const used: WorktreeInfo = {
  name: "wt-b",
  path: "/x/b",
  branch: "b",
  inUse: true,
};
const agent = {
  id: "a1",
  projectId: "p1",
  name: "Bot One",
  kind: { kind: "agent", adapter: "claude" },
  worktree: "wt-b",
} as unknown as ProcessInfo;

function setup(itemId: string | null = null, p = project) {
  useProjectStore.setState(
    {
      projects: [p],
      renameProject: m.rename,
      setProjectColor: m.color,
      closeProject: m.close,
      reloadProjectConfig: m.reload,
    } as never,
    false,
  );
  useWorktreeStore.setState(
    { worktreesByProject: { p1: [free, used] } },
    false,
  );
  useProcessStore.setState(
    { processes: [agent], activeProcessId: null },
    false,
  );
  useFloorStore.setState({
    popover: { projectId: "p1", kind: "menu", itemId },
  });
  const anchor = document.createElement("button");
  document.body.append(anchor);
  return render(
    <ProjectMenuPopover projectId="p1" itemId={itemId} anchor={anchor} />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  m.rename.mockResolvedValue(undefined);
  m.color.mockResolvedValue(undefined);
  m.close.mockResolvedValue(undefined);
});

describe("ProjectMenuPopover", () => {
  it("renames, blank becomes null", () => {
    setup();
    const input = screen.getByLabelText("Project name");
    fireEvent.change(input, { target: { value: "Beta" } });
    fireEvent.blur(input);
    expect(m.rename).toHaveBeenCalledWith("p1", "Beta");
    fireEvent.change(input, { target: { value: "  " } });
    fireEvent.blur(input);
    expect(m.rename).toHaveBeenLastCalledWith("p1", null);
  });

  it("saves an unblurred name when Esc closes the popover", () => {
    const { unmount } = setup();
    const input = screen.getByLabelText("Project name");
    fireEvent.change(input, { target: { value: "Gamma" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(useFloorStore.getState().popover).toBeNull();
    unmount();
    expect(m.rename).toHaveBeenCalledTimes(1);
    expect(m.rename).toHaveBeenCalledWith("p1", "Gamma");
  });

  it("does not rename on unmount when the name is unchanged or already saved", () => {
    const { unmount } = setup();
    const input = screen.getByLabelText("Project name");
    fireEvent.change(input, { target: { value: "Beta" } });
    fireEvent.blur(input);
    unmount();
    expect(m.rename).toHaveBeenCalledTimes(1);
  });

  it("commits the colour on the native change event only", () => {
    setup();
    const el = screen.getByLabelText("Project colour") as HTMLInputElement;
    fireEvent.input(el, { target: { value: "#445566" } });
    expect(m.color).not.toHaveBeenCalled();
    fireEvent.change(el, { target: { value: "#445566" } });
    expect(m.color).toHaveBeenCalledWith("p1", "#445566");
    fireEvent.click(screen.getByText("No colour"));
    expect(m.color).toHaveBeenLastCalledWith("p1", null);
  });

  it("shows the current colour after No colour and commits picking the old one again", () => {
    setup();
    const swatch = () =>
      screen.getByLabelText("Project colour") as HTMLInputElement;
    expect(swatch().value).toBe("#112233");
    fireEvent.click(screen.getByText("No colour"));
    act(() =>
      useProjectStore.setState({ projects: [{ ...project, color: null }] }),
    );
    // jsdom has no --accent: the theme accent resolves to black.
    expect(swatch().value).toBe("#000000");
    fireEvent.change(swatch(), { target: { value: "#112233" } });
    expect(m.color).toHaveBeenLastCalledWith("p1", "#112233");
  });

  it("lists worktrees as Free or with the agent name and opens one", () => {
    setup();
    expect(screen.getByText("Free")).toBeTruthy();
    expect(screen.getByText("Bot One")).toBeTruthy();
    fireEvent.click(screen.getByText("wt-a"));
    expect(useFloorStore.getState().popover?.itemId).toBe("wt-a");
  });

  it("hides worktrees and board by section", () => {
    setup(null, {
      ...project,
      sections: { ...project.sections, worktrees: false, issues: false },
    });
    expect(screen.queryByText("wt-a")).toBeNull();
    expect(screen.queryByText("Git board")).toBeNull();
  });

  it("has no New terminal item", () => {
    setup();
    expect(screen.queryByText("New terminal")).toBeNull();
  });

  it("runs terminal, board, reload", () => {
    const open = vi.fn();
    useLayoutStore.setState({ openBoardInWorkArea: open } as never);
    setup();
    fireEvent.click(screen.getByText("Reload podium.yml"));
    expect(m.reload).toHaveBeenCalledWith("p1");
    fireEvent.click(screen.getByText("Git board"));
    expect(open).toHaveBeenCalledWith("p1");
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("removes the project only after the inline confirm", () => {
    setup();
    fireEvent.click(screen.getByText("Remove project"));
    expect(
      screen.getByText("Remove Acme from Podium? The folder stays on disk."),
    ).toBeTruthy();
    fireEvent.click(screen.getByText("Keep"));
    expect(m.close).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Remove project"));
    fireEvent.click(screen.getByText("Remove"));
    expect(m.close).toHaveBeenCalledWith("p1");
  });

  it("worktree view: free worktree can be removed, terminal opens", async () => {
    m.worktreeRemove.mockResolvedValue([used]);
    setup("wt-a");
    expect(screen.queryByText("Open the agent")).toBeNull();
    fireEvent.click(screen.getByText("Open a terminal here"));
    expect(m.newTerminal).toHaveBeenCalledWith(project, free);
    await act(async () => {
      fireEvent.click(screen.getByText("Remove worktree"));
    });
    expect(m.worktreeRemove).toHaveBeenCalledWith("p1", "wt-a", false);
    expect(useWorktreeStore.getState().worktreesByProject.p1).toEqual([used]);
  });

  it("worktree view: dirty asks inline and retries with force", async () => {
    m.worktreeRemove
      .mockRejectedValueOnce({ kind: "worktreeDirty", message: "dirty" })
      .mockResolvedValueOnce([]);
    setup("wt-a");
    await act(async () => {
      fireEvent.click(screen.getByText("Remove worktree"));
    });
    expect(screen.getByText(/uncommitted changes/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByText("Discard and remove"));
    });
    expect(m.worktreeRemove).toHaveBeenLastCalledWith("p1", "wt-a", true);
  });

  it("worktree view: in-use worktree has the agent and no remove", () => {
    setup("wt-b");
    expect(screen.queryByText("Remove worktree")).toBeNull();
    fireEvent.click(screen.getByText("Back to the project menu"));
    expect(useFloorStore.getState().popover?.itemId).toBeNull();
    fireEvent.click(screen.getByText("Open the agent"));
    expect(useFloorStore.getState().terminalId).toBe("a1");
    expect(useProcessStore.getState().activeProcessId).toBe("a1");
  });
});
