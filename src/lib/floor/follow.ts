import { useFloorStore } from "../../state/floorStore";
import type { Follow } from "../viewport";
import { getRooms, subscribeRooms } from "./rooms";
import type { RoomModel } from "./types";

const placeKey = (rooms: RoomModel[]) =>
  rooms
    .map(({ project, rect: r }) => `${project.id}:${r.x},${r.y},${r.w},${r.h}`)
    .join();

/**
 * `Follow` for popovers and menus on the floor: fires on each camera change,
 * and when a room moves or resizes (a push, a re-layout), so a popover stays
 * on its fixture.
 */
export const followCamera: Follow = (cb) => {
  let at = placeKey(getRooms());
  const offRooms = subscribeRooms(() => {
    const next = placeKey(getRooms());
    if (next === at) return;
    at = next;
    cb(false);
  });
  const offCamera = useFloorStore.subscribe((s, prev) => {
    if (s.camera !== prev.camera) cb(s.animate);
  });
  return () => {
    offCamera();
    offRooms();
  };
};
