import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { RoomModel } from "../../lib/floor/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

const allRooms = [
  {
    project: { id: "p1", name: "Alpha" },
    rect: { x: 0, y: 0, w: 200, h: 120 },
  },
  {
    project: { id: "p2", name: "Beta" },
    rect: { x: 400, y: 0, w: 200, h: 120 },
  },
] as unknown as RoomModel[];
let rooms = allRooms;
const useFloorData = vi.fn();
vi.mock("../../lib/floor/rooms", () => ({
  useRooms: () => rooms,
  useFloorData: () => useFloorData(),
}));
const jumpTo = vi.fn();
vi.mock("../../lib/floor/camera", () => ({
  jumpTo: (t: string) => jumpTo(t),
}));
vi.mock("./Room", () => ({
  Room: ({ room }: { room: RoomModel }) => (
    <div data-testid="room">
      {room.project.name}
      <button>Desk</button>
    </div>
  ),
}));

import { useFloorStore } from "../../state/floorStore";
import { FloorMap } from "./FloorMap";

const map = () => screen.getByRole("region", { name: "Floor map" });
const layer = () => map().firstElementChild as HTMLElement;

// jsdom has no PointerEvent; without it fireEvent drops clientX and button.
window.PointerEvent ??= class extends MouseEvent {} as typeof PointerEvent;

beforeEach(() => {
  vi.clearAllMocks();
  rooms = allRooms;
  useFloorStore.getState().reset();
});

