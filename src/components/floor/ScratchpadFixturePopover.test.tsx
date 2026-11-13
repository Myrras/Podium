import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));
vi.mock("../ScratchpadEditor", () => ({
  SCRATCHPAD_PLACEHOLDER: "",
  ScratchpadEditor: () => <textarea aria-label="Scratchpad content" />,
}));
vi.mock("../ScratchpadToolbar", () => ({ ScratchpadToolbar: () => null }));

import type { ScratchpadInfo } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { ScratchpadFixturePopover } from "./ScratchpadFixturePopover";

const initialFloor = useFloorStore.getState();
const initialScratchpad = useScratchpadStore.getState();

function sp(id: string, title: string): ScratchpadInfo {
  return {
    id,
    projectId: "p1",
    title,
    content: "",
    archived: false,
    archivedAt: null,
    createdAt: "2024-04-03T12:00:00Z",
    updatedAt: "2024-04-03T12:00:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: null,
  };
}

function renderPopover(itemId: string | null) {
  const anchor = document.createElement("button");
  document.body.append(anchor);
  return render(
    <ScratchpadFixturePopover projectId="p1" itemId={itemId} anchor={anchor} />,
  );
}

beforeEach(() => {
  useFloorStore.setState(initialFloor, true);
  useScratchpadStore.setState(
    {
      ...initialScratchpad,
      scratchpadsByProject: { p1: [sp("a", "Notes A")] },
      refresh: vi.fn(() => Promise.resolve()),
    },
    true,
  );
});

describe("ScratchpadFixturePopover", () => {
  it("lists_scratchpads_and_opens_one_inside_the_popover", () => {
    useFloorStore.setState({
      popover: { projectId: "p1", kind: "scratchpads", itemId: null },
    });
    renderPopover(null);

    expect(
      screen.getByRole("button", { name: "New scratchpad" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByText("Notes A"));
    expect(useFloorStore.getState().popover?.itemId).toBe("a");
  });

  it("opened_item_has_back_to_scratchpads", () => {
    useFloorStore.setState({
      popover: { projectId: "p1", kind: "scratchpads", itemId: "a" },
    });
    renderPopover("a");

    expect(screen.getByLabelText("Scratchpad title")).toHaveValue("Notes A");
    expect(screen.queryByLabelText("On this page")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Back to scratchpads" }),
    );
    expect(useFloorStore.getState().popover?.itemId).toBeNull();
  });

  it("opened_item_start_agent_on_this_opens_the_modal", () => {
    renderPopover("a");

    fireEvent.click(
      screen.getByRole("button", { name: "Start agent on this" }),
    );
    expect(useFloorStore.getState().scratchpadAgentModal).toEqual({
      projectId: "p1",
      scratchpadIds: ["a"],
      initialName: "Notes A",
    });
  });

  it("start_agent_opens_the_scratchpad_agent_modal", () => {
    renderPopover(null);

    fireEvent.click(
      screen.getByRole("button", { name: 'Start an agent on "Notes A"' }),
    );
    expect(useFloorStore.getState().scratchpadAgentModal).toEqual({
      projectId: "p1",
      scratchpadIds: ["a"],
      initialName: "Notes A",
    });
  });
});
