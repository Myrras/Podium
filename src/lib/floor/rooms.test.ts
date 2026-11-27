import { renderHook } from "@testing-library/react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProcessInfo, ProjectInfo } from "../../ipc/types";
import { useAgentActivityStore } from "../../state/agentActivityStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import { useWorktreeStore } from "../../state/worktreeStore";
import { pillState } from "../processStatus";
import {
  deskState,
  getRooms,
  roomOf,
  subscribeRooms,
  useFloorData,
  useRooms,
} from "./rooms";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/commands", () => ({ toIpcError: vi.fn() }));
vi.mock("../terminalRegistry", () => ({ disposeTerminal: vi.fn() }));
// Counts desk-state work; behaves as the real one.
vi.mock("../processStatus", async (importOriginal) => {
  const real = await importOriginal<typeof import("../processStatus")>();
  return { ...real, pillState: vi.fn(real.pillState) };
});

const project = (id: string, over: Partial<ProjectInfo> = {}): ProjectInfo =>
  ({
    id,
    name: id,
    root: `/fake/${id}`,
    sections: {
      todos: true,
      terminals: true,
      worktrees: true,
      scratchpads: true,
      issues: true,
    },
    ...over,
  }) as ProjectInfo;

const proc = (
  id: string,
  projectId: string,
  kind: "agent" | "terminal" | "service",
  status: ProcessInfo["status"] = { state: "running", pid: 1, since: "x" },
): ProcessInfo =>
  ({
    id,
    projectId,
    kind: kind === "agent" ? { kind, adapter: "claude" } : { kind },
    status,
  }) as ProcessInfo;

const act1 = (activity: "working" | "waiting" | "idle") => ({
  activity,
  reported: true,
  waitingFor: null,
  sinceMs: null,
  subagents: 0,
  shells: 0,
});

beforeEach(() => {
  useProjectStore.setState({ projects: [] });
  useProcessStore.setState({ processes: [] });
  useAgentActivityStore.setState({ details: {} });
  useLayoutStore.setState({ floorPositions: {} });
});

describe("deskState", () => {
  it("maps pill states", () => {
    const run = proc("a", "p", "agent");
    expect(deskState(run, act1("waiting"))).toBe("needs");
    expect(deskState(run, act1("working"))).toBe("working");
    expect(deskState(run, act1("idle"))).toBe("idle");
    const exited = (crashed: boolean) =>
      proc("a", "p", "agent", {
        state: "exited",
        code: 1,
        crashed,
        stopped: false,
        at: "x",
      });
    expect(deskState(exited(true), act1("idle"))).toBe("crashed");
    expect(deskState(exited(false), act1("idle"))).toBe("ended");
    expect(
      deskState(proc("a", "p", "agent", { state: "notStarted" }), act1("idle")),
    ).toBe("ended");
  });
});