describe("FloorMap", () => {
  it("renders every room in the world layer and loads the floor data", () => {
    render(<FloorMap />);
    expect(screen.getAllByTestId("room").map((r) => r.textContent)).toEqual([
      "AlphaDesk",
      "BetaDesk",
    ]);
    expect(layer().contains(screen.getAllByTestId("room")[0])).toBe(true);
    expect(useFloorData).toHaveBeenCalled();
  });

  it("writes the camera to the layer and flags far zoom", () => {
    render(<FloorMap />);
    expect(layer().style.transform).toBe("translate(0px, 0px) scale(1)");
    expect(map().hasAttribute("data-far")).toBe(false);

    act(() =>
      useFloorStore
        .getState()
        .setCamera({ x: 10, y: -20, z: 0.3 }, { animate: true }),
    );
    expect(layer().style.transform).toBe("translate(10px, -20px) scale(0.3)");
    expect(layer().style.getPropertyValue("--z")).toBe("0.3");
    expect(layer().hasAttribute("data-animate")).toBe(true);
    expect(map().hasAttribute("data-far")).toBe(true);
    expect(map().style.getPropertyValue("--gs")).toBe("12px");
  });

  it("zooms around the cursor on Ctrl/Cmd + wheel only", () => {
    render(<FloorMap />);
    fireEvent.wheel(map(), { deltaY: -50, clientX: 100, clientY: 50 });
    expect(useFloorStore.getState().camera).toEqual({ x: 0, y: 0, z: 1 });

    const e = new WheelEvent("wheel", {
      deltaY: -100,
      clientX: 100,
      clientY: 50,
      ctrlKey: true,
      cancelable: true,
    });
    act(() => void map().dispatchEvent(e));
    expect(e.defaultPrevented).toBe(true);
    const { camera, focus } = useFloorStore.getState();
    expect(camera.z).toBe(2); // e^1 clamped to ZOOM_MAX
    expect(camera.x).toBe(-100); // the point under the cursor stays put
    expect(camera.y).toBe(-50);
    expect(focus).toBeNull();
  });

  it("pans past a 4 px threshold and not from a button", () => {
    useFloorStore
      .getState()
      .setCamera({ x: 5, y: 5, z: 1 }, { focus: "all", animate: true });
    render(<FloorMap />);

    fireEvent.pointerDown(map(), { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(map(), { clientX: 2, clientY: 2, buttons: 1 });
    expect(useFloorStore.getState().camera).toEqual({ x: 5, y: 5, z: 1 });

    fireEvent.pointerMove(map(), { clientX: 30, clientY: -10, buttons: 1 });
    expect(useFloorStore.getState()).toMatchObject({
      camera: { x: 35, y: -5, z: 1 },
      focus: null,
      animate: false,
    });
    fireEvent.pointerUp(map());

    fireEvent.pointerDown(screen.getAllByRole("button")[0], {
      clientX: 0,
      clientY: 0,
      button: 0,
    });
    fireEvent.pointerMove(map(), { clientX: 50, clientY: 50, buttons: 1 });
    expect(useFloorStore.getState().camera).toEqual({ x: 35, y: -5, z: 1 });
  });

  it("ends a pan whose release it missed", () => {
    render(<FloorMap />);
    fireEvent.pointerDown(map(), { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(map(), { clientX: 20, clientY: 0, buttons: 1 });
    fireEvent.pointerMove(map(), { clientX: 60, clientY: 0, buttons: 0 });
    fireEvent.pointerMove(map(), { clientX: 90, clientY: 0, buttons: 1 });
    expect(useFloorStore.getState().camera.x).toBe(20);
  });

  it("keeps a popover open while panning, closes it on a background click", () => {
    const popover = {
      projectId: "p1" as never,
      kind: "todos" as const,
      itemId: null,
    };
    render(<FloorMap />);
    expect(map().hasAttribute("data-popover-keep")).toBe(true);
    useFloorStore.getState().openPopover(popover);

    fireEvent.pointerDown(map(), { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(map(), { clientX: 30, clientY: 0, buttons: 1 });
    fireEvent.pointerUp(map());
    expect(useFloorStore.getState().popover).toEqual(popover);

    fireEvent.pointerDown(screen.getAllByRole("button")[0], { button: 0 });
    fireEvent.pointerUp(map());
    expect(useFloorStore.getState().popover).toEqual(popover);

    fireEvent.pointerDown(map(), { clientX: 0, clientY: 0, button: 0 });
    fireEvent.pointerMove(map(), { clientX: 2, clientY: 1, buttons: 1 });
    fireEvent.pointerUp(map());
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("shows everything on Esc unless a popover or terminal is open", () => {
    render(<FloorMap />);
    fireEvent.keyDown(map(), { key: "Escape" });
    expect(jumpTo).toHaveBeenCalledWith("all");

    jumpTo.mockClear();
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1" as never, kind: "todos", itemId: null });
    fireEvent.keyDown(map(), { key: "Escape" });
    useFloorStore.setState({ popover: null, terminalId: "t1" as never });
    fireEvent.keyDown(map(), { key: "Escape" });
    expect(jumpTo).not.toHaveBeenCalled();
  });

  describe("viewport size", () => {
    let resize: (w: number, h: number) => void;
    beforeEach(() => {
      vi.stubGlobal(
        "ResizeObserver",
        class {
          constructor(cb: ResizeObserverCallback) {
            resize = (width, height) =>
              cb(
                [{ contentRect: { width, height } } as ResizeObserverEntry],
                this as unknown as ResizeObserver,
              );
          }
          observe() {}
          disconnect() {}
        },
      );
    });
    afterEach(() => vi.unstubAllGlobals());

    it("shows everything once known, then keeps the focus framed", () => {
      render(<FloorMap />);
      act(() => resize(800, 600));
      expect(useFloorStore.getState().viewport).toEqual({ w: 800, h: 600 });
      expect(jumpTo).toHaveBeenCalledWith("all");

      expect(jumpTo).toHaveBeenCalledTimes(1);
      jumpTo.mockClear();
      useFloorStore.setState({ focus: null });
      act(() => resize(900, 600));
      expect(jumpTo).not.toHaveBeenCalled();

      useFloorStore.setState({ focus: "p2" as never });
      act(() => resize(1000, 600));
      expect(jumpTo).toHaveBeenCalledWith("p2");
    });

    it("fits all once the rooms arrive after the viewport", () => {
      rooms = [];
      const { rerender } = render(<FloorMap />);
      act(() => resize(800, 600));
      expect(jumpTo).not.toHaveBeenCalled();

      rooms = allRooms;
      rerender(<FloorMap />);
      expect(jumpTo).toHaveBeenCalledWith("all");
      jumpTo.mockClear();
      rooms = [...allRooms];
      rerender(<FloorMap />);
      expect(jumpTo).not.toHaveBeenCalled();
    });

    it("does not fit again after a remount (closing the Git board)", () => {
      const { unmount } = render(<FloorMap />);
      act(() => resize(800, 600));
      expect(jumpTo).toHaveBeenCalledTimes(1);
      unmount();
      jumpTo.mockClear();
      render(<FloorMap />);
      act(() => resize(800, 600));
      expect(jumpTo).not.toHaveBeenCalled();
    });

    it("does not fit when the camera moved before the rooms arrived", () => {
      rooms = [];
      const { rerender } = render(<FloorMap />);
      act(() => resize(800, 600));
      act(() => useFloorStore.getState().setCamera({ x: 9, y: 9, z: 1 }));
      rooms = allRooms;
      rerender(<FloorMap />);
      expect(jumpTo).not.toHaveBeenCalled();
    });
  });
});
