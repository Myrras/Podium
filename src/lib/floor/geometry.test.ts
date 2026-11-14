import { describe, expect, it } from "vitest";
import {
  FIT_PAD,
  FIXTURE_W,
  type WallFixture,
  NAMEPLATE_H,
  ZOOM_MAX,
  ZOOM_MIN,
  bounds,
  frame,
  hits,
  layoutRooms,
  roomSize,
  snap,
  toScreen,
  zoomAt,
} from "./geometry";

const r = (x: number, y: number, w = 200, h = 120) => ({ x, y, w, h });

describe("roomSize", () => {
  const three: WallFixture[] = ["todos", "scratchpads", "commands"];
  it("is a bare wall without desks", () => {
    expect(roomSize(0, [])).toEqual({ w: 200, h: 98 });
    expect(roomSize(0, ["services"])).toEqual({ w: 200, h: 98 });
  });
  it("sizes by columns and rows", () => {
    expect(roomSize(1, [])).toEqual({ w: 200, h: 270 });
    expect(roomSize(2, [])).toEqual({ w: 360, h: 270 });
    expect(roomSize(4, [])).toEqual({ w: 680, h: 270 });
    expect(roomSize(5, [])).toEqual({ w: 680, h: 450 });
  });
  it("fits its wall fixtures, padding and gaps included", () => {
    // 2 border + 12 padding + 76 + 106 + 82 + 2 gaps of 4 = 286, on the grid.
    expect(roomSize(1, three).w).toBe(320);
    expect(roomSize(0, [...three, "services"]).w).toBe(400);
    for (let n = 1; n <= 4; n++) {
      const fx = [...three, "services"].slice(0, n) as WallFixture[];
      const need =
        14 + fx.reduce((w, f) => w + FIXTURE_W[f], 0) + 4 * (fx.length - 1);
      expect(roomSize(1, fx).w).toBeGreaterThanOrEqual(need);
    }
    // Desks still win when they need more.
    expect(roomSize(4, three).w).toBe(680);
  });
});

describe("snap", () => {
  it("rounds to the grid", () => {
    expect(snap([19, 21])).toEqual([0, 40]);
    expect(snap([-61, 100])).toEqual([-80, 120]);
  });
});

describe("hits", () => {
  it("keeps one cell apart", () => {
    expect(hits(r(0, 0), [r(239, 0)])).toBe(true);
    expect(hits(r(0, 0), [r(240, 0)])).toBe(false); // exactly one cell apart is fine
    expect(hits(r(0, 0), [r(0, 159)])).toBe(true);
    expect(hits(r(0, 0), [r(0, 160)])).toBe(false);
    expect(hits(r(0, 0), [])).toBe(false);
  });
});

