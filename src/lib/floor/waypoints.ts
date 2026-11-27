import type { ProjectId } from "../../ipc/types";
import type { Camera, RoomModel, Size } from "./types";
import { toScreen } from "./geometry";

export interface Waypoint {
  projectId: ProjectId;
  /** Marker centre in viewport px. */
  x: number;
  y: number;
  angle: number;
  needs: number;
  crashed: number;
}

const INSET = 22;
const GAP = 8;
// shortcut: marker size is estimated (no DOM here), long names may overlap a little; measure if that matters.
const MARKER_W = 170;
const MARKER_H = 32;

const clamp = (v: number, lo: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));

export function waypoints(
  rooms: RoomModel[],
  camera: Camera,
  viewport: Size,
): Waypoint[] {
  const { w: W, h: H } = viewport;
  const hx = MARKER_W / 2;
  const hy = MARKER_H / 2;
  const out: Waypoint[] = [];
  for (const room of rooms) {
    if (room.needs <= 0 && room.crashed <= 0) continue;
    const r = toScreen(room.rect, camera);
    const ox = Math.min(r.x + r.w, W) - Math.max(r.x, 0);
    const oy = Math.min(r.y + r.h, H) - Math.max(r.y, 0);
    if (ox > Math.min(140, r.w / 2) && oy > Math.min(140, r.h / 2)) continue;
    const dx = r.x + r.w / 2 - W / 2;
    const dy = r.y + r.h / 2 - H / 2;
    const k = Math.min(
      (W / 2 - INSET - hx) / (Math.abs(dx) || 1e-6),
      (H / 2 - INSET - hy) / (Math.abs(dy) || 1e-6),
    );
    const onSide = Math.abs(dx) * H > Math.abs(dy) * W;
    const loX = INSET + hx;
    const loY = INSET + hy;
    const hiX = Math.max(loX, W - INSET - hx);
    const hiY = Math.max(loY, H - INSET - hy);
    let x = clamp(W / 2 + dx * k, loX, hiX);
    let y = clamp(H / 2 + dy * k, loY, hiY);
    const overlaps = () =>
      out.some(
        (q) =>
          Math.abs(x - q.x) < MARKER_W + GAP &&
          Math.abs(y - q.y) < MARKER_H + GAP,
      );
    // Push apart along the edge the marker sits on, then stop at the far end.
    while (overlaps()) {
      if (onSide) {
        y += MARKER_H + GAP;
        if (y > hiY) break;
      } else {
        x += MARKER_W + GAP;
        if (x > hiX) break;
      }
    }
    out.push({
      projectId: room.project.id,
      x: clamp(x, loX, hiX),
      y: clamp(y, loY, hiY),
      angle: Math.atan2(dy, dx),
      needs: room.needs,
      crashed: room.crashed,
    });
  }
  return out;
}
