import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const stub = vi.hoisted(
  () =>
    (name: string) =>
    ({
      projectId,
      itemId,
      anchor,
    }: {
      projectId: string;
      itemId: string | null;
      anchor: unknown;
    }) => (
      <div data-testid={name}>
        {projectId}/{String(itemId)}/{anchor ? "anchor" : "none"}
      </div>
    ),
);
vi.mock("./TodoFixturePopover", () => ({ TodoFixturePopover: stub("todos") }));
vi.mock("./ScratchpadFixturePopover", () => ({
  ScratchpadFixturePopover: stub("scratchpads"),
}));
vi.mock("./ServicesFixturePopover", () => ({
  ServicesFixturePopover: stub("services"),
}));
// Uncontrolled input: keeps its value unless the content remounts.
vi.mock("./ProjectMenuPopover", () => ({
  ProjectMenuPopover: ({ projectId }: { projectId: string }) => (
    <input aria-label="menu name" defaultValue={projectId} />
  ),
}));

import type { ProjectInfo } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { FloorPopovers } from "./FloorPopovers";

let button: HTMLButtonElement;

beforeEach(() => {
  useFloorStore.getState().reset();
  useProjectStore.setState(
    { projects: [{ id: "p1", name: "Demo" } as ProjectInfo] },
    false,
  );
  button = document.createElement("button");
  button.dataset.fixture = "p1:services";
  document.body.append(button);
});
afterEach(() => button.remove());

describe("FloorPopovers", () => {
  it("renders nothing while no popover is open", () => {
    render(<FloorPopovers />);
    expect(screen.queryByTestId("services")).toBeNull();
  });

  it("renders the content for the kind at its anchor", () => {
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1", kind: "services", itemId: "x" });
    render(<FloorPopovers />);
    expect(screen.getByTestId("services")).toHaveTextContent("p1/x/anchor");
    expect(screen.queryByTestId("todos")).toBeNull();
  });

  it("closes when the anchor is missing", () => {
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1", kind: "todos", itemId: null });
    render(<FloorPopovers />);
    expect(screen.queryByTestId("todos")).toBeNull();
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("remounts the content when another project's popover opens", () => {
    useProjectStore.setState(
      {
        projects: [
          { id: "p1", name: "Demo" } as ProjectInfo,
          { id: "p2", name: "Other" } as ProjectInfo,
        ],
      },
      false,
    );
    const menus = ["p1", "p2"].map((id) => {
      const b = document.createElement("button");
      b.dataset.fixture = `${id}:menu`;
      document.body.append(b);
      return b;
    });
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1", kind: "menu", itemId: null });
    render(<FloorPopovers />);
    expect(screen.getByLabelText("menu name")).toHaveValue("p1");
    act(() =>
      useFloorStore
        .getState()
        .openPopover({ projectId: "p2", kind: "menu", itemId: null }),
    );
    expect(screen.getByLabelText("menu name")).toHaveValue("p2");
    menus.forEach((b) => b.remove());
  });

  it("closes when the project is gone", () => {
    useProjectStore.setState({ projects: [] }, false);
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1", kind: "services", itemId: null });
    render(<FloorPopovers />);
    expect(screen.queryByTestId("services")).toBeNull();
    expect(useFloorStore.getState().popover).toBeNull();
  });
});
