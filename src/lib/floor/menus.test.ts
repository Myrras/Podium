import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { MenuItem } from "../../components/ContextMenu";
import type {
  ProcessInfo,
  ProcessStatus,
  ScratchpadInfo,
  TodoInfo,
} from "../../ipc/types";
import { UNTRACKED_ACTIVITY } from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import { processMenu, scratchpadMenu, todoMenu } from "./menus";

const P = "proj-1";
const running: ProcessStatus = { state: "running", pid: 7, since: "now" };
const exited: ProcessStatus = {
  state: "exited",
  code: 0,
  stopped: false,
  crashed: false,
} as ProcessStatus;
const crashed = { ...exited, crashed: true } as ProcessStatus;
const notStarted: ProcessStatus = { state: "notStarted" };

const proc = (kind: string, status: ProcessStatus) =>
  ({
    id: "p1",
    projectId: P,
    name: "x",
    kind: { kind },
    status,
  }) as unknown as ProcessInfo;
const labels = (items: MenuItem[]) =>
  items
    .filter((i) => i !== "separator")
    .map((i) => (i as { label: string }).label);
const pick = (items: MenuItem[], label: string) =>
  items.find((i) => i !== "separator" && i.label === label) as Exclude<
    MenuItem,
    "separator"
  >;
const menu = (kind: string, status: ProcessStatus) =>
  processMenu(proc(kind, status), UNTRACKED_ACTIVITY);

const stopProcess = vi.fn(() => Promise.resolve());
const startProcess = vi.fn(() => Promise.resolve());
const restartProcess = vi.fn(() => Promise.resolve());
const removeProcess = vi.fn(() => Promise.resolve());
const setActiveProcess = vi.fn();
const spawnAgent = vi.fn();
const setTodoDone = vi.fn();
const setTodoArchived = vi.fn();
const setScratchpadArchived = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  useFloorStore.getState().reset();
  useProcessStore.setState({
    stopProcess,
    startProcess,
    restartProcess,
    removeProcess,
    setActiveProcess,
    spawnAgent,
  } as never);
  useTodoStore.setState({ setTodoDone, setTodoArchived } as never);
  useScratchpadStore.setState({ setScratchpadArchived } as never);
  useProjectStore.setState({ setActiveProject: vi.fn() } as never);
});

describe("processMenu", () => {
  it("agent: active offers Open and Stop", () => {
    expect(labels(menu("agent", running))).toEqual(["Open", "Stop"]);
  });
  it("agent: ended or crashed offers Resume and Clear the desk", () => {
    for (const s of [exited, crashed])
      expect(labels(menu("agent", s))).toEqual([
        "Open",
        "Resume session",
        "Clear the desk",
      ]);
    expect(pick(menu("agent", exited), "Clear the desk").danger).toBe(true);
  });
  it("agent actions call the store", () => {
    const m = menu("agent", running);
    pick(m, "Open").onSelect();
    expect(setActiveProcess).toHaveBeenCalledWith("p1");
    expect(useFloorStore.getState().terminalId).toBe("p1");
    pick(m, "Stop").onSelect();
    expect(stopProcess).toHaveBeenCalledWith("p1");
    pick(menu("agent", crashed), "Resume session").onSelect();
    expect(startProcess).toHaveBeenCalledWith("p1");
    pick(menu("agent", crashed), "Clear the desk").onSelect();
    expect(removeProcess).toHaveBeenCalledWith("p1");
  });

  it("terminal: running vs not running", () => {
    expect(labels(menu("terminal", running))).toEqual([
      "Open",
      "Stop",
      "Restart",
      "Close terminal",
    ]);
    expect(labels(menu("terminal", notStarted))).toEqual([
      "Open",
      "Start",
      "Close terminal",
    ]);
    pick(menu("terminal", notStarted), "Start").onSelect();
    expect(startProcess).toHaveBeenCalledWith("p1");
    pick(menu("terminal", running), "Restart").onSelect();
    expect(restartProcess).toHaveBeenCalledWith("p1");
  });
  it("terminal: Close stops first when running, and closes its modal", async () => {
    useFloorStore.getState().openTerminal("p1");
    pick(menu("terminal", running), "Close terminal").onSelect();
    expect(useFloorStore.getState().terminalId).toBeNull();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalledWith("p1"));
    expect(stopProcess).toHaveBeenCalledWith("p1");
    expect(stopProcess.mock.invocationCallOrder[0]).toBeLessThan(
      removeProcess.mock.invocationCallOrder[0],
    );
  });
  it("terminal: Close keeps another terminal's modal and skips Stop when not running", async () => {
    useFloorStore.getState().openTerminal("other");
    pick(menu("terminal", exited), "Close terminal").onSelect();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalled());
    expect(stopProcess).not.toHaveBeenCalled();
    expect(useFloorStore.getState().terminalId).toBe("other");
  });

  it("service: no remove", () => {
    expect(labels(menu("service", running))).toEqual([
      "Open",
      "Stop",
      "Restart",
    ]);
    expect(labels(menu("service", exited))).toEqual(["Open", "Start"]);
  });
});

