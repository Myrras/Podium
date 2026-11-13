import { useEffect, useMemo } from "react";
import { useShallow } from "zustand/react/shallow";

import type {
  ProcessId,
  ProcessInfo,
  ProjectId,
  ProjectInfo,
} from "../../ipc/types";
import {
  type ProcessActivity,
  UNTRACKED_ACTIVITY,
  useAgentActivityStore,
} from "../../state/agentActivityStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import { pillState } from "../processStatus";
import { layoutRooms, roomSize } from "./geometry";
import type { DeskState, RoomModel } from "./types";

export function deskState(
  process: ProcessInfo,
  activity: ProcessActivity,
): DeskState {
  switch (pillState(process.status, process.kind.kind, activity)) {
    case "needs":
      return "needs";
    case "working":
      return "working";
    case "crashed":
      return "crashed";
    case "exited":
    case "notStarted":
      return "ended";
    default:
      return "idle";
  }
}

const stateOf =
  (details: Record<string, ProcessActivity>) => (p: ProcessInfo) =>
    deskState(p, details[p.id] ?? UNTRACKED_ACTIVITY);

/** Pure room model; `state` gives each process's desk state. */
function buildRooms(
  projects: ProjectInfo[],
  processes: ProcessInfo[],
  saved: Record<string, [number, number]>,
  state: (p: ProcessInfo) => DeskState,
): RoomModel[] {
  const parts = projects.map((project) => {
    const own = processes.filter((p) => p.projectId === project.id);
    const kind = (k: string) => own.filter((p) => p.kind.kind === k);
    const agents = kind("agent");
    const terminals = project.sections.terminals ? kind("terminal") : [];
    const services = kind("service");
    const states = agents.map(state);
    return {
      project,
      agents,
      terminals,
      services,
      needs: states.filter((s) => s === "needs").length,
      crashed:
        states.filter((s) => s === "crashed").length +
        services.filter((s) => state(s) === "crashed").length,
      lit: states.some((s) => s === "working" || s === "needs"),
    };
  });
  const places = layoutRooms(
    parts.map((r) => {
      const desks = r.agents.length + r.terminals.length;
      return {
        key: r.project.root,
        size: roomSize(desks, r.services.length > 0),
        hasDesks: desks > 0,
      };
    }),
    saved,
  );
  return parts.map((r) => {
    const [x, y] = places[r.project.root];
    const size = roomSize(
      r.agents.length + r.terminals.length,
      r.services.length > 0,
    );
    return { ...r, rect: { x, y, ...size } };
  });
}

export function useRooms(): RoomModel[] {
  const projects = useProjectStore((s) => s.projects);
  const processes = useProcessStore((s) => s.processes);
  const saved = useLayoutStore((s) => s.floorPositions);
  // Activity ticks every second: depend on the desk states only.
  const states = useAgentActivityStore(
    useShallow((s) => processes.map(stateOf(s.details))),
  );
  return useMemo(() => {
    const byId = new Map(processes.map((p, i) => [p.id, states[i]]));
    return buildRooms(projects, processes, saved, (p) => byId.get(p.id)!);
  }, [projects, processes, saved, states]);
}

/** The same data as `useRooms`, outside React. */
export function getRooms(): RoomModel[] {
  return buildRooms(
    useProjectStore.getState().projects,
    useProcessStore.getState().processes,
    useLayoutStore.getState().floorPositions,
    stateOf(useAgentActivityStore.getState().details),
  );
}

export function roomOf(
  processId: ProcessId,
  rooms: RoomModel[],
): RoomModel | null {
  return (
    rooms.find((r) =>
      [r.agents, r.terminals, r.services].some((l) =>
        l.some((p) => p.id === processId),
      ),
    ) ?? null
  );
}

/** Loads to-dos and scratchpads of every project and saves room places; FloorMap calls it. */
export function useFloorData(): void {
  const ids = useProjectStore((s) => s.projects.map((p) => p.id).join("\n"));
  useEffect(() => {
    for (const id of ids ? (ids.split("\n") as ProjectId[]) : []) {
      void useTodoStore.getState().refresh(id);
      void useScratchpadStore.getState().refresh(id);
    }
  }, [ids]);

  const rooms = useRooms();
  useEffect(() => {
    const { floorPositions, setFloorPosition } = useLayoutStore.getState();
    for (const { project, rect } of rooms) {
      const at = floorPositions[project.root];
      if (!at || at[0] !== rect.x || at[1] !== rect.y)
        setFloorPosition(project.root, [rect.x, rect.y]);
    }
  }, [rooms]);
}
