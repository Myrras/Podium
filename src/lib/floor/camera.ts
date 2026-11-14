import type { ProcessId, ProjectId } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { bounds, frame, zoomAt } from "./geometry";
import { getRooms, roomOf } from "./rooms";

export function jumpTo(target: "all" | ProjectId): void {
  const rooms = getRooms();
  const { viewport, setCamera } = useFloorStore.getState();
  if (target === "all") {
    const box = bounds(rooms.map((r) => r.rect));
    if (box)
      setCamera(frame(box, viewport, 1), { focus: "all", animate: true });
    return;
  }
  const room = rooms.find((r) => r.project.id === target);
  if (room)
    setCamera(frame(room.rect, viewport, 1.5), {
      focus: target,
      animate: true,
    });
}

/** Zoom around the viewport centre. */
export function zoomBy(factor: number): void {
  const { camera, viewport, setCamera } = useFloorStore.getState();
  setCamera(zoomAt(camera, viewport.w / 2, viewport.h / 2, factor));
}

/** jumpTo the agent's room and make it the active process; nothing when it has no room (removed). */
export function showAgent(processId: ProcessId): void {
  const room = roomOf(processId, getRooms());
  if (!room) return;
  jumpTo(room.project.id);
  useProcessStore.getState().setActiveProcess(processId);
}
