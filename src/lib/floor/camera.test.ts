import { beforeEach, describe, expect, it, vi } from "vitest";

import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { jumpTo, showAgent, zoomBy } from "./camera";
import { bounds, frame } from "./geometry";
import type { RoomModel } from "./types";

const rooms = [
  {
    project: { id: "p1" },
    rect: { x: 0, y: 0, w: 200, h: 120 },
    agents: [{ id: "a1" }],
    terminals: [],
    services: [],
  },
  {
    project: { id: "p2" },
    rect: { x: 400, y: 0, w: 200, h: 120 },
    agents: [],
    terminals: [],
    services: [],
  },
] as unknown as RoomModel[];
let current = rooms;
vi.mock("./rooms", async (orig) => ({
  ...(await orig<typeof import("./rooms")>()),
  getRooms: () => current,
}));
vi.mock("../../ipc/commands", () => ({ toIpcError: vi.fn() }));
vi.mock("../terminalRegistry", () => ({ disposeTerminal: vi.fn() }));

const viewport = { w: 1000, h: 600 };
beforeEach(() => {
  current = rooms;
  useFloorStore.getState().reset();
  useFloorStore.getState().setViewport(viewport);
});

describe("camera", () => {
  it("jumpTo all frames the bounds", () => {
    jumpTo("all");
    expect(useFloorStore.getState()).toMatchObject({
      camera: frame(bounds(rooms.map((r) => r.rect))!, viewport, 1),
      focus: "all",
      animate: true,
    });
  });

  it("jumpTo a project frames its room with max zoom 1.5", () => {
    jumpTo("p2");
    expect(useFloorStore.getState()).toMatchObject({
      camera: frame(rooms[1].rect, viewport, 1.5),
      focus: "p2",
      animate: true,
    });
  });

  it("does nothing without rooms or for an unknown project", () => {
    current = [];
    jumpTo("all");
    jumpTo("p1");
    current = rooms;
    jumpTo("zzz");
    expect(useFloorStore.getState().camera).toEqual({ x: 0, y: 0, z: 1 });
    expect(useFloorStore.getState().focus).toBeNull();
  });

  it("zoomBy keeps the viewport centre fixed", () => {
    useFloorStore.getState().setCamera({ x: 100, y: 50, z: 1 });
    zoomBy(2);
    const { camera } = useFloorStore.getState();
    expect(camera.z).toBe(2);
    expect((500 - camera.x) / camera.z).toBe(400);
    expect((300 - camera.y) / camera.z).toBe(250);
  });

  it("showAgent jumps to the room and pins the agent", () => {
    showAgent("a1");
    expect(useFloorStore.getState().focus).toBe("p1");
    expect(useProcessStore.getState().activeProcessId).toBe("a1");
  });
});
