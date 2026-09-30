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
    const secondGroup = secondHandle.closest('[role="button"]')!
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

  it("collapses one subsection to its header with a row count", () => {
    useProjectStore.setState({ projects: [project()] });
    useProcessStore.setState({
      processes: [terminal("t1", "shell", null), terminal("t2", "logs", null)],
    });
    render(<Sidebar />);

    const toggle = screen.getByRole("button", { name: "Collapse Terminals" });
    fireEvent.click(toggle);

    const expand = screen.getByRole("button", { name: "Expand Terminals" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    expect(expand).toHaveTextContent("Terminals (2)");
    expect(expand.closest("[data-collapsed]")).toHaveAttribute(
      "data-collapsed",
      "true",
    );
    // Persisted per project root, so it survives a restart.
    expect(
      useLayoutStore.getState().collapsedSections["/fake/webshop\nterminals"],
    ).toBe(true);
  });

  it("collapses and expands every subsection from the project header", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    fireEvent.click(
      screen.getByRole("button", { name: "Collapse all sections of Webshop" }),
    );
    for (const title of ["To-dos", "Agents", "Terminals", "Scratchpads"]) {
      expect(
        screen.getByRole("button", { name: `Expand ${title}` }),
      ).toBeInTheDocument();
    }

    fireEvent.click(
      screen.getByRole("button", { name: "Expand all sections of Webshop" }),
    );
    expect(
      screen.getByRole("button", { name: "Collapse To-dos" }),
    ).toBeInTheDocument();
  });

  it("expands a collapsed subsection when its add button is used", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    fireEvent.click(screen.getByRole("button", { name: "Collapse To-dos" }));
    fireEvent.click(screen.getByRole("button", { name: "New to-do" }));

    expect(
      screen.getByRole("button", { name: "Collapse To-dos" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("New to-do text")).toBeInTheDocument();
  });
});