const todo = (patch: Partial<TodoInfo> = {}): TodoInfo => ({
  id: "t1",
  projectId: P,
  text: "Fictitious task",
  description: null,
  done: false,
  createdAt: "2024-04-03T12:00:00Z",
  doneAt: null,
  archived: false,
  archivedAt: null,
  links: [],
  comments: [],
  assignedAgent: null,
  ...patch,
});

describe("todoMenu", () => {
  it("unassigned: Start agent spawns", () => {
    const m = todoMenu(P, todo());
    expect(labels(m)).toEqual(["Open", "Start agent", "Mark done", "Archive"]);
    pick(m, "Start agent").onSelect();
    expect(spawnAgent).toHaveBeenCalledWith(P, { todoIds: ["t1"] });
    expect(pick(m, "Archive").danger).toBe(true);
  });
  it("active agent: Open the agent; inactive falls back to Start", () => {
    const agent = {
      processId: "p9",
      active: true,
    } as TodoInfo["assignedAgent"];
    const m = todoMenu(P, todo({ assignedAgent: agent }));
    expect(labels(m)[1]).toBe("Open the agent");
    pick(m, "Open the agent").onSelect();
    expect(setActiveProcess).toHaveBeenCalledWith("p9");
    expect(useFloorStore.getState().terminalId).toBe("p9");
    const off = { processId: "p9", active: false } as TodoInfo["assignedAgent"];
    expect(labels(todoMenu(P, todo({ assignedAgent: off })))[1]).toBe(
      "Start agent",
    );
  });
  it("done flips the label and the action; Open and Archive call the stores", () => {
    pick(todoMenu(P, todo()), "Mark done").onSelect();
    expect(setTodoDone).toHaveBeenCalledWith(P, "t1", true);
    const m = todoMenu(P, todo({ done: true }));
    pick(m, "Mark not done").onSelect();
    expect(setTodoDone).toHaveBeenCalledWith(P, "t1", false);
    pick(m, "Archive").onSelect();
    expect(setTodoArchived).toHaveBeenCalledWith(P, "t1", true);
    pick(m, "Open").onSelect();
    expect(useFloorStore.getState().popover).toEqual({
      projectId: P,
      kind: "todos",
      itemId: "t1",
    });
  });
});

describe("scratchpadMenu", () => {
  const pad = {
    id: "s1",
    projectId: P,
    title: "Fictitious notes",
  } as ScratchpadInfo;
  it("has Open, Start agent on this, Archive", () => {
    const m = scratchpadMenu(P, pad);
    expect(labels(m)).toEqual(["Open", "Start agent on this", "Archive"]);
    pick(m, "Open").onSelect();
    expect(useFloorStore.getState().popover).toEqual({
      projectId: P,
      kind: "scratchpads",
      itemId: "s1",
    });
    pick(m, "Start agent on this").onSelect();
    expect(useFloorStore.getState().scratchpadAgentModal).toEqual({
      projectId: P,
      scratchpadIds: ["s1"],
      initialName: "Fictitious notes",
    });
    pick(m, "Archive").onSelect();
    expect(setScratchpadArchived).toHaveBeenCalledWith(P, "s1", true);
    expect(pick(m, "Archive").danger).toBe(true);
  });
});
