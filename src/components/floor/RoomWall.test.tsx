import { act, fireEvent, render, screen } from "@testing-library/react";
import { Profiler } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  ProcessInfo,
  ProjectInfo,
  ScratchpadInfo,
  TodoInfo,
} from "../../ipc/types";
import type { RoomModel } from "../../lib/floor/types";
import { useFloorStore } from "../../state/floorStore";
import { useScratchpadStore } from "../../state/scratchpadStore";
import { useTodoStore } from "../../state/todoStore";
import { todoMenu, scratchpadMenu, processMenu } from "../../lib/floor/menus";
import { RoomWall } from "./RoomWall";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

vi.mock("../../lib/floor/menus", () => ({
  todoMenu: vi.fn(() => [{ label: "todo-item", onSelect: vi.fn() }]),
  scratchpadMenu: vi.fn(() => [{ label: "pad-item", onSelect: vi.fn() }]),
  processMenu: vi.fn(() => [{ label: "proc-item", onSelect: vi.fn() }]),
}));

const project = {
  id: "p1",
  name: "Alpha",
  sections: { todos: true, scratchpads: true, terminals: true },
} as unknown as ProjectInfo;

const todo = (id: string, extra: Partial<TodoInfo> = {}) =>
  ({
    id,
    projectId: "p1",
    done: false,
    assignedAgent: null,
    ...extra,
  }) as TodoInfo;

const service = (id: string, status: ProcessInfo["status"]) =>
  ({ id, status }) as unknown as ProcessInfo;

function room(over: Partial<RoomModel> = {}, p = project): RoomModel {
  return { project: p, services: [], ...over } as unknown as RoomModel;
}

function seed(todos: TodoInfo[], pads: number) {
  useTodoStore.setState({ todosByProject: { p1: todos } } as never);
  useScratchpadStore.setState({
    scratchpadsByProject: {
      p1: Array.from(
        { length: pads },
        (_, i) => ({ id: `s${i}` }) as ScratchpadInfo,
      ),
    },
  } as never);
}

beforeEach(() => useFloorStore.getState().reset());