describe("layoutRooms", () => {
  const s = { w: 200, h: 120 };
  it("keeps a valid saved position", () => {
    const out = layoutRooms([{ key: "a", size: s, hasDesks: false }], {
      a: [400, 80],
    });
    expect(out.a).toEqual([400, 80]);
  });
  it("pushes a colliding saved position aside and replaces invalid ones", () => {
    const out = layoutRooms(
      [
        { key: "a", size: s, hasDesks: true },
        { key: "b", size: s, hasDesks: true },
        { key: "c", size: s, hasDesks: true },
      ],
      { a: [0, 0], b: [40, 40], c: [Number.NaN, 0] },
    );
    expect(out.a).toEqual([0, 0]);
    expect(out.b).toEqual([40, 160]); // down is the shorter way out
    expect(out.c).toEqual([240, 0]);
  });
  const tall = { w: 200, h: 400 };
  it("keeps the room that grew and pushes its neighbour right", () => {
    const out = layoutRooms(
      [
        { key: "a", size: { w: 360, h: 400 }, hasDesks: true, grown: true },
        { key: "b", size: tall, hasDesks: true },
      ],
      { a: [0, 0], b: [240, 0] },
    );
    expect(out.a).toEqual([0, 0]);
    expect(out.b).toEqual([400, 0]);
  });
  it("cascades a push into the next room", () => {
    const out = layoutRooms(
      [
        { key: "a", size: { w: 360, h: 400 }, hasDesks: true, grown: true },
        { key: "b", size: tall, hasDesks: true },
        { key: "c", size: tall, hasDesks: true },
      ],
      { a: [0, 0], b: [240, 0], c: [480, 0] },
    );
    expect(out).toEqual({ a: [0, 0], b: [400, 0], c: [640, 0] });
  });
  it("pushes the room below down when a room grows in height", () => {
    const out = layoutRooms(
      [
        { key: "a", size: { w: 200, h: 640 }, hasDesks: true, grown: true },
        { key: "b", size: tall, hasDesks: true },
      ],
      { a: [0, 0], b: [0, 480] },
    );
    expect(out).toEqual({ a: [0, 0], b: [0, 680] });
  });
  it("moves the right/lower room when neither grew", () => {
    const rooms = [
      { key: "a", size: s, hasDesks: true },
      { key: "b", size: s, hasDesks: true },
    ];
    expect(layoutRooms(rooms, { a: [200, 0], b: [0, 0] })).toEqual({
      a: [240, 0],
      b: [0, 0],
    });
    // Tie: the later one moves.
    expect(layoutRooms(rooms, { a: [0, 40], b: [40, 0] })).toEqual({
      a: [0, 40],
      b: [240, 0],
    });
  });
  it("first-fits rooms still overlapping after the push cap", () => {
    // 40 rooms on one spot need more than the 200 push rounds (checked by hand).
    const pile = Array.from({ length: 40 }, (_, i) => ({
      key: `r${i}`,
      size: s,
      hasDesks: true,
    }));
    const out = layoutRooms(
      pile,
      Object.fromEntries(pile.map((p) => [p.key, [0, 0]])),
    );
    const rects = pile.map((p) => ({
      x: out[p.key][0],
      y: out[p.key][1],
      ...s,
    }));
    expect(out.r0).toEqual([0, 0]);
    rects.forEach((a, i) => {
      expect(a.x % 40 === 0 && a.y % 40 === 0).toBe(true);
      expect(hits(a, rects.slice(i + 1))).toBe(false);
    });
  });
  it("places rooms with desks first, wraps rows and never gives up", () => {
    const rooms = [
      { key: "empty", size: s, hasDesks: false },
      { key: "busy", size: s, hasDesks: true },
    ];
    expect(layoutRooms(rooms, {}).busy).toEqual([0, 0]);
    const wide = { w: 1600, h: 120 };
    const out = layoutRooms(
      [
        { key: "a", size: wide, hasDesks: true },
        { key: "b", size: wide, hasDesks: true },
        { key: "huge", size: { w: 2400, h: 120 }, hasDesks: true },
      ],
      {},
    );
    expect(out.a).toEqual([0, 0]);
    expect(out.b).toEqual([0, 160]);
    expect(out.huge).toEqual([0, 320]);
  });
});

describe("bounds", () => {
  it("is null when empty, else the union", () => {
    expect(bounds([])).toBeNull();
    expect(bounds([r(0, 0), r(300, 200, 100, 100)])).toEqual({
      x: 0,
      y: 0,
      w: 400,
      h: 300,
    });
  });
});

describe("frame", () => {
  it("fits the box with padding and the nameplate, centred", () => {
    const box = r(0, 0, 400, 300);
    const cam = frame(box, { w: 1000, h: 800 }, 2);
    const h = 300 + NAMEPLATE_H;
    expect(cam.z).toBeCloseTo(
      Math.min((1000 - 2 * FIT_PAD) / 400, (800 - 2 * FIT_PAD) / h, 2),
    );
    const s = toScreen({ ...box, y: -NAMEPLATE_H, h }, cam);
    expect(s.x + s.w / 2).toBeCloseTo(500);
    expect(s.y + s.h / 2).toBeCloseTo(400);
    expect(s.y).toBeGreaterThanOrEqual(FIT_PAD - 1e-6);
  });
  it("clamps z", () => {
    expect(frame(r(0, 0, 100, 100), { w: 2000, h: 2000 }, 1).z).toBe(1);
    expect(frame(r(0, 0, 100000, 100), { w: 500, h: 500 }, 1).z).toBe(ZOOM_MIN);
  });
});

describe("zoomAt", () => {
  it("keeps the point under the cursor fixed", () => {
    const cam = { x: 10, y: 20, z: 1 };
    const next = zoomAt(cam, 300, 200, 1.5);
    expect(next.z).toBe(1.5);
    expect((300 - next.x) / next.z).toBeCloseTo((300 - cam.x) / cam.z);
    expect((200 - next.y) / next.z).toBeCloseTo((200 - cam.y) / cam.z);
  });
  it("applies the clamped factor", () => {
    const next = zoomAt({ x: 0, y: 0, z: 1.8 }, 100, 100, 2);
    expect(next.z).toBe(ZOOM_MAX);
    const k = ZOOM_MAX / 1.8;
    expect(next.x).toBeCloseTo(100 - 100 * k);
    expect(zoomAt({ x: 0, y: 0, z: 0.35 }, 0, 0, 0.1).z).toBe(ZOOM_MIN);
  });
});

describe("toScreen", () => {
  it("scales and offsets", () => {
    expect(toScreen(r(10, 20, 100, 50), { x: 5, y: -5, z: 2 })).toEqual({
      x: 25,
      y: 35,
      w: 200,
      h: 100,
    });
  });
});
