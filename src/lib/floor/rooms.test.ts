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
import { deskState, getRooms, roomOf, useFloorData, useRooms } from "./rooms";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/commands", () => ({ toIpcError: vi.fn() }));
vi.mock("../terminalRegistry", () => ({ disposeTerminal: vi.fn() }));

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
    expect(r1).toMatchObject({ needs: 1, crashed: 1, lit: true });
    expect(r2.terminals).toEqual([]);
    expect(r2).toMatchObject({ needs: 0, crashed: 0, lit: false });
    expect(r1.rect.x % 40).toBe(0);
  });

  it("never writes positions and finds a room by process", () => {
    seed();
    const rooms = getRooms();
    expect(useLayoutStore.getState().floorPositions).toEqual({});
    expect(roomOf("t1", rooms)?.project.id).toBe("p1");
    expect(roomOf("nope", rooms)).toBeNull();
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
  });
});

describe("useFloorData", () => {
  it("refreshes every project and saves places once", () => {
    const todo = vi.fn().mockResolvedValue(undefined);
    const pad = vi.fn().mockResolvedValue(undefined);
    useTodoStore.setState({ refresh: todo });
    useScratchpadStore.setState({ refresh: pad });
    useProjectStore.setState({ projects: [project("p1"), project("p2")] });
    renderHook(() => useFloorData());
    expect(todo).toHaveBeenCalledTimes(2);
    expect(pad).toHaveBeenCalledWith("p2");
    const saved = useLayoutStore.getState().floorPositions;
    expect(Object.keys(saved).sort()).toEqual(["/fake/p1", "/fake/p2"]);
    expect(saved["/fake/p1"]).toEqual([
      getRooms()[0].rect.x,
      getRooms()[0].rect.y,
    ]);
  });
});
