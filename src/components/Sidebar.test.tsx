import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { ProjectInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { Sidebar } from "./Sidebar";

const initialProject = useProjectStore.getState();
const initialLayout = useLayoutStore.getState();
const initialProcess = useProcessStore.getState();

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

describe("Sidebar project header", () => {
  beforeEach(() => {
    useProjectStore.setState(initialProject, true);
    useLayoutStore.setState(initialLayout, true);
    useProcessStore.setState(initialProcess, true);
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
});