describe("RoomWall", () => {
  it("counts open to-dos, caps notes at 8 and colours them", () => {
    seed(
      [
        ...Array.from({ length: 9 }, (_, i) => todo(`t${i}`)),
        todo("done", { done: true }),
      ],
      0,
    );
    const { container } = render(<RoomWall room={room()} />);
    expect(screen.getByLabelText("To-dos of Alpha, 9")).toBeTruthy();
    expect(container.querySelectorAll("[data-note]")).toHaveLength(8);
    expect(screen.getByText("+1")).toBeTruthy();
  });

  it("tints notes by owner and hollows stopped ones", () => {
    const agent = { processId: "a", name: "A", color: "#ff0000" };
    seed(
      [
        todo("1", { assignedAgent: { ...agent, active: true } }),
        todo("2", { assignedAgent: { ...agent, active: false } }),
      ],
      0,
    );
    const { container } = render(<RoomWall room={room()} />);
    const notes = container.querySelectorAll<HTMLElement>("[data-note]");
    expect(notes[0].style.getPropertyValue("--note")).toBe("#ff0000");
    expect(notes[0].dataset.hollow).toBe("false");
    expect(notes[1].dataset.hollow).toBe("true");
  });

  it("opens the popover from a fixture", () => {
    seed([], 5);
    const { container } = render(<RoomWall room={room()} />);
    expect(container.querySelectorAll("[class*=books] i")).toHaveLength(4);
    fireEvent.click(screen.getByLabelText("Scratchpads of Alpha, 5"));
    expect(useFloorStore.getState().popover).toEqual({
      projectId: "p1",
      kind: "scratchpads",
      itemId: null,
    });
    expect(
      document.querySelector('[data-fixture="p1:scratchpads"]'),
    ).toBeTruthy();
  });

  it("marks the open fixture and ignores another room's popover", () => {
    seed([], 1);
    const renders = vi.fn();
    render(
      <Profiler id="wall" onRender={renders}>
        <RoomWall room={room()} />
      </Profiler>,
    );
    const pads = screen.getByLabelText("Scratchpads of Alpha, 1");
    act(() =>
      useFloorStore
        .getState()
        .openPopover({ projectId: "p1", kind: "scratchpads", itemId: null }),
    );
    expect(pads.dataset.on).toBe("true");
    renders.mockClear();
    act(() =>
      useFloorStore
        .getState()
        .openPopover({ projectId: "p2", kind: "todos", itemId: null }),
    );
    expect(pads.dataset.on).toBe("false");
    renders.mockClear();
    act(() =>
      useFloorStore
        .getState()
        .openPopover({ projectId: "p2", kind: "scratchpads", itemId: null }),
    );
    expect(renders).not.toHaveBeenCalled();
  });

  it("hides fixtures of hidden sections", () => {
    seed([], 0);
    const hidden = {
      ...project,
      sections: { todos: false, scratchpads: false },
    } as unknown as ProjectInfo;
    render(<RoomWall room={room({}, hidden)} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows a count-less Commands fixture only when terminals are on", () => {
    seed([], 0);
    const { unmount } = render(<RoomWall room={room()} />);
    fireEvent.click(screen.getByLabelText("Commands of Alpha"));
    expect(useFloorStore.getState().popover).toEqual({
      projectId: "p1",
      kind: "commands",
      itemId: null,
    });
    expect(document.querySelector('[data-fixture="p1:commands"]')).toBeTruthy();
    unmount();
    const off = {
      ...project,
      sections: { todos: true, scratchpads: true, terminals: false },
    } as unknown as ProjectInfo;
    render(<RoomWall room={room({}, off)} />);
    expect(screen.queryByLabelText("Commands of Alpha")).toBeNull();
  });

  it("shows the rack with crashed light first and a bang", () => {
    seed([], 0);
    const exited = (crashed: boolean) =>
      ({ state: "exited", code: 1, crashed, stopped: false, at: "" }) as const;
    const services = [
      service("a", { state: "running", pid: 1, since: "" }),
      service("b", exited(false)),
      service("c", { state: "notStarted" }),
      service("d", exited(true)),
    ];
    const { container } = render(<RoomWall room={room({ services })} />);
    expect(screen.getByLabelText("Services of Alpha, 4")).toBeTruthy();
    const lights = [...container.querySelectorAll<HTMLElement>("[data-light]")];
    expect(lights.map((l) => l.dataset.light)).toEqual([
      "crashed",
      "up",
      "exited",
    ]);
    expect(screen.getByText("+1")).toBeTruthy();
    expect(screen.getByText("!")).toBeTruthy();
  });

  it("lights stopping and never-started services off, running ones up", () => {
    seed([], 0);
    const services = [
      service("a", { state: "stopping" }),
      service("b", { state: "notStarted" }),
      service("c", { state: "running", pid: 1, since: "" }),
    ];
    const { container } = render(<RoomWall room={room({ services })} />);
    const lights = [...container.querySelectorAll<HTMLElement>("[data-light]")];
    expect(lights.map((l) => l.dataset.light)).toEqual(["off", "off", "up"]);
  });

  it("has no rack without services", () => {
    seed([], 0);
    render(<RoomWall room={room()} />);
    expect(screen.queryByLabelText(/Services of/)).toBeNull();
  });

  it("opens the to-do menu on right-click of a note", () => {
    const t = todo("t1");
    seed([t], 0);
    const { container } = render(<RoomWall room={room()} />);
    fireEvent.contextMenu(container.querySelector("[data-note]")!);
    expect(todoMenu).toHaveBeenCalledWith("p1", t);
    expect(screen.getByText("todo-item")).toBeTruthy();
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("opens the scratchpad menu on right-click of a notebook", () => {
    seed([], 1);
    const { container } = render(<RoomWall room={room()} />);
    fireEvent.contextMenu(container.querySelector("[data-book]")!);
    expect(scratchpadMenu).toHaveBeenCalledWith("p1", { id: "s0" });
    expect(screen.getByText("pad-item")).toBeTruthy();
  });

  it("opens the process menu on right-click of a light", () => {
    seed([], 0);
    const sv = service("a", { state: "notStarted" });
    const { container } = render(<RoomWall room={room({ services: [sv] })} />);
    fireEvent.contextMenu(container.querySelector("[data-light]")!);
    expect(processMenu).toHaveBeenCalledWith(sv, expect.anything());
    expect(screen.getByText("proc-item")).toBeTruthy();
  });

  it("ignores right-click elsewhere on a fixture", () => {
    seed([todo("t1")], 0);
    render(<RoomWall room={room()} />);
    fireEvent.contextMenu(screen.getByLabelText("To-dos of Alpha, 1"));
    expect(screen.queryByText("todo-item")).toBeNull();
  });
});