describe("rooms", () => {
  const seed = () => {
    useProjectStore.setState({
      projects: [
        project("p1"),
        project("p2", { sections: { terminals: false } as never }),
      ],
    });
    useProcessStore.setState({
      processes: [
        proc("a1", "p1", "agent"),
        proc("a2", "p1", "agent"),
        proc("t1", "p1", "terminal"),
        proc("s1", "p1", "service", {
          state: "exited",
          code: 1,
          crashed: true,
          stopped: false,
          at: "x",
        }),
        proc("t2", "p2", "terminal"),
      ],
    });
    useAgentActivityStore.setState({
      details: { a1: act1("waiting"), a2: act1("working") },
    });
  };

  it("groups processes per project", () => {
    seed();
    const [r1, r2] = getRooms();
    expect(r1.agents.map((p) => p.id)).toEqual(["a1", "a2"]);
    expect(r1.terminals).toHaveLength(1);
    expect(r1.services).toHaveLength(1);
    expect(r1).toMatchObject({ needs: 1, working: 1, crashed: 1, lit: true });
    expect(r2.terminals).toEqual([]);
    expect(r2).toMatchObject({ needs: 0, working: 0, crashed: 0, lit: false });
    expect(r1.rect.x % 40).toBe(0);
  });

  it("gives each agent in a room its own bot design", () => {
    seed();
    const [r1, r2] = getRooms();
    expect(Object.keys(r1.variants).sort()).toEqual(["a1", "a2"]);
    expect(r1.variants.a1).not.toBe(r1.variants.a2);
    expect(r2.variants).toEqual({});
  });

  it("is wide enough for the wall fixtures it shows", () => {
    useProjectStore.setState({
      projects: [
        project("w1"),
        project("w2", {
          sections: { todos: true, scratchpads: false, terminals: false },
        } as never),
      ],
    });
    useProcessStore.setState({
      processes: [
        proc("wa", "w1", "agent"),
        proc("ws", "w1", "service"),
        proc("wb", "w2", "agent"),
      ],
    });
    const [all, todosOnly] = getRooms();
    // To-dos, Scratchpads, Commands and Services: 374 px of wall.
    expect(all.rect.w).toBe(400);
    expect(todosOnly.rect.w).toBe(200);
  });

  it("never writes positions and finds a room by process", () => {
    seed();
    const rooms = getRooms();
    expect(useLayoutStore.getState().floorPositions).toEqual({});
    expect(roomOf("t1", rooms)?.project.id).toBe("p1");
    expect(roomOf("nope", rooms)).toBeNull();
  });

  it("pushes the neighbour aside when a room outgrows its spot", () => {
    useProjectStore.setState({ projects: [project("ga"), project("gb")] });
    useLayoutStore.setState({
      floorPositions: { "/fake/ga": [0, 0], "/fake/gb": [360, 0] },
    });
    const a1 = proc("ga1", "ga", "agent");
    const b1 = proc("gb1", "gb", "agent");
    useProcessStore.setState({ processes: [a1, b1] });
    expect(getRooms().map((r) => [r.rect.x, r.rect.y])).toEqual([
      [0, 0],
      [360, 0],
    ]);
    useProcessStore.setState({
      processes: [a1, b1, proc("ga2", "ga", "agent")],
    });
    // Every later call agrees, not only the first one after the change.
    for (let i = 0; i < 2; i++) {
      const [a, b] = getRooms();
      expect(a.rect).toMatchObject({ x: 0, y: 0, w: 360 });
      expect(b.rect).toMatchObject({ x: 400, y: 0 });
    }
  });

  it("forgets an older growth when another room grows", () => {
    useProjectStore.setState({ projects: [project("ha"), project("hb")] });
    useLayoutStore.setState({
      floorPositions: { "/fake/ha": [360, 0], "/fake/hb": [0, 280] },
    });
    const a = (n: number) => proc(`ha${n}`, "ha", "agent");
    const b = (n: number) => proc(`hb${n}`, "hb", "agent");
    useProcessStore.setState({ processes: [a(1), b(1)] });
    getRooms();
    useProcessStore.setState({ processes: [a(1), a(2), b(1)] });
    getRooms(); // ha grew; it hits nothing
    useProcessStore.setState({ processes: [a(1), a(2), b(1), b(2), b(3)] });
    // hb grew into ha: ha moves even though hb lies further right/down.
    const [ha, hb] = getRooms();
    expect(hb.rect).toMatchObject({ x: 0, y: 280, w: 520 });
    expect(ha.rect).toMatchObject({ x: 560, y: 0 });
  });

  it("useRooms keeps its identity while only activity time ticks", () => {
    seed();
    const { result } = renderHook(() => useRooms());
    const first = result.current;
    act(() =>
      useAgentActivityStore.setState({
        details: {
          a1: { ...act1("waiting"), sinceMs: 5 },
          a2: act1("working"),
        },
      }),
    );
    expect(result.current).toBe(first);
    act(() =>
      useAgentActivityStore.setState({ details: { a2: act1("working") } }),
    );
    expect(result.current).not.toBe(first);
    expect(result.current[0].needs).toBe(0);
    expect(result.current[0].working).toBe(1);
    const second = result.current;
    act(() => useAgentActivityStore.setState({ details: {} }));
    expect(result.current).not.toBe(second);
    expect(result.current[0].working).toBe(0);
  });
});

describe("rooms cache", () => {
  it("returns the same array for the same inputs and does not advance growth", () => {
    useProjectStore.setState({ projects: [project("ca"), project("cb")] });
    useLayoutStore.setState({
      floorPositions: { "/fake/ca": [0, 0], "/fake/cb": [360, 0] },
    });
    const a1 = proc("ca1", "ca", "agent");
    const b1 = proc("cb1", "cb", "agent");
    useProcessStore.setState({ processes: [a1, b1] });
    const first = getRooms();
    expect(getRooms()).toBe(first);
    useProcessStore.setState({
      processes: [a1, b1, proc("ca2", "ca", "agent")],
    });
    const grown = getRooms();
    expect(grown[1].rect.x).toBe(400);
    // Repeated calls and an activity tick hit the cache: the push stays.
    useAgentActivityStore.setState({ details: {} });
    for (let i = 0; i < 3; i++) expect(getRooms()).toBe(grown);
  });

  it("keeps an unchanged room's object when another room changes", () => {
    seedTwo();
    const [r1, r2] = getRooms();
    useAgentActivityStore.setState({ details: { x1: act1("waiting") } });
    const [n1, n2] = getRooms();
    expect(n1).not.toBe(r1);
    expect(n1.needs).toBe(1);
    expect(n2).toBe(r2);
  });

  it("shares one result between useRooms callers and getRooms", () => {
    seedTwo();
    const one = renderHook(() => useRooms());
    const two = renderHook(() => useRooms());
    expect(one.result.current).toBe(two.result.current);
    expect(getRooms()).toBe(one.result.current);
  });
});

