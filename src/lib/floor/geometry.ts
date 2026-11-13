import type { Camera, Rect, Size } from "./types";

export const GRID = 40,
  DESK_W = 152,
  DESK_ROW_H = 240,
  WALL_H = 80,
  ZOOM_MIN = 0.3,
  ZOOM_MAX = 2,
  FAR_Z = 0.4,
  FIT_PAD = 44,
  NAMEPLATE_H = 32;

const WORLD_W = 1680;
const DESKS_PER_ROW = 4;

export function roomSize(deskCount: number, hasServices: boolean): Size {
  const min = hasServices ? 280 : 200;
  if (deskCount <= 0) return { w: min, h: 120 };
  const cols = Math.min(deskCount, DESKS_PER_ROW);
  const rows = Math.ceil(deskCount / DESKS_PER_ROW);
  return {
    w: Math.max(
      min,
      Math.ceil((30 + cols * DESK_W + (cols - 1) * 8) / GRID) * GRID,
    ),
    h: 160 + rows * DESK_ROW_H,
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
  rooms: { key: string; size: Size; hasDesks: boolean }[],
  saved: Record<string, [number, number]>,
): Record<string, [number, number]> {
  const out: Record<string, [number, number]> = {};
  const placed: Rect[] = [];
  const put = (size: Size, x: number, y: number, key: string) => {
    out[key] = [x, y];
    placed.push({ x, y, ...size });
  };
  // Array.sort is stable: rooms with desks first, input order otherwise.
  const ordered = [...rooms].sort(
    (a, b) => Number(b.hasDesks) - Number(a.hasDesks),
  );
  for (const { key, size, hasDesks: _ } of ordered) {
    const s = saved[key];
    if (
      Array.isArray(s) &&
      s.length === 2 &&
      s.every(Number.isFinite) &&
      !hits({ x: s[0], y: s[1], ...size }, placed)
    ) {
      put(size, s[0], s[1], key);
      continue;
    }
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
