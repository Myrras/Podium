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
import { useAgentActivityStore } from "../state/agentActivityStore";
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
const initialActivity = useAgentActivityStore.getState();

function project(overrides: Partial<ProjectInfo> = {}): ProjectInfo {
  return {
    id: "proj-1",
    name: "Webshop",
    root: "/fake/webshop",
    iconInitials: "WS",
    configError: null,
    renamed: false,
    color: null,
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
    useAgentActivityStore.setState(initialActivity, true);
    vi.mocked(invoke)
      .mockReset()
      .mockImplementation(() => Promise.resolve([]));
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

  it("drops an unsaved picked colour when the picker blurs", () => {
    const setProjectColor = vi.fn(() => Promise.resolve());
    useProjectStore.setState({
      projects: [project({ color: "#3e63dd" })],
      setProjectColor,
    });
    render(<Sidebar />);

    const picker = screen.getByLabelText("Change colour of Webshop");
    const badge = picker.closest("label")!;
    fireEvent.input(picker, { target: { value: "#30a46c" } });
    expect(badge).toHaveStyle({ background: "#30a46c" });

    fireEvent.blur(picker);
    expect(badge).toHaveStyle({ background: "#3e63dd" });
    expect(setProjectColor).not.toHaveBeenCalled();
  });

  it("keeps a newer draft when an earlier save finishes", async () => {
    let finish!: () => void;
    const setProjectColor = vi.fn(
      () => new Promise<void>((resolve) => (finish = resolve)),
    );
    useProjectStore.setState({
      projects: [project({ color: "#3e63dd" })],
      setProjectColor,
    });
    render(<Sidebar />);

    const picker = screen.getByLabelText("Change colour of Webshop");
    const badge = picker.closest("label")!;
    fireEvent.change(picker, { target: { value: "#e5484d" } });
    fireEvent.input(picker, { target: { value: "#30a46c" } });
    finish();
    await Promise.resolve();
    await Promise.resolve();
    expect(badge).toHaveStyle({ background: "#30a46c" });
  });

  it("picks black or white initials by contrast", () => {
    useProjectStore.setState({
      projects: [project({ color: "#e5484d" })],
      setProjectColor: vi.fn(() => Promise.resolve()),
    });
    render(<Sidebar />);
    const picker = screen.getByLabelText("Change colour of Webshop");
    const badge = picker.closest("label")!;
    expect(badge).toHaveStyle({ color: "#000" });
    fireEvent.input(picker, { target: { value: "#1a1a2e" } });
    expect(badge).toHaveStyle({ color: "#fff" });
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

  it("collapses one subsection to its header with a row count", () => {
    useProjectStore.setState({ projects: [project()] });
    useProcessStore.setState({
      processes: [terminal("t1", "shell", null), terminal("t2", "logs", null)],
    });
    render(<Sidebar />);

    fireEvent.click(
      screen.getByRole("button", { name: "Terminals", expanded: true }),
    );

    // The count is part of the accessible name, not only of the visible text.
    const toggle = screen.getByRole("button", { name: "Terminals (2)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    // The rows are unmounted, not merely hidden.
    expect(screen.queryByText("shell")).not.toBeInTheDocument();
    // Persisted per project root, so it survives a restart.
    expect(
      useLayoutStore.getState().collapsedSections["/fake/webshop\nterminals"],
    ).toBe(true);

    fireEvent.click(toggle);
    expect(screen.getByText("shell")).toBeInTheDocument();
    // Expanding drops the key instead of storing `false`.
    expect(useLayoutStore.getState().collapsedSections).toEqual({});
  });

  it("flags a collapsed subsection whose hidden row needs input", () => {
    useProjectStore.setState({ projects: [project()] });
    useProcessStore.setState({
      processes: [terminal("t1", "shell", null), terminal("t2", "logs", null)],
    });
    useAgentActivityStore.setState({ activity: { t2: "waiting" } });
    render(<Sidebar />);

    // Expanded, the row itself shows it; the header stays quiet.
    expect(
      screen.queryByRole("img", { name: /needs your input/ }),
    ).not.toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "Terminals", expanded: true }),
    );
    expect(
      screen.getByRole("button", { name: "Terminals (2) 1 needs your input" }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("collapses and expands every subsection from the project header", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    fireEvent.click(
      screen.getByRole("button", { name: "Collapse all sections of Webshop" }),
    );
    for (const title of ["To-dos", "Agents", "Terminals", "Scratchpads"]) {
      expect(
        screen.getByRole("button", { name: title, expanded: false }),
      ).toBeInTheDocument();
    }

    fireEvent.click(
      screen.getByRole("button", { name: "Expand all sections of Webshop" }),
    );
    expect(
      screen.getByRole("button", { name: "To-dos", expanded: true }),
    ).toBeInTheDocument();
    expect(useLayoutStore.getState().collapsedSections).toEqual({});
  });

  it("offers Expand all once every visible subsection is collapsed by hand", () => {
    // No services and no worktrees: Processes and Worktrees are not rendered,
    // so they must not keep the button on "Collapse all".
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    for (const title of ["To-dos", "Agents", "Terminals", "Scratchpads"]) {
      fireEvent.click(
        screen.getByRole("button", { name: title, expanded: true }),
      );
    }

    expect(
      screen.getByRole("button", { name: "Expand all sections of Webshop" }),
    ).toBeInTheDocument();
  });

  it("expands a collapsed subsection when its add button is used", () => {
    useProjectStore.setState({ projects: [project()] });
    render(<Sidebar />);

    fireEvent.click(
      screen.getByRole("button", { name: "To-dos", expanded: true }),
    );
    fireEvent.click(screen.getByRole("button", { name: "New to-do" }));

    expect(
      screen.getByRole("button", { name: "To-dos", expanded: true }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("New to-do text")).toBeInTheDocument();
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
