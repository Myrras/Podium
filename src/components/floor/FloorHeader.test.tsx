import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

const h = vi.hoisted(() => ({ rooms: [] as unknown[] }));
vi.mock("../../lib/floor/camera", () => ({ jumpTo: vi.fn(), zoomBy: vi.fn() }));
vi.mock("../../lib/floor/rooms", () => ({ useRooms: () => h.rooms }));

import { Profiler } from "react";

import { jumpTo, zoomBy } from "../../lib/floor/camera";
import { useAgentActivityStore } from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProjectStore } from "../../state/projectStore";
import { FloorHeader } from "./FloorHeader";

const room = (id: string, needs = 0, crashed = 0) => ({
  project: { id, name: `Proj ${id}`, color: null, iconInitials: "PX" },
  agents: [{ id: `a-${id}` }],
  working: 1,
  needs,
  crashed,
});

describe("FloorHeader", () => {
  const open = vi.fn(() => Promise.resolve());
  const reset = vi.fn();
  beforeEach(() => {
    vi.clearAllMocks();
    h.rooms = [room("p1", 2, 0), room("p2", 0, 1)];
    useFloorStore.getState().reset();
    useProjectStore.setState({ openProjectDialog: open }, false);
    useLayoutStore.setState({ resetFloorPositions: reset }, false);
  });

  it("does not re-render on an activity tick", () => {
    const renders = vi.fn();
    render(
      <Profiler id="header" onRender={renders}>
        <FloorHeader />
      </Profiler>,
    );
    renders.mockClear();
    act(() => useAgentActivityStore.setState({ details: {} }));
    expect(renders).not.toHaveBeenCalled();
  });

  it("chips jump, totals show, buttons work", () => {
    render(<FloorHeader />);
    fireEvent.click(screen.getByRole("button", { name: /Proj p1/ }));
    expect(jumpTo).toHaveBeenCalledWith("p1");
    expect(screen.getByText("2 need you")).toBeTruthy();
    expect(screen.getByText("1 crashed")).toBeTruthy();
    expect(screen.getByText("2 working")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(zoomBy).toHaveBeenCalledWith(0.8);
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(zoomBy).toHaveBeenCalledWith(1.25);
    fireEvent.click(screen.getByRole("button", { name: "Reset layout" }));
    expect(reset).toHaveBeenCalled();
    expect(jumpTo).toHaveBeenLastCalledWith("all");
    for (const name of ["Zoom out", "Zoom in", "Fit all", "Reset layout"])
      expect(
        screen.getByRole("button", { name }).closest("[data-popover-keep]"),
      ).not.toBeNull();
    fireEvent.click(screen.getByText("Add project…"));
    expect(open).toHaveBeenCalled();
  });

  it("pressed state follows focus", () => {
    useFloorStore.setState({ focus: "p2" });
    render(<FloorHeader />);
    expect(
      screen
        .getByRole("button", { name: /Proj p2/ })
        .getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("shows the empty text without projects", () => {
    h.rooms = [];
    render(<FloorHeader />);
    expect(screen.getByText(/No projects yet/)).toBeTruthy();
  });

  it("Esc shows all unless a popover or terminal is open", () => {
    render(<FloorHeader />);
    const header = screen.getByRole("banner");
    fireEvent.keyDown(header, { key: "Escape" });
    expect(jumpTo).toHaveBeenCalledWith("all");
    vi.mocked(jumpTo).mockClear();
    act(() => useFloorStore.setState({ terminalId: "t1" }));
    fireEvent.keyDown(header, { key: "Escape" });
    expect(jumpTo).not.toHaveBeenCalled();
  });

  it("marks the chip row edges that can scroll", () => {
    const { container } = render(<FloorHeader />);
    const row = container.querySelector("header > div") as HTMLElement;
    const dim = (k: string, v: number) =>
      Object.defineProperty(row, k, { value: v, configurable: true });
    dim("clientWidth", 100);
    dim("scrollWidth", 300);
    fireEvent.scroll(row);
    expect(row.hasAttribute("data-more-right")).toBe(true);
    expect(row.hasAttribute("data-more-left")).toBe(false);
    row.scrollLeft = 200;
    fireEvent.scroll(row);
    expect(row.hasAttribute("data-more-right")).toBe(false);
    expect(row.hasAttribute("data-more-left")).toBe(true);
  });
});
