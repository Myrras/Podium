import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("../../lib/floor/rooms", () => ({
  deskState: () => "working",
  useRooms: () => [{ project: { id: "p1" }, agents: [{}, {}], services: [{}] }],
}));

import type { ProcessInfo, TodoInfo } from "../../ipc/types";
import {
  UNTRACKED_ACTIVITY,
  useAgentActivityStore,
} from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import { FloorStatusBar } from "./FloorStatusBar";

const agent = {
  id: "a1",
  projectId: "p1",
  name: "Agent one",
  kind: { kind: "agent", adapter: "claude-code" },
  worktree: "feat-x",
} as unknown as ProcessInfo;

const todo = (id: string, done: boolean, processId?: string) =>
  ({
    id,
    text: `Todo ${id}`,
    done,
    assignedAgent: processId ? { processId } : null,
  }) as unknown as TodoInfo;

beforeEach(() => {
  useFloorStore.getState().reset();
  useProcessStore.setState({ processes: [agent], activeProcessId: null });
  useTodoStore.setState({ todosByProject: {} });
  useScratchpadStore.setState({ scratchpadsByProject: {} });
  useAgentActivityStore.setState({ details: {} });
});

describe("FloorStatusBar", () => {
  it("shows the hint by default", () => {
    render(<FloorStatusBar />);
    expect(screen.getByRole("status").textContent).toMatch(/Drag the floor/);
  });

  it("shows room counts when one project is in focus", () => {
    useTodoStore.setState({
      todosByProject: { p1: [todo("t1", false), todo("t2", true)] },
    });
    useScratchpadStore.setState({
      scratchpadsByProject: { p1: [{} as never] },
    });
    useFloorStore.setState({ focus: "p1" });
    render(<FloorStatusBar />);
    expect(screen.getByRole("status").textContent).toBe(
      "2 agents · 1 open to-do · 1 scratchpad · 1 service",
    );
  });

  it("shows the pinned agent's details; an unknown hover falls back", () => {
    useProcessStore.setState({ activeProcessId: "a1" });
    useTodoStore.setState({
      todosByProject: { p1: [todo("t1", false, "a1")] },
    });
    useAgentActivityStore.setState({
      details: {
        a1: {
          ...UNTRACKED_ACTIVITY,
          sinceMs: Date.now() - 65_000,
          subagents: 2,
          shells: 1,
        },
      },
    });
    render(<FloorStatusBar />);
    expect(screen.getByRole("status").textContent).toBe(
      "Working for 1m 05s · Todo t1 · worktree feat-x · 2 sub-agents · 1 background shell",
    );
    act(() => useFloorStore.getState().setHoverDesk("zz"));
    expect(screen.getByRole("status").textContent).toMatch(/^Working/);
  });

  it("advances the elapsed time while shown", () => {
    vi.useFakeTimers();
    try {
      useProcessStore.setState({ activeProcessId: "a1" });
      useAgentActivityStore.setState({
        details: {
          a1: { ...UNTRACKED_ACTIVITY, sinceMs: Date.now() - 65_000 },
        },
      });
      render(<FloorStatusBar />);
      expect(screen.getByRole("status").textContent).toMatch(/for 1m 05s/);
      act(() => vi.advanceTimersByTime(10_000));
      expect(screen.getByRole("status").textContent).toMatch(/for 1m 15s/);
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows a pinned service by its pill word, not as an idle desk", () => {
    useProcessStore.setState({
      processes: [
        agent,
        {
          id: "s1",
          projectId: "p1",
          name: "web",
          kind: { kind: "service" },
          status: { state: "running" },
        } as unknown as ProcessInfo,
      ],
      activeProcessId: "s1",
    });
    render(<FloorStatusBar />);
    expect(screen.getByRole("status").textContent).toBe("Up");
  });

  it("with a room in focus, shows the pinned desk only when it is in that room", () => {
    useProcessStore.setState({ activeProcessId: "a1" });
    useFloorStore.setState({ focus: "p2" });
    render(<FloorStatusBar />);
    expect(screen.getByRole("status").textContent).toMatch(/Drag the floor/);
    act(() => useFloorStore.setState({ focus: "p1" }));
    expect(screen.getByRole("status").textContent).toMatch(/^Working/);
    act(() => useFloorStore.setState({ focus: "all" }));
    expect(screen.getByRole("status").textContent).toMatch(/^Working/);
  });
});
