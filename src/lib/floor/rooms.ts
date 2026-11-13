// Stub (Floor slice 1); slice 6 implements it.
import type { ProcessId, ProcessInfo } from "../../ipc/types";
import type { ProcessActivity } from "../../state/agentActivityStore";
import type { DeskState, RoomModel } from "./types";

export function deskState(
  _process: ProcessInfo,
  _activity: ProcessActivity,
): DeskState {
  return "idle";
}

export function useRooms(): RoomModel[] {
  return [];
}

/** The same data as `useRooms`, outside React. */
export function getRooms(): RoomModel[] {
  return [];
}

export function roomOf(
  _processId: ProcessId,
  _rooms: RoomModel[],
): RoomModel | null {
  return null;
}

/** Loads to-dos and scratchpads of every project; FloorMap calls it. */
export function useFloorData(): void {}
