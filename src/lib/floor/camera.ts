// Stub (Floor slice 1); slice 6 implements it.
import type { ProcessId, ProjectId } from "../../ipc/types";

export function jumpTo(_target: "all" | ProjectId): void {}

/** Zoom around the viewport centre. */
export function zoomBy(_factor: number): void {}

/** jumpTo the agent's room and make it the active process. */
export function showAgent(_processId: ProcessId): void {}
