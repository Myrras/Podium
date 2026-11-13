import { render, screen } from "@testing-library/react";
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
vi.mock("./ProjectMenuPopover", () => ({ ProjectMenuPopover: stub("menu") }));

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
