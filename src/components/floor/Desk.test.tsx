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

vi.mock("../../lib/floor/menus", async (orig) => ({
  ...(await orig<typeof import("../../lib/floor/menus")>()),
  processMenu: vi.fn(() => [{ label: "Mock item", onSelect: vi.fn() }]),
}));

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

  it("flags only a working desk so other animations pause", () => {
    seed({ activity: "working" });
    const a = render(<Desk process={proc()} />);
    expect(a.getByRole("button")).toHaveAttribute("data-working");
    a.unmount();
    seed({ activity: "waiting" });
    const b = render(<Desk process={proc()} />);
    expect(b.getByRole("button")).not.toHaveAttribute("data-working");
  });

  it("dims mini bots on an idle desk, not on a working one", () => {
    seed({ activity: "waiting", subagents: 2 });
    const a = render(<Desk process={proc()} />);
    const idle = a.container.querySelector("[class*=minis]");
    expect(idle?.children).toHaveLength(2);
    expect(idle).toHaveAttribute("data-dim");
    a.unmount();
    seed({ activity: "working", subagents: 2 });
    const b = render(<Desk process={proc()} />);
    expect(b.container.querySelector("[class*=minis]")).not.toHaveAttribute(
      "data-dim",
    );
  });

  it("caps mini bots at 3 plus a +N counter", () => {
    seed({ activity: "waiting", subagents: 5 });
    const { container } = render(<Desk process={proc()} />);
    const minis = container.querySelector("[class*=minis]");
    expect(minis?.querySelectorAll(":scope > :not(b)")).toHaveLength(3);
    expect(screen.getByText("+2")).toBeTruthy();
  });

  it("marks needs with ? and crashed with !", () => {
    seed({ activity: "waiting" });
    const a = render(<Desk process={proc()} />);
    expect(a.container.textContent).toContain("?");
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

  it("puts an idle bot to sleep: grey with floating Zs, ended shows no Z", () => {
    seed({ activity: "idle" });
    const a = render(<Desk process={proc()} />);
    expect(a.container.querySelector("[class*=zzz]")?.textContent).toBe("zzZ");
    const bot = a.container.querySelector(
      "[data-state=idle] > i",
    ) as HTMLElement;
    expect(bot.style.backgroundImage).toContain(encodeURIComponent("#8b949e"));
    a.unmount();
    const b = render(
      <Desk
        process={proc({ status: { state: "exited", code: 0 } as never })}
      />,
    );
    expect(b.getByText("ended")).toBeTruthy();
    expect(b.container.querySelector("[class*=zzz]")).toBeNull();
  });

  it("draws the bot design it is given", () => {
    const imgs = [0, 1, 2].map((variant) => {
      const r = render(<Desk process={proc()} variant={variant} />);
      const img = (r.container.querySelector("[data-state] > i") as HTMLElement)
        .style.backgroundImage;
      r.unmount();
      return img;
    });
    expect(new Set(imgs).size).toBe(3);
  });

  it("renders a terminal desk without a bot", () => {
    render(<Desk process={proc({ kind: { kind: "terminal" } })} />);
    expect(screen.getByText("terminal")).toBeTruthy();
  });

  it("shows a stopped terminal grey and labelled stopped, a running one not", () => {
    const term = (status: unknown) =>
      proc({ kind: { kind: "terminal" }, status: status as never });
    const a = render(<Desk process={term({ state: "notStarted" })} />);
    expect(a.getByText("stopped")).toBeTruthy();
    expect(a.getByRole("button")).toHaveAttribute("data-stopped");
    a.unmount();
    const b = render(
      <Desk process={term({ state: "exited", code: 1, crashed: true })} />,
    );
    expect(b.container.textContent).toContain("!");
    b.unmount();
    const c = render(<Desk process={term(proc().status)} />);
    expect(c.getByText("terminal")).toBeTruthy();
    expect(c.getByRole("button")).not.toHaveAttribute("data-stopped");
    expect(c.container.textContent).not.toContain("!");
  });

  it("opens one process menu on right-click and keeps left click opening", () => {
    render(<Desk process={proc()} />);
    const btn = screen.getByRole("button", { name: /Fix login/ });
    const ev = fireEvent.contextMenu(btn, { clientX: 10, clientY: 20 });
    expect(ev).toBe(false);
    expect(screen.getByText("Mock item")).toBeTruthy();
    fireEvent.contextMenu(btn, { clientX: 30, clientY: 40 });
    expect(screen.getAllByText("Mock item")).toHaveLength(1);
    fireEvent.click(btn);
    expect(useFloorStore.getState().terminalId).toBe("p1");
  });

  it("pins and opens on click, reports hover, marks the active desk", () => {
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

  it("sets a title with the full name and state, plus sprite counts", () => {
    seed({
      activity: "working",
      sinceMs: Date.now() - 4 * 60_000,
      subagents: 3,
      shells: 1,
    });
    const a = render(<Desk process={proc({ name: "harbor-api" })} />);
    expect(a.getByRole("button")).toHaveAttribute(
      "title",
      "harbor-api, working 4m",
    );
    expect(a.getByTitle("3 sub-agents")).toBeTruthy();
    expect(a.getByTitle("1 background shell")).toBeTruthy();
    a.unmount();
    const t = render(
      <Desk
        process={proc({ name: "shell", kind: { kind: "terminal" } as never })}
      />,
    );
    expect(t.getByRole("button")).toHaveAttribute("title", "shell, terminal");
    t.unmount();
    const s2 = render(
      <Desk
        process={proc({
          name: "build",
          kind: { kind: "terminal" } as never,
          status: { state: "exited", code: 0 } as never,
        })}
      />,
    );
    expect(s2.getByRole("button")).toHaveAttribute("title", "build, stopped");
  });
});
