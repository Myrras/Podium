import { describe, expect, it } from "vitest";
import type { ProjectId } from "../../ipc/types";
import type { RoomModel } from "./types";
import { waypoints } from "./waypoints";

const room = (id: string, x: number, y: number, needs = 1, crashed = 0) =>
  ({
    project: { id: id as ProjectId },
    rect: { x, y, w: 400, h: 400 },
    needs,
    crashed,
  }) as unknown as RoomModel;
const cam = { x: 0, y: 0, z: 1 };
const vp = { w: 1000, h: 600 };

describe("waypoints", () => {
  it("skips quiet rooms and rooms in view", () => {
    expect(waypoints([room("a", 2000, 0, 0, 0)], cam, vp)).toEqual([]);
    expect(waypoints([room("a", 100, 100)], cam, vp)).toEqual([]);
  });

  it("marks an out-of-view room on the edge, pointing at it", () => {
    const [m] = waypoints([room("a", 2000, 100, 2, 1)], cam, vp);
    expect(m).toMatchObject({ projectId: "a", needs: 2, crashed: 1 });
    expect(m.x).toBeGreaterThan(vp.w / 2);
    expect(m.x).toBeLessThanOrEqual(vp.w - 22);
    expect(Math.abs(m.angle)).toBeLessThan(Math.PI / 2);
  });

  it("keeps a barely visible room as a marker", () => {
    expect(waypoints([room("a", 950, 100)], cam, vp)).toHaveLength(1);
  });

  it("pushes overlapping markers apart", () => {
    const [a, b] = waypoints(
      [room("a", 2000, 100), room("b", 2000, 120)],
      cam,
      vp,
    );
    expect(Math.abs(a.y - b.y)).toBeGreaterThanOrEqual(32);
  });
});
