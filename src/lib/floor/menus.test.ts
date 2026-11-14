import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));
const toastError = vi.hoisted(() => vi.fn());
vi.mock("../../state/toastStore", () => ({ toastError }));

import { invoke } from "@tauri-apps/api/core";

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
import {
  openProcess,
  processMenu,
  scratchpadMenu,
  todoAgentAction,
  todoMenu,
} from "./menus";

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
// The store holds the process as the menu saw it; a test may change it after.
const menu = (kind: string, status: ProcessStatus) => {
  setStatus(kind, status);
  return processMenu(proc(kind, status), UNTRACKED_ACTIVITY);
};
const setStatus = (kind: string, status: ProcessStatus) =>
  useProcessStore.setState({ processes: [proc(kind, status)] });
const stopCalls = () =>
  vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "process_stop");
// Close's stop goes through the IPC; it must come before the remove.
const stoppedBeforeRemove = () =>
  vi.mocked(invoke).mock.invocationCallOrder[
    vi.mocked(invoke).mock.calls.findIndex(([cmd]) => cmd === "process_stop")
  ] < removeProcess.mock.invocationCallOrder[0];

const stopProcess = vi.fn(() => Promise.resolve());
const startProcess = vi.fn(() => Promise.resolve());
const restartProcess = vi.fn(() => Promise.resolve());
const removeProcess = vi.fn(() => Promise.resolve(true));
const setActiveProcess = vi.fn();
const spawnAgent = vi.fn();
const setTodoDone = vi.fn();
const setTodoArchived = vi.fn();
const setScratchpadArchived = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockImplementation(() => Promise.resolve([]));
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
  it("agent: active offers Open, Stop, then Close agent", () => {
    const m = menu("agent", running);
    expect(m.map((i) => (i === "separator" ? "-" : i.label))).toEqual([
      "Open",
      "Stop",
      "-",
      "Close agent",
    ]);
    expect(pick(m, "Close agent").danger).toBe(true);
  });
  it("agent: ended, crashed or never started offers Resume and Close agent", () => {
    for (const s of [exited, crashed, notStarted])
      expect(labels(menu("agent", s))).toEqual([
        "Open",
        "Resume",
        "Close agent",
      ]);
  });
  it("agent actions call the store", () => {
    const m = menu("agent", running);
    pick(m, "Open").onSelect();
    expect(setActiveProcess).toHaveBeenCalledWith("p1");
    expect(useFloorStore.getState().terminalId).toBe("p1");
    pick(m, "Stop").onSelect();
    expect(stopProcess).toHaveBeenCalledWith("p1");
    pick(menu("agent", notStarted), "Resume").onSelect();
    expect(startProcess).toHaveBeenCalledWith("p1");
  });
  it("agent: Close stops first when running, and closes its modal", async () => {
    useFloorStore.getState().openTerminal("p1");
    pick(menu("agent", running), "Close agent").onSelect();
    // The modal stays until the stop and remove are done.
    expect(useFloorStore.getState().terminalId).toBe("p1");
    await vi.waitFor(() =>
      expect(useFloorStore.getState().terminalId).toBeNull(),
    );
    expect(removeProcess).toHaveBeenCalledWith("p1");
    expect(stopCalls()).toEqual([["process_stop", { processId: "p1" }]]);
    expect(stoppedBeforeRemove()).toBe(true);
  });
  it("agent: Close only removes when ended or never started", async () => {
    for (const s of [exited, notStarted]) {
      vi.clearAllMocks();
      pick(menu("agent", s), "Close agent").onSelect();
      await vi.waitFor(() => expect(removeProcess).toHaveBeenCalledWith("p1"));
      expect(stopCalls()).toEqual([]);
    }
  });
  it("Close reads the status at action time: restarted in between, it stops first", async () => {
    const item = pick(menu("agent", exited), "Close agent");
    setStatus("agent", running);
    item.onSelect();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalledWith("p1"));
    expect(stoppedBeforeRemove()).toBe(true);
  });
  it("Close keeps the process when its stop fails", async () => {
    vi.mocked(invoke).mockImplementation((cmd) =>
      cmd === "process_stop"
        ? Promise.reject({ kind: "other", message: "nope" })
        : Promise.resolve([]),
    );
    useFloorStore.getState().openTerminal("p1");
    pick(menu("terminal", running), "Close terminal").onSelect();
    await vi.waitFor(() =>
      expect(toastError).toHaveBeenCalledWith("Could not stop process", "nope"),
    );
    expect(removeProcess).not.toHaveBeenCalled();
    expect(useFloorStore.getState().terminalId).toBe("p1");
  });
  it("Close keeps the modal when the remove fails", async () => {
    removeProcess.mockResolvedValueOnce(false);
    useFloorStore.getState().openTerminal("p1");
    pick(menu("terminal", exited), "Close terminal").onSelect();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalledWith("p1"));
    await Promise.resolve();
    expect(useFloorStore.getState().terminalId).toBe("p1");
  });
  it("Close of a stopped process removes it, then closes its modal", async () => {
    useFloorStore.getState().openTerminal("p1");
    pick(menu("terminal", exited), "Close terminal").onSelect();
    await vi.waitFor(() =>
      expect(useFloorStore.getState().terminalId).toBeNull(),
    );
    expect(removeProcess).toHaveBeenCalledWith("p1");
    expect(stopCalls()).toEqual([]);
  });
  it("Close still removes one that ended before the stop landed", async () => {
    vi.mocked(invoke).mockImplementation((cmd) =>
      cmd === "process_stop"
        ? Promise.reject({ kind: "processNotRunning", message: "gone" })
        : Promise.resolve([]),
    );
    pick(menu("terminal", running), "Close terminal").onSelect();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalledWith("p1"));
    expect(toastError).not.toHaveBeenCalled();
  });
  it("Close does nothing for a process already gone", async () => {
    const item = pick(menu("terminal", running), "Close terminal");
    useProcessStore.setState({ processes: [] });
    item.onSelect();
    await Promise.resolve();
    expect(stopCalls()).toEqual([]);
    expect(removeProcess).not.toHaveBeenCalled();
  });
  it("a stale Stop or Start does nothing", () => {
    const stop = pick(menu("terminal", running), "Stop");
    setStatus("terminal", exited);
    stop.onSelect();
    expect(stopProcess).not.toHaveBeenCalled();

    const resume = pick(menu("agent", exited), "Resume");
    setStatus("agent", running);
    resume.onSelect();
    expect(startProcess).not.toHaveBeenCalled();

    const start = pick(menu("terminal", notStarted), "Start");
    useProcessStore.setState({ processes: [] });
    start.onSelect();
    expect(startProcess).not.toHaveBeenCalled();
  });

  it("terminal: running vs not running", () => {
    expect(labels(menu("terminal", running))).toEqual([
      "Open",
      "Stop",
      "Restart",
      "Close terminal",
    ]);
    for (const s of [notStarted, exited])
      expect(labels(menu("terminal", s))).toEqual([
        "Open",
        "Start",
        "Close terminal",
      ]);
    expect(menu("terminal", running)[3]).toBe("separator");
    expect(pick(menu("terminal", running), "Close terminal").danger).toBe(true);
    pick(menu("terminal", notStarted), "Start").onSelect();
    expect(startProcess).toHaveBeenCalledWith("p1");
    pick(menu("terminal", running), "Restart").onSelect();
    expect(restartProcess).toHaveBeenCalledWith("p1");
  });
  it("terminal: Close stops first when running, and closes its modal", async () => {
    useFloorStore.getState().openTerminal("p1");
    pick(menu("terminal", running), "Close terminal").onSelect();
    // The modal stays until the stop and remove are done.
    expect(useFloorStore.getState().terminalId).toBe("p1");
    await vi.waitFor(() =>
      expect(useFloorStore.getState().terminalId).toBeNull(),
    );
    expect(removeProcess).toHaveBeenCalledWith("p1");
    expect(stopCalls()).toEqual([["process_stop", { processId: "p1" }]]);
    expect(stoppedBeforeRemove()).toBe(true);
  });
  it("terminal: Close keeps another terminal's modal and skips Stop when not running", async () => {
    useFloorStore.getState().openTerminal("other");
    pick(menu("terminal", notStarted), "Close terminal").onSelect();
    await vi.waitFor(() => expect(removeProcess).toHaveBeenCalled());
    expect(stopCalls()).toEqual([]);
    expect(useFloorStore.getState().terminalId).toBe("other");
  });

  it("service: no remove", () => {
    expect(labels(menu("service", running))).toEqual([
      "Open",
      "Stop",
      "Restart",
    ]);
    expect(labels(menu("service", exited))).toEqual(["Open", "Start"]);
    expect(menu("service", running)).not.toContain("separator");
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
  it("todoAgentAction: Cmd/Ctrl-click with a picker asks instead of spawning", () => {
    const ask = vi.fn();
    const a = todoAgentAction(P, todo());
    expect(a.label).toBe("Start agent");
    a.run({ metaKey: true, ctrlKey: false }, ask);
    expect(ask).toHaveBeenCalledWith(P, ["t1"], todo().text);
    expect(spawnAgent).not.toHaveBeenCalled();
  });
  it("openProcess pins the process and opens its terminal", () => {
    openProcess("p3");
    expect(setActiveProcess).toHaveBeenCalledWith("p3");
    expect(useFloorStore.getState().terminalId).toBe("p3");
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

describe("confirm steps", () => {
  it("carry the modal texts", () => {
    expect(pick(menu("agent", running), "Stop").confirm).toEqual({
      title: "Stop x?",
      body: "The session ends. You can resume it later.",
      action: "Stop",
    });
    expect(pick(menu("agent", exited), "Close agent").confirm).toEqual({
      title: "Close x?",
      body: "The agent stops and its desk is removed from the floor.",
      action: "Close",
    });
    expect(pick(menu("agent", exited), "Resume").confirm).toBeUndefined();
    const stop = {
      title: "Stop x?",
      body: "The process stops. You can start it again later.",
      action: "Stop",
    };
    expect(pick(menu("terminal", running), "Stop").confirm).toEqual(stop);
    expect(pick(menu("service", running), "Stop").confirm).toEqual(stop);
    expect(pick(menu("terminal", running), "Close terminal").confirm).toEqual({
      title: "Close x?",
      body: "The terminal stops and is removed, with its output.",
      action: "Close",
    });
  });

  it("leaves archive unconfirmed", () => {
    expect(
      pick(todoMenu(P, {} as TodoInfo), "Archive").confirm,
    ).toBeUndefined();
    expect(
      pick(
        scratchpadMenu(P, { id: "s", title: "t" } as ScratchpadInfo),
        "Archive",
      ).confirm,
    ).toBeUndefined();
  });
});
