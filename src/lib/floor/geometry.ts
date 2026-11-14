import type { Camera, Rect, Size } from "./types";

export const GRID = 40,
  DESK_W = 152,
  DESK_H = 172,
  WALL_H = 80,
  ZOOM_MIN = 0.3,
  ZOOM_MAX = 2,
  FAR_Z = 0.4,
  FIT_PAD = 44,
  NAMEPLATE_H = 32;

const WORLD_W = 1680;
const DESKS_PER_ROW = 4;
const PUSH_ROUNDS = 200;

// Room.module.css: 1px border x2, desks padding 8 x2, row gap 8; the wall is WALL_H.
const ROOM_CHROME = 2 + WALL_H + 16;
const ROW_GAP = 8;

/**
 * Width of each wall fixture (RoomWall.module.css `.fx`: padding 8 x2, border 2 x2, 11px label
 * with a 2-digit count), measured in WebKit. shortcut: a 3-digit count is ~7px wider; measure
 * the labels if that ever matters.
 */
export const FIXTURE_W = {
  todos: 76,
  scratchpads: 106,
  commands: 82,
  services: 84,
};
export type WallFixture = keyof typeof FIXTURE_W;

// RoomWall.module.css .wall: padding 6 x2, gap 4; Room.module.css: border 1 x2.
const wallWidth = (fixtures: WallFixture[]) =>
  fixtures.reduce((w, f) => w + FIXTURE_W[f] + 4, 2 + 12 - 4);

/** Room size; at least as wide as its wall `fixtures`, in the order they hang. */
export function roomSize(deskCount: number, fixtures: WallFixture[]): Size {
  const min = Math.max(200, Math.ceil(wallWidth(fixtures) / GRID) * GRID);
  if (deskCount <= 0) return { w: min, h: ROOM_CHROME };
  const cols = Math.min(deskCount, DESKS_PER_ROW);
  const rows = Math.ceil(deskCount / DESKS_PER_ROW);
  return {
    w: Math.max(
      min,
      Math.ceil((30 + cols * DESK_W + (cols - 1) * 8) / GRID) * GRID,
    ),
    h: ROOM_CHROME + rows * DESK_H + (rows - 1) * ROW_GAP,
  };
}

export function snap(xy: [number, number]): [number, number] {
  return [Math.round(xy[0] / GRID) * GRID, Math.round(xy[1] / GRID) * GRID];
}

/** True when rect comes within GRID of any other rect (others grown by GRID on every side). */
export function hits(rect: Rect, others: Rect[]): boolean {
  return others.some(
    (b) =>
      rect.x < b.x + b.w + GRID &&
      b.x < rect.x + rect.w + GRID &&
      rect.y < b.y + b.h + GRID &&
      b.y < rect.y + rect.h + GRID,
  );
}

export function layoutRooms(
  rooms: { key: string; size: Size; hasDesks: boolean; grown?: boolean }[],
  saved: Record<string, [number, number]>,
): Record<string, [number, number]> {
  const out: Record<string, [number, number]> = {};
  const placed: Rect[] = [];
  const put = (size: Size, x: number, y: number, key: string) => {
    out[key] = [x, y];
    placed.push({ x, y, ...size });
  };
  // Rooms with a valid saved spot start there, then push each other apart.
  const pushed = rooms.flatMap((room) => {
    const s = saved[room.key];
    return Array.isArray(s) && s.length === 2 && s.every(Number.isFinite)
      ? [{ room, rect: { x: s[0], y: s[1], ...room.size } }]
      : [];
  });
  // Every move goes only right or down, so this ends; the cap bounds big piles.
  for (let round = 0; round < PUSH_ROUNDS; round++) {
    let pair: [number, number] | null = null;
    for (let i = 0; i < pushed.length && !pair; i++)
      for (let j = i + 1; j < pushed.length && !pair; j++)
        if (hits(pushed[i].rect, [pushed[j].rect])) pair = [i, j];
    if (!pair) break;
    const [a, b] = pair.map((i) => pushed[i]);
    const sum = (r: Rect) => r.x + r.y;
    // The room that did not grow moves; else the one further right/down (tie: later in input).
    const [mover, still] =
      Boolean(a.room.grown) !== Boolean(b.room.grown)
        ? a.room.grown
          ? [b, a]
          : [a, b]
        : sum(a.rect) > sum(b.rect)
          ? [a, b]
          : [b, a];
    const m = mover.rect,
      o = still.rect;
    const x = Math.ceil((o.x + o.w + GRID) / GRID) * GRID;
    const y = Math.ceil((o.y + o.h + GRID) / GRID) * GRID;
    if (x - m.x <= y - m.y) m.x = x;
    else m.y = y;
  }
  // Rooms still overlapping after the cap fall back to first-fit below.
  for (const { room, rect } of pushed)
    if (!hits(rect, placed)) put(room.size, rect.x, rect.y, room.key);
  // Array.sort is stable: rooms with desks first, input order otherwise.
  const ordered = [...rooms].sort(
    (a, b) => Number(b.hasDesks) - Number(a.hasDesks),
  );
  for (const { key, size } of ordered) {
    if (key in out) continue;
    // Terminates: past the lowest placed room, row y is always free. A room wider than the
    // world still gets x = 0 (shortcut: no horizontal wrap for it).
    search: for (let y = 0; ; y += GRID) {
      for (let x = 0; x === 0 || x + size.w <= WORLD_W; x += GRID) {
        if (!hits({ x, y, ...size }, placed)) {
          put(size, x, y, key);
          break search;
        }
      }
    }
  }
  return out;
}

export function bounds(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null;
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const x1 = Math.max(...rects.map((r) => r.x + r.w));
  const y1 = Math.max(...rects.map((r) => r.y + r.h));
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Camera that fits box (plus the nameplate above it) with FIT_PAD around, centred. */
export function frame(box: Rect, viewport: Size, maxZ: number): Camera {
  const y = box.y - NAMEPLATE_H;
  const h = box.h + NAMEPLATE_H;
  const z = Math.max(
    ZOOM_MIN,
    Math.min(
      maxZ,
      (viewport.w - FIT_PAD * 2) / box.w,
      (viewport.h - FIT_PAD * 2) / h,
    ),
  );
  return {
    z,
    x: (viewport.w - box.w * z) / 2 - box.x * z,
    y: (viewport.h - h * z) / 2 - y * z,
  };
}

export function zoomAt(
  camera: Camera,
  px: number,
  py: number,
  factor: number,
): Camera {
  const z = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, camera.z * factor));
  const k = z / camera.z;
  return { z, x: px - (px - camera.x) * k, y: py - (py - camera.y) * k };
}

export function toScreen(rect: Rect, camera: Camera): Rect {
  return {
    x: rect.x * camera.z + camera.x,
    y: rect.y * camera.z + camera.y,
    w: rect.w * camera.z,
    h: rect.h * camera.z,
  };
}
