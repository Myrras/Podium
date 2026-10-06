import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import { invoke } from "@tauri-apps/api/core";

import type { ProcessInfo, ProjectInfo, WorktreeInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useWorktreeStore } from "../state/worktreeStore";
import { Sidebar } from "./Sidebar";

const initialProject = useProjectStore.getState();
const initialLayout = useLayoutStore.getState();
const initialProcess = useProcessStore.getState();
const initialWorktree = useWorktreeStore.getState();

function project(overrides: Partial<ProjectInfo> = {}): ProjectInfo {
  return {
    id: "proj-1",
    name: "Webshop",
    root: "/fake/webshop",
    iconInitials: "WS",
    configError: null,
    renamed: false,
    color: null,
    ...overrides,
  };
}

function terminal(
  id: string,
  name: string,
  worktree: string | null,
): ProcessInfo {
  return {
    id,
    projectId: "proj-1",
    name,
    kind: { kind: "terminal" },
    status: { state: "running", pid: 1, since: "2026-08-20T10:00:00Z" },
    restartPolicy: "never",
    command: "zsh",
    worktree,
    color: null,
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

describe("Sidebar project header", () => {
  beforeEach(() => {
    useProjectStore.setState(initialProject, true);
    useLayoutStore.setState(initialLayout, true);
    useProcessStore.setState(initialProcess, true);
    useWorktreeStore.setState(initialWorktree, true);
    vi.mocked(invoke)
      .mockReset()
      .mockImplementation(() => Promise.resolve([]));
  });

  it("keeps the drag handle draggable in the DOM regardless of hover state", () => {
    // jsdom never evaluates :hover, so this asserts the element the CSS
    // reveals on hover is present and draggable at rest — the actual
    // hover-triggered visibility is CSS-only and isn't asserted here.
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    const handle = screen.getByRole("img", {
      name: "Drag to reorder Webshop",
    });
    expect(handle).toHaveAttribute("draggable", "true");
  });

  it("recolours the badge while picking and persists only on commit", () => {
    const setProjectColor = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [project({ color: "#3e63dd" })],
      setProjectColor,
    });
    render(<Sidebar />);

    const picker = screen.getByLabelText("Change colour of Webshop");
    const badge = picker.closest("label")!;
    expect(picker).toHaveValue("#3e63dd");

    // Dragging in the native picker fires `input` events: the badge follows,
    // but nothing is saved yet.
    fireEvent.input(picker, { target: { value: "#30a46c" } });
    expect(badge).toHaveStyle({ background: "#30a46c" });
    expect(setProjectColor).not.toHaveBeenCalled();

    // The picker committing fires the native `change` event: saved once.
    fireEvent.change(picker, { target: { value: "#e5484d" } });
    expect(setProjectColor).toHaveBeenCalledTimes(1);
    expect(setProjectColor).toHaveBeenCalledWith("proj-1", "#e5484d");
  });

  it("resets a picked colour from a right-click or the Delete key", () => {
    const setProjectColor = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [project({ color: "#3e63dd" })],
      setProjectColor,
    });
    render(<Sidebar />);

    const picker = screen.getByLabelText("Change colour of Webshop");
    fireEvent.contextMenu(picker);
    expect(setProjectColor).toHaveBeenLastCalledWith("proj-1", null);

    fireEvent.keyDown(picker, { key: "Delete" });
    expect(setProjectColor).toHaveBeenCalledTimes(2);
  });

  it("does not reset a badge that has no colour of its own", () => {
    const setProjectColor = vi.fn(() => Promise.resolve());
    useProjectStore.setState({ projects: [project()], setProjectColor });
    render(<Sidebar />);

    fireEvent.contextMenu(screen.getByLabelText("Change colour of Webshop"));
    expect(setProjectColor).not.toHaveBeenCalled();
  });

  it("collapses the group from its caret button, not from keys on the picker", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    const caret = screen.getByRole("button", { name: "Collapse Webshop" });
    expect(caret).toHaveAttribute("aria-expanded", "true");

    // Space on the focused picker belongs to the picker.
    fireEvent.keyDown(screen.getByLabelText("Change colour of Webshop"), {
      key: " ",
    });
    expect(caret).toHaveAttribute("aria-expanded", "true");

    fireEvent.click(caret);
    expect(
      screen.getByRole("button", { name: "Expand Webshop" }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("reorders projects when a project is dropped onto another", () => {
    const reorderProjects = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [
        project({ id: "proj-1", name: "Webshop" }),
        project({ id: "proj-2", name: "Marketing site" }),
      ],
      reorderProjects,
    });
    render(<Sidebar />);

    const [firstHandle, secondHandle] = screen.getAllByRole("img", {
      name: /Drag to reorder/,
    });
    // handle → header row → project group.
    const secondGroup = secondHandle.parentElement!
      .parentElement as HTMLElement;

    // jsdom's DragEvent has no real DataTransfer; the handler writes to it.
    fireEvent.dragStart(firstHandle, { dataTransfer: {} });
    fireEvent.dragOver(secondGroup);
    fireEvent.drop(secondGroup);

    expect(reorderProjects).toHaveBeenCalledWith("proj-1", "proj-2");
  });

  it("nests a worktree-rooted terminal under its worktree row, not the Terminals list", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "worktree_list"
        ? Promise.resolve([wt("fix-login")])
        : Promise.resolve([]),
    );
    useProjectStore.setState({ projects: [project()] });
    useProcessStore.setState({
      processes: [terminal("t1", "worktree-shell", "fix-login")],
    });

    render(<Sidebar />);

    // The terminal shows once, nested under its worktree — and the plain
    // Terminals subsection (no worktree processes left) falls back to its
    // empty hint instead of listing it a second time.
    await screen.findByText("worktree-shell");
    expect(screen.getByText("No terminals yet.")).toBeInTheDocument();
  });
});
