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

vi.mock("../../lib/floor/menus", () => ({
  scratchpadMenu: vi.fn(() => [{ label: "Menu item", onSelect: () => {} }]),
}));

import type { ScratchpadInfo } from "../../ipc/types";
import { scratchpadMenu } from "../../lib/floor/menus";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
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

  it("one header bar: title, count and the list actions", () => {
    renderPopover(null);
    const heading = screen.getByRole("heading");
    expect(heading).toHaveTextContent("Scratchpads1");
    const header = heading.parentElement as HTMLElement;
    expect(header).toContainElement(
      screen.getByRole("button", { name: "New scratchpad" }),
    );
    expect(header).toContainElement(
      screen.getByRole("button", { name: "View archived scratchpads" }),
    );
  });

  it("rows show the relative updated time", () => {
    useScratchpadStore.setState({
      scratchpadsByProject: {
        p1: [{ ...sp("a", "Notes A"), updatedAt: new Date().toISOString() }],
      },
    });
    renderPopover(null);
    expect(screen.getByText("now").tagName).toBe("TIME");
  });

  it("empty list invites to add one", () => {
    useScratchpadStore.setState({ scratchpadsByProject: { p1: [] } });
    renderPopover(null);
    expect(
      screen.getByText("No scratchpads yet. Add one with +."),
    ).toBeInTheDocument();
  });

  it("does not highlight the scratchpad open in Project mode's work area", () => {
    useLayoutStore.setState({
      openScratchpad: { projectId: "p1", scratchpadId: "a" },
    });
    renderPopover(null);
    expect(screen.getByText("Notes A").closest("[data-open]")).toBeNull();
    useLayoutStore.setState({ openScratchpad: null });
  });

  it("right-click on a row opens its menu", () => {
    renderPopover(null);
    const ev = fireEvent.contextMenu(screen.getByText("Notes A"));
    expect(ev).toBe(false);
    expect(scratchpadMenu).toHaveBeenLastCalledWith(
      "p1",
      expect.objectContaining({ id: "a" }),
    );
    expect(screen.getByRole("menuitem", { name: "Menu item" })).toBeVisible();
  });

  it("opened_item_title_in_header_actions_in_footer", () => {
    renderPopover("a");
    const header = document.querySelector("header") as HTMLElement;
    expect(header).toContainElement(screen.getByLabelText("Scratchpad title"));
    const footer = document.querySelector("footer") as HTMLElement;
    for (const name of ["Start agent on this", "Full screen", "Archive"])
      expect(footer).toContainElement(screen.getByRole("button", { name }));
    expect(screen.queryByLabelText("Close scratchpad")).toBeNull();

    // Full screen covers the popover, so the pane takes its own header back.
    fireEvent.click(screen.getByRole("button", { name: "Full screen" }));
    expect(screen.getByLabelText("Exit fullscreen")).toBeInTheDocument();
    expect(header).not.toContainElement(
      screen.getByLabelText("Scratchpad title"),
    );
  });

  it("full_screen_pane_stays_inside_the_popover_and_re_docks", () => {
    renderPopover("a");
    const popover = screen.getByRole("group");
    fireEvent.click(screen.getByRole("button", { name: "Full screen" }));

    // Popover.module.css drops the popover's backdrop-filter via :has(), so
    // the fixed pane covers the window instead of the popover.
    const pane = document.querySelector("[data-fullscreen]");
    expect(popover).toContainElement(pane as HTMLElement);
    expect(popover.matches(":has([data-fullscreen])")).toBe(true);

    fireEvent.click(screen.getByLabelText("Exit fullscreen"));
    expect(document.querySelector("[data-fullscreen]")).toBeNull();
    expect(popover.matches(":has([data-fullscreen])")).toBe(false);
    expect(document.querySelector("header")).toContainElement(
      screen.getByLabelText("Scratchpad title"),
    );
    expect(document.querySelector("footer")).toContainElement(
      screen.getByRole("button", { name: "Full screen" }),
    );
  });
});
