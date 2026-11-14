import { useEffect, useSyncExternalStore } from "react";

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
import { useWorktreeStore } from "../../state/worktreeStore";
import { pillState } from "../processStatus";
import { layoutRooms, roomSize, type WallFixture } from "./geometry";
import type { DeskState, RoomModel, Size } from "./types";

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

/** Last size per room key, and the keys that grew at the latest size change of any room. */
const lastSize = new Map<string, Size>();
let grownKeys = new Set<string>();

/** Kept until the next size change so every caller agrees; cleared then so an old growth never wins a later push. */
function grown(sizes: { key: string; size: Size }[]): Set<string> {
  const changed = sizes.filter(({ key, size }) => {
    const last = lastSize.get(key);
    return !last || last.w !== size.w || last.h !== size.h;
  });
  if (changed.length === 0) return grownKeys;
  grownKeys = new Set(
    changed
      .filter(({ key, size }) => {
        const last = lastSize.get(key);
        return !!last && (size.w > last.w || size.h > last.h);
      })
      .map(({ key }) => key),
  );
  for (const { key, size } of changed) lastSize.set(key, size);
  return grownKeys;
}

/** Room model; `state` gives each process's desk state. */
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
    // The fixtures RoomWall hangs, so the room is wide enough for them.
    const fixtures = (
      [
        project.sections.todos && "todos",
        project.sections.scratchpads && "scratchpads",
        project.sections.terminals && "commands",
        services.length > 0 && "services",
      ] as const
    ).filter((f): f is WallFixture => f !== false);
    return {
      project,
      agents,
      terminals,
      services,
      size: roomSize(agents.length + terminals.length, fixtures),
      needs: states.filter((s) => s === "needs").length,
      crashed:
        states.filter((s) => s === "crashed").length +
        services.filter((s) => state(s) === "crashed").length,
      lit: states.some((s) => s === "working" || s === "needs"),
    };
  });
  const items = parts.map((r) => ({
    key: r.project.root,
    size: r.size,
    hasDesks: r.agents.length + r.terminals.length > 0,
  }));
  const grew = grown(items);
  const places = layoutRooms(
    items.map((i) => ({ ...i, grown: grew.has(i.key) })),
    saved,
  );
  return parts.map(({ size, ...r }) => {
    const [x, y] = places[r.project.root];
    return { ...r, rect: { x, y, ...size } };
  });
}

const sameList = (a: ProcessInfo[], b: ProcessInfo[]) =>
  a.length === b.length && a.every((p, i) => p === b[i]);

const sameRoom = (a: RoomModel, b: RoomModel) =>
  a.project === b.project &&
  a.needs === b.needs &&
  a.crashed === b.crashed &&
  a.lit === b.lit &&
  a.rect.x === b.rect.x &&
  a.rect.y === b.rect.y &&
  a.rect.w === b.rect.w &&
  a.rect.h === b.rect.h &&
  sameList(a.agents, b.agents) &&
  sameList(a.terminals, b.terminals) &&
  sameList(a.services, b.services);

/** The last inputs and result; every caller shares it, so the layout runs once per change. */
let cache: {
  projects: ProjectInfo[];
  processes: ProcessInfo[];
  saved: Record<string, [number, number]>;
  states: string;
  rooms: RoomModel[];
} | null = null;

/** The rooms of the current stores, outside React; unchanged inputs return the same array. */
export function getRooms(): RoomModel[] {
  const projects = useProjectStore.getState().projects;
  const processes = useProcessStore.getState().processes;
  const saved = useLayoutStore.getState().floorPositions;
  // Activity ticks every second: depend on the desk states only.
  const list = processes.map(stateOf(useAgentActivityStore.getState().details));
  const states = list.join();
  if (
    cache?.projects === projects &&
    cache.processes === processes &&
    cache.saved === saved &&
    cache.states === states
  )
    return cache.rooms;
  const byId = new Map(processes.map((p, i) => [p.id, list[i]]));
  const prev = new Map(cache?.rooms.map((r) => [r.project.id, r]));
  // Keep an unchanged room's object, so memo(Room) skips it.
  const rooms = buildRooms(projects, processes, saved, (p) =>
    byId.get(p.id)!,
  ).map((r) => {
    const old = prev.get(r.project.id);
    return old && sameRoom(old, r) ? old : r;
  });
  const same =
    cache?.rooms.length === rooms.length &&
    rooms.every((r, i) => r === cache!.rooms[i]);
  cache = {
    projects,
    processes,
    saved,
    states,
    rooms: same ? cache!.rooms : rooms,
  };
  return cache.rooms;
}

const subscribe = (onChange: () => void) => {
  const off = [
    useProjectStore.subscribe(onChange),
    useProcessStore.subscribe(onChange),
    useLayoutStore.subscribe(onChange),
    useAgentActivityStore.subscribe(onChange),
  ];
  return () => off.forEach((f) => f());
};

/** `getRooms` in React; re-renders only when the rooms change. */
export function useRooms(): RoomModel[] {
  return useSyncExternalStore(subscribe, getRooms);
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

/** Loads to-dos, scratchpads and worktrees of every project and saves room places; FloorMap calls it. */
export function useFloorData(): void {
  const ids = useProjectStore((s) => s.projects.map((p) => p.id).join("\n"));
  useEffect(() => {
    for (const id of ids ? (ids.split("\n") as ProjectId[]) : []) {
      void useTodoStore.getState().refresh(id);
      void useScratchpadStore.getState().refresh(id);
      void useWorktreeStore.getState().refresh(id);
    }
  }, [ids]);

  const rooms = useRooms();
  useEffect(() => {
    const { floorPositions, setFloorPositions } = useLayoutStore.getState();
    const moved: Record<string, [number, number]> = {};
    for (const { project, rect } of rooms) {
      const at = floorPositions[project.root];
      if (!at || at[0] !== rect.x || at[1] !== rect.y)
        moved[project.root] = [rect.x, rect.y];
    }
    if (Object.keys(moved).length > 0) setFloorPositions(moved);
  }, [rooms]);
}
