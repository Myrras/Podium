import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));

import type { ProcessInfo } from "../../ipc/types";
import {
  UNTRACKED_ACTIVITY,
  useAgentActivityStore,
} from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { Desk } from "./Desk";

const proc = (over: Partial<ProcessInfo> = {}): ProcessInfo => ({
  id: "p1",
  projectId: "proj",
  name: "Fix login",
  kind: { kind: "agent", adapter: "claude" },
  status: { state: "running", pid: 1, since: "2026-01-01T00:00:00Z" },
  restartPolicy: "never",
  command: "claude",
  worktree: null,
  color: "#58a6ff",
  viewing: false,
  ...over,
});

const seed = (patch: Partial<typeof UNTRACKED_ACTIVITY>) =>
  useAgentActivityStore.setState({
    details: { p1: { ...UNTRACKED_ACTIVITY, ...patch } },
  } as never);

beforeEach(() => {
  useAgentActivityStore.setState({ details: {}, activity: {} } as never);
  useFloorStore.getState().reset();
  useProcessStore.setState({ activeProcessId: null } as never);
});

describe("Desk", () => {
  it("shows a working agent with time, capped sub-agents and shells", () => {
    seed({
      activity: "working",
      sinceMs: Date.now() - 4 * 60_000,
      subagents: 5,
      shells: 4,
    });
    const { container } = render(<Desk process={proc()} />);
    expect(screen.getByText("working 4m")).toBeTruthy();
    expect(container.querySelectorAll("b")).toHaveLength(2);
    expect(screen.getAllByText("+2")).toHaveLength(1);
    expect(screen.getByText("+1")).toBeTruthy();
  });

  it("marks needs with ? and crashed with !, without mini bots when idle", () => {
    seed({ activity: "waiting", subagents: 2 });
    const a = render(<Desk process={proc()} />);
    expect(a.container.textContent).toContain("?");
    expect(a.container.querySelector("[class*=minis]")).toBeNull();
    a.unmount();
    const b = render(
      <Desk
        process={proc({
          status: { state: "exited", code: 1, crashed: true } as never,
        })}
      />,
    );
    expect(b.container.textContent).toContain("!");
    expect(screen.getByText("crashed")).toBeTruthy();
  });

  it("renders a terminal desk without a bot", () => {
    render(<Desk process={proc({ kind: { kind: "terminal" } })} />);
    expect(screen.getByText("terminal")).toBeTruthy();
  });

  it("pins and opens on click, reports hover, outlines the active desk", () => {
    useProcessStore.setState({ activeProcessId: "p1" } as never);
    render(<Desk process={proc()} />);
    const btn = screen.getByRole("button");
    expect(btn.dataset.active).toBe("true");
    fireEvent.mouseEnter(btn);
    expect(useFloorStore.getState().hoverDesk).toBe("p1");
    fireEvent.mouseLeave(btn);
    expect(useFloorStore.getState().hoverDesk).toBeNull();
    useProcessStore.setState({ activeProcessId: null } as never);
    fireEvent.click(btn);
    expect(useProcessStore.getState().activeProcessId).toBe("p1");
    expect(useFloorStore.getState().terminalId).toBe("p1");
  });
});
