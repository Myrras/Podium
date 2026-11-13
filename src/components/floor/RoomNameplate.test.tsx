import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectInfo } from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";

const m = vi.hoisted(() => ({ rooms: [] as unknown[] }));
vi.mock("../../lib/floor/camera", () => ({ jumpTo: vi.fn() }));
vi.mock("../../lib/floor/startAgent", () => ({ startAgentIn: vi.fn() }));
vi.mock("../../lib/floor/rooms", () => ({ getRooms: () => m.rooms }));

import { jumpTo } from "../../lib/floor/camera";
import { startAgentIn } from "../../lib/floor/startAgent";
import { RoomNameplate } from "./RoomNameplate";

const mk = (id: string, x: number, y: number): RoomModel => ({
  project: {
    id,
    root: `/r/${id}`,
    name: `Proj ${id}`,
    iconInitials: "PX",
    color: null,
  } as unknown as ProjectInfo,
  rect: { x, y, w: 400, h: 400 },
  agents: [],
  terminals: [],
  services: [],
  needs: 2,
  crashed: 1,
  lit: false,
});
const a = mk("a", 0, 0);
const b = mk("b", 800, 0);

// jsdom has no PointerEvent: without it clientX/pointerId are dropped.
class TestPointerEvent extends MouseEvent {
  pointerId: number;
  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 0;
  }
}
vi.stubGlobal("PointerEvent", TestPointerEvent);

beforeEach(() => {
  vi.clearAllMocks();
  m.rooms = [a, b];
  useFloorStore.getState().reset();
  useLayoutStore.setState({ floorPositions: {} });
});

const grip = () => screen.getByRole("button", { name: /Proj a/ });
const drag = (dx: number) => {
  fireEvent.pointerDown(grip(), {
    clientX: 0,
    clientY: 0,
    button: 0,
    pointerId: 1,
  });
  fireEvent.pointerMove(grip(), { clientX: dx, clientY: 0, pointerId: 1 });
};

describe("RoomNameplate", () => {
  it("shows counts, jumps on click, + and menu work", () => {
    render(<RoomNameplate room={a} />);
    expect(screen.getByText("? 2")).toBeTruthy();
    expect(screen.getByText("! 1")).toBeTruthy();
    fireEvent.click(grip());
    expect(jumpTo).toHaveBeenCalledWith("a");
    fireEvent.click(screen.getByRole("button", { name: "New agent" }));
    expect(startAgentIn).toHaveBeenCalled();
    const menu = screen.getByRole("button", { name: "Project menu" });
    expect(menu.dataset.fixture).toBe("a:menu");
    fireEvent.click(menu);
    expect(useFloorStore.getState().popover).toEqual({
      projectId: "a",
      kind: "menu",
      itemId: null,
    });
  });

  it("drags with snap, saves, and does not jump", () => {
    render(<RoomNameplate room={a} />);
    drag(130);
    fireEvent.pointerUp(grip(), { pointerId: 1 });
    fireEvent.click(grip());
    expect(useLayoutStore.getState().floorPositions["/r/a"]).toEqual([120, 0]);
    expect(jumpTo).not.toHaveBeenCalled();
  });

  it("marks and drops a colliding drag", () => {
    const { container } = render(<RoomNameplate room={a} />);
    drag(700);
    expect(
      (container.firstElementChild as HTMLElement).dataset.bad,
    ).toBeDefined();
    fireEvent.pointerUp(grip(), { pointerId: 1 });
    expect(useLayoutStore.getState().floorPositions["/r/a"]).toBeUndefined();
  });

  it("moves one cell with arrow keys unless blocked", () => {
    const { rerender } = render(<RoomNameplate room={a} />);
    fireEvent.keyDown(grip(), { key: "ArrowDown" });
    expect(useLayoutStore.getState().floorPositions["/r/a"]).toEqual([0, 40]);
    useLayoutStore.setState({ floorPositions: {} });
    rerender(<RoomNameplate room={mk("a", 360, 0)} />);
    fireEvent.keyDown(grip(), { key: "ArrowRight" });
    expect(useLayoutStore.getState().floorPositions["/r/a"]).toBeUndefined();
  });
});
