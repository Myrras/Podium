import { act, fireEvent, render, screen } from "@testing-library/react";
import { Profiler } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectId } from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { waypoints } from "../../lib/floor/waypoints";
import { useFloorStore } from "../../state/floorStore";
import { FloorWaypoints } from "./FloorWaypoints";

const jumpTo = vi.hoisted(() => vi.fn());
vi.mock("../../lib/floor/camera", () => ({ jumpTo }));
const rooms = vi.hoisted(() => [
  {
    project: {
      id: "p1" as ProjectId,
      name: "Alpha",
      iconInitials: "AL",
      color: null,
    },
    rect: { x: 3000, y: 0, w: 400, h: 400 },
    needs: 2,
    crashed: 1,
  },
]);
vi.mock("../../lib/floor/rooms", () => ({ useRooms: () => rooms }));
// Counts the computations; the real function still runs.
vi.mock("../../lib/floor/waypoints", async (importOriginal) => {
  const real =
    await importOriginal<typeof import("../../lib/floor/waypoints")>();
  return { waypoints: vi.fn(real.waypoints) };
});

describe("FloorWaypoints", () => {
  beforeEach(() => {
    jumpTo.mockClear();
    useFloorStore.setState(
      { ...useFloorStore.getState(), viewport: { w: 1000, h: 600 } },
      true,
    );
  });

  it("renders a marker that jumps to the room", () => {
    render(<FloorWaypoints />);
    const btn = screen.getByRole("button", {
      name: "Go to Alpha: 2 agents waiting on you, 1 crashed",
    });
    fireEvent.click(btn);
    expect(jumpTo).toHaveBeenCalledWith("p1");
  });

  it("moves markers on a pan without re-rendering, animating only jumps", () => {
    const commits = vi.fn();
    render(
      <Profiler id="w" onRender={commits}>
        <FloorWaypoints />
      </Profiler>,
    );
    const marker = screen.getByRole("button", { name: /Go to Alpha/ });
    const at = (camera: { x: number; y: number; z: number }) => {
      const [m] = waypoints(rooms as unknown as RoomModel[], camera, {
        w: 1000,
        h: 600,
      });
      return `translate(${m.x}px, ${m.y}px)`;
    };
    expect(marker.style.transform).toBe(at({ x: 0, y: 0, z: 1 }));
    commits.mockClear();

    const pan = { x: -10, y: 120, z: 1 };
    act(() => useFloorStore.getState().setCamera(pan));
    expect(commits).not.toHaveBeenCalled();
    expect(marker.style.transform).toBe(at(pan));
    expect(marker.hasAttribute("data-animate")).toBe(false);

    act(() =>
      useFloorStore
        .getState()
        .setCamera({ x: -20, y: 0, z: 1 }, { animate: true }),
    );
    expect(marker.hasAttribute("data-animate")).toBe(true);
  });

  it("computes once per camera change and not on other floor changes", () => {
    const commits = vi.fn();
    render(
      <Profiler id="w" onRender={commits}>
        <FloorWaypoints />
      </Profiler>,
    );
    vi.mocked(waypoints).mockClear();
    commits.mockClear();

    act(() => useFloorStore.getState().setHoverDesk("a1"));
    expect(waypoints).not.toHaveBeenCalled();
    expect(commits).not.toHaveBeenCalled();

    act(() => useFloorStore.getState().setCamera({ x: -5, y: 0, z: 1 }));
    expect(waypoints).toHaveBeenCalledTimes(1);
  });
});
