import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

const h = vi.hoisted(() => ({ rooms: [] as unknown[] }));
vi.mock("../../lib/floor/camera", () => ({ jumpTo: vi.fn(), zoomBy: vi.fn() }));
vi.mock("../../lib/floor/rooms", () => ({
  useRooms: () => h.rooms,
  deskState: () => "working",
}));

import { jumpTo, zoomBy } from "../../lib/floor/camera";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProjectStore } from "../../state/projectStore";
import { FloorHeader } from "./FloorHeader";

const room = (id: string, needs = 0, crashed = 0) => ({
  project: { id, name: `Proj ${id}`, color: null, iconInitials: "PX" },
  agents: [{ id: `a-${id}` }],
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

  it("chips jump, totals show, buttons work", () => {
    render(<FloorHeader />);
    fireEvent.click(screen.getByRole("button", { name: /Proj p1/ }));
    expect(jumpTo).toHaveBeenCalledWith("p1");
    expect(screen.getByText("2 need you")).toBeTruthy();
    expect(screen.getByText("1 crashed")).toBeTruthy();
    expect(screen.getByText("2 working")).toBeTruthy();
    fireEvent.click(screen.getByText("Zoom out"));
    expect(zoomBy).toHaveBeenCalledWith(0.8);
    fireEvent.click(screen.getByText("Zoom in"));
    expect(zoomBy).toHaveBeenCalledWith(1.25);
    fireEvent.click(screen.getByText("Reset layout"));
    expect(reset).toHaveBeenCalled();
    expect(jumpTo).toHaveBeenLastCalledWith("all");
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
});