function seedTwo() {
  useProjectStore.setState({ projects: [project("x"), project("y")] });
  useProcessStore.setState({
    processes: [proc("x1", "x", "agent"), proc("y1", "y", "agent")],
  });
}

describe("useFloorData", () => {
  it("refreshes every project and saves places once", () => {
    const todo = vi.fn().mockResolvedValue(undefined);
    const pad = vi.fn().mockResolvedValue(undefined);
    useTodoStore.setState({ refresh: todo });
    useScratchpadStore.setState({ refresh: pad });
    const trees = vi.fn().mockResolvedValue(undefined);
    useWorktreeStore.setState({ refresh: trees });
    useProjectStore.setState({ projects: [project("p1"), project("p2")] });
    const setFloorPositions = useLayoutStore.getState().setFloorPositions;
    const write = vi.fn(setFloorPositions);
    useLayoutStore.setState({ setFloorPositions: write });
    renderHook(() => useFloorData());
    expect(write).toHaveBeenCalledTimes(1);
    useLayoutStore.setState({ setFloorPositions });
    expect(todo).toHaveBeenCalledTimes(2);
    expect(pad).toHaveBeenCalledWith("p2");
    expect(trees.mock.calls).toEqual([["p1"], ["p2"]]);
    const saved = useLayoutStore.getState().floorPositions;
    expect(Object.keys(saved).sort()).toEqual(["/fake/p1", "/fake/p2"]);
    expect(saved["/fake/p1"]).toEqual([
      getRooms()[0].rect.x,
      getRooms()[0].rect.y,
    ]);
  });

  it("refreshes only an added project, and only the lists it shows", () => {
    const todo = vi.fn().mockResolvedValue(undefined);
    const pad = vi.fn().mockResolvedValue(undefined);
    const trees = vi.fn().mockResolvedValue(undefined);
    useTodoStore.setState({ refresh: todo });
    useScratchpadStore.setState({ refresh: pad });
    useWorktreeStore.setState({ refresh: trees });
    useProjectStore.setState({ projects: [project("p1")] });
    renderHook(() => useFloorData());
    vi.clearAllMocks();

    const sections = {
      todos: true,
      terminals: true,
      worktrees: false,
      scratchpads: false,
      issues: true,
    };
    act(() =>
      useProjectStore.setState({
        projects: [project("p1"), project("p2", { sections })],
      }),
    );
    expect(todo.mock.calls).toEqual([["p2"]]);
    expect(pad).not.toHaveBeenCalled();
    expect(trees).not.toHaveBeenCalled();

    // Turning a section on loads just that list.
    act(() =>
      useProjectStore.setState({
        projects: [
          project("p1"),
          project("p2", { sections: { ...sections, worktrees: true } }),
        ],
      }),
    );
    expect(trees.mock.calls).toEqual([["p2"]]);
    expect(todo).toHaveBeenCalledTimes(1);
  });
});

describe("desk-state work", () => {
  beforeEach(() => {
    useProjectStore.setState({ projects: [project("p")] });
    useProcessStore.setState({
      processes: [proc("a1", "p", "agent"), proc("a2", "p", "agent")],
    });
  });

  it("maps the desk states once per store change, not once per subscriber", () => {
    for (let i = 0; i < 3; i++) renderHook(() => useRooms());
    vi.mocked(pillState).mockClear();
    act(() =>
      useAgentActivityStore.setState({ details: { a1: act1("working") } }),
    );
    expect(pillState).toHaveBeenCalledTimes(2);
  });

  it("ignores a layout store change outside floorPositions", () => {
    renderHook(() => useRooms());
    const onChange = vi.fn();
    const off = subscribeRooms(onChange);
    vi.mocked(pillState).mockClear();
    act(() => useLayoutStore.setState({ mode: "floor" }));
    expect(onChange).not.toHaveBeenCalled();
    expect(pillState).not.toHaveBeenCalled();
    act(() => useLayoutStore.getState().setFloorPosition("/fake/p", [400, 0]));
    expect(onChange).toHaveBeenCalledTimes(1);
    off();
  });
});
