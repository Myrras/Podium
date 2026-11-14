import { useFloorStore } from "../../state/floorStore";
import type { Follow } from "../viewport";

/** `Follow` for popovers and menus on the floor: fires on each camera change. */
export const followCamera: Follow = (cb) =>
  useFloorStore.subscribe((s, prev) => {
    if (s.camera !== prev.camera) cb(s.animate);
  });
