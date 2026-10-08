import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
import { useSettingsStore } from "../state/settingsStore";
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
    sections: {
      todos: true,
      terminals: true,
      worktrees: true,
      scratchpads: true,
      issues: false,
    },
    nativeInWslMode: false,
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

  it("hides a section whose toggle is off and keeps the others", () => {
    useProjectStore.setState({
      projects: [
        project({
          sections: {
            todos: true,
            terminals: true,
            worktrees: true,
            scratchpads: false,
            issues: false,
          },
        }),
      ],
    });
    render(<Sidebar />);

    expect(screen.queryByText("Scratchpads")).not.toBeInTheDocument();
    expect(screen.getByText("Terminals")).toBeInTheDocument();
    // Agents are not toggleable.
    expect(screen.getByText("Agents")).toBeInTheDocument();
  });

  it("shows the Git board as a header button, not a section, when enabled", () => {
    useProjectStore.setState({
      projects: [
        project({
          sections: {
            todos: true,
            terminals: true,
            worktrees: true,
            scratchpads: true,
            issues: true,
          },
        }),
      ],
    });
    const { rerender } = render(<Sidebar />);
    expect(
      screen.getByRole("button", { name: /^Git board/ }),
    ).toBeInTheDocument();

    useProjectStore.setState({ projects: [project()] });
    rerender(<Sidebar />);
    expect(
      screen.queryByRole("button", { name: /^Git board/ }),
    ).not.toBeInTheDocument();
  });

  it("toggling a Sections checkbox calls setProjectSections with the changed key", () => {
    const setProjectSections = vi.fn(() => Promise.resolve());
    useProjectStore.setState({ projects: [project()], setProjectSections });
    render(<Sidebar />);

    fireEvent.click(
      screen.getByRole("button", { name: "Sections for Webshop" }),
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "Scratchpads" }));

    expect(setProjectSections).toHaveBeenCalledWith("proj-1", {
      scratchpads: false,
    });
  });

  it("two quick Sections toggles both stick", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);
    fireEvent.click(
      screen.getByRole("button", { name: "Sections for Webshop" }),
    );
    // Neither backend call has answered yet.
    vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));
    fireEvent.click(screen.getByRole("checkbox", { name: "Scratchpads" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Terminals" }));

    const calls = vi
      .mocked(invoke)
      .mock.calls.filter(([cmd]) => cmd === "project_set_sections");
    expect(calls[calls.length - 1]?.[1]).toEqual({
      projectId: "proj-1",
      sections: {
        todos: true,
        terminals: false,
        worktrees: true,
        scratchpads: false,
        issues: false,
      },
    });
  });

  it("hiding the Git board section closes that project's open board", () => {
    useProjectStore.setState({
      projects: [
        project({
          sections: {
            todos: true,
            terminals: true,
            worktrees: true,
            scratchpads: true,
            issues: true,
          },
        }),
      ],
    });
    useLayoutStore.setState({ openBoard: { projectId: "proj-1" } });
    render(<Sidebar />);
    fireEvent.click(
      screen.getByRole("button", { name: "Sections for Webshop" }),
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "Git board" }));
    expect(useLayoutStore.getState().openBoard).toBeNull();
  });

  it("drops the hover actions while renaming so they never cover the input", () => {
    // The actions overlay the end of the name (CSS-only, not asserted here);
    // while renaming they are not rendered at all.
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);
    expect(
      screen.getByRole("button", { name: "Reload podium.yml for Webshop" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Rename Webshop" }));

    expect(
      screen.getByRole("textbox", { name: "Rename Webshop" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reload podium.yml for Webshop" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the podium.yml error icon outside the slot the hover actions overlay", () => {
    // The strip is clipped to the name's slot (CSS-only, not asserted here),
    // so the error icon — whose tooltip is the only place the error text
    // shows — must sit after that slot, not inside it.
    useProjectStore.setState({
      projects: [project({ configError: "unknown field `procesess`" })],
    });
    render(<Sidebar />);

    const rename = screen.getByRole("button", { name: "Rename Webshop" });
    const name = screen.getByText("Webshop");
    const error = screen.getByRole("img", { name: "podium.yml error" });
    const slot = name.parentElement!;

    expect(slot).toContainElement(rename);
    expect(slot).not.toContainElement(error);
    expect(
      slot.compareDocumentPosition(error) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(error).toHaveAttribute("title", "unknown field `procesess`");
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

  describe("new terminal shell", () => {
    const initialSettings = useSettingsStore.getState();
    afterEach(() => useSettingsStore.setState(initialSettings, true));

    async function addTerminalIn(p: ProjectInfo) {
      useSettingsStore.setState({
        terminal: { ...initialSettings.terminal, shell: "zsh" },
      });
      useProjectStore.setState({ projects: [p] });
      render(<Sidebar />);
      fireEvent.click(screen.getByRole("button", { name: "New terminal" }));
      await waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("process_add", expect.anything()),
      );
      const call = vi
        .mocked(invoke)
        .mock.calls.find(([cmd]) => cmd === "process_add");
      return (call?.[1] as { spec: { command?: string } }).spec;
    }

    it("launches the configured shell", async () => {
      const spec = await addTerminalIn(project());
      expect(spec.command).toBe("zsh");
    });

    it("leaves it out for a Windows-drive project running natively in WSL mode", async () => {
      // The setting names the distro's shell; `cmd /C zsh` would fail, so
      // the backend's native default (PowerShell) opens instead.
      const spec = await addTerminalIn(project({ nativeInWslMode: true }));
      expect(spec.command).toBeUndefined();
    });
  });
});
