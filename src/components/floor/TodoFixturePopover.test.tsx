import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

vi.mock("../../lib/floor/menus", async (orig) => ({
  ...(await orig<typeof import("../../lib/floor/menus")>()),
  todoMenu: vi.fn(() => [{ label: "Menu item", onSelect: () => {} }]),
}));

import type { TodoInfo } from "../../ipc/types";
import { todoMenu } from "../../lib/floor/menus";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useTodoStore } from "../../state/todoStore";
import { TodoFixturePopover } from "./TodoFixturePopover";

const P = "proj-1";
const todo = (id: string, text: string): TodoInfo => ({
  id,
  projectId: P,
  text,
  description: null,
  done: false,
  createdAt: "2024-04-03T12:00:00Z",
  doneAt: null,
  archived: false,
  archivedAt: null,
  links: [],
  comments: [],
  assignedAgent: null,
});

const spawnAgent = vi.fn();

function open(itemId: string | null) {
  useFloorStore.getState().openPopover({ projectId: P, kind: "todos", itemId });
  return render(
    <TodoFixturePopover projectId={P} itemId={itemId} anchor={document.body} />,
  );
}

beforeEach(() => {
  spawnAgent.mockReset();
  useFloorStore.getState().reset();
  useTodoStore.setState({
    todosByProject: { [P]: [todo("t1", "Fix the lamp")] },
    refresh: vi.fn(() => Promise.resolve()),
  } as never);
  useProcessStore.setState({ spawnAgent } as never);
});

describe("TodoFixturePopover", () => {
  it("lists to-dos and opens one inside the popover", () => {
    useFloorStore
      .getState()
      .openPopover({ projectId: P, kind: "todos", itemId: null });
    open(null);
    fireEvent.click(screen.getByText("Fix the lamp"));
    expect(useFloorStore.getState().popover?.itemId).toBe("t1");
  });

  it("shows the item with a Back link that returns to the list", () => {
    open("t1");
    fireEvent.click(screen.getByRole("button", { name: /Back to to-dos/ }));
    expect(useFloorStore.getState().popover?.itemId).toBeNull();
  });

  it("Cmd-click on Start opens the agent modal, plain click spawns", () => {
    open(null);
    const start = screen.getByLabelText(/Start an agent on "Fix the lamp"/);
    fireEvent.click(start, { metaKey: true });
    expect(useFloorStore.getState().agentModal).toEqual({
      projectId: P,
      todoIds: ["t1"],
      initialName: "Fix the lamp",
    });
    fireEvent.click(start);
    expect(spawnAgent).toHaveBeenCalledWith(P, { todoIds: ["t1"] });
  });

  it("item view: Start agent follows the list's rule", () => {
    open("t1");
    const start = screen.getByRole("button", { name: "Start agent" });
    fireEvent.click(start, { ctrlKey: true });
    expect(useFloorStore.getState().agentModal).toEqual({
      projectId: P,
      todoIds: ["t1"],
      initialName: "Fix the lamp",
    });
    fireEvent.click(start);
    expect(spawnAgent).toHaveBeenCalledWith(P, { todoIds: ["t1"] });
  });

  it("item view: Archive this to-do archives it and returns to the list", () => {
    const setTodoArchived = vi.fn(() => Promise.resolve(null));
    useTodoStore.setState({ setTodoArchived } as never);
    open("t1");
    fireEvent.click(screen.getByRole("button", { name: "Archive this to-do" }));
    expect(setTodoArchived).toHaveBeenCalledWith(P, "t1", true);
    expect(useFloorStore.getState().popover?.itemId).toBeNull();
  });

  it("item view: a live owner shows Open the agent", () => {
    const setActiveProcess = vi.fn();
    useProcessStore.setState({ setActiveProcess } as never);
    useTodoStore.setState({
      todosByProject: {
        [P]: [
          {
            ...todo("t1", "Fix the lamp"),
            assignedAgent: {
              processId: "p9",
              name: "A",
              color: null,
              active: true,
            },
          },
        ],
      },
    } as never);
    open("t1");
    expect(screen.queryByRole("button", { name: "Start agent" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open the agent" }));
    expect(setActiveProcess).toHaveBeenCalledWith("p9");
    expect(useFloorStore.getState().terminalId).toBe("p9");
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("one header bar: title, open count and the list actions", () => {
    useTodoStore.setState({
      todosByProject: {
        [P]: [
          todo("t1", "Fix the lamp"),
          { ...todo("t2", "Water the fern"), done: true },
        ],
      },
    } as never);
    open(null);
    const header = screen.getByRole("heading", { name: /To-dos/ })
      .parentElement as HTMLElement;
    expect(header.tagName).toBe("HEADER");
    expect(screen.getByRole("heading")).toHaveTextContent("To-dos1");
    expect(header).toContainElement(screen.getByLabelText("New to-do"));
    expect(header).toContainElement(
      screen.getByLabelText("View archived to-dos"),
    );
    expect(document.querySelector('[class*="bareBar"]')).toBeNull();
  });

  it("the add input is the first row of the list", () => {
    useTodoStore.setState({
      todosByProject: { [P]: [todo("t1", "Fix the lamp")] },
    } as never);
    open(null);
    fireEvent.click(screen.getByLabelText("New to-do"));
    const input = screen.getByLabelText("New to-do text");
    expect(
      input.compareDocumentPosition(screen.getByText("Fix the lamp")) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("rows carry the owner colour: solid, hollow once stopped, none when free", () => {
    const owned = (active: boolean, color: string | null) => ({
      processId: "p9",
      name: "Agent A",
      color,
      active,
    });
    useTodoStore.setState({
      todosByProject: {
        [P]: [
          { ...todo("t1", "Live"), assignedAgent: owned(true, "#ff0000") },
          { ...todo("t2", "Stopped"), assignedAgent: owned(false, null) },
          todo("t3", "Free"),
        ],
      },
    } as never);
    open(null);
    const live = screen.getByRole("img", { name: "Owned by Agent A" });
    expect(live).toHaveAttribute("data-owned", "true");
    expect(live).not.toHaveAttribute("data-stopped");
    expect(
      live
        .closest<HTMLElement>("[data-assigned]")
        ?.style.getPropertyValue("--session-color"),
    ).toBe("#ff0000");
    expect(
      screen.getByRole("img", { name: "Owned by Agent A (stopped)" }),
    ).toHaveAttribute("data-stopped", "true");
    // Free to-do: no owner mark.
    expect(screen.getAllByRole("img")).toHaveLength(2);
    expect(document.querySelectorAll("[data-assigned]")).toHaveLength(2);
    expect(screen.getByText("Free").closest("button")).toHaveAttribute(
      "title",
      "Free",
    );
  });

  it("empty list invites to add one", () => {
    useTodoStore.setState({ todosByProject: { [P]: [] } } as never);
    open(null);
    expect(
      screen.getByText("No open to-dos. Add one with +."),
    ).toBeInTheDocument();
  });

  it("with 0 open and 3 done, invites to add one and still lists the done ones", () => {
    const done = (id: string) => ({ ...todo(id, `Done ${id}`), done: true });
    useTodoStore.setState({
      todosByProject: { [P]: [done("d1"), done("d2"), done("d3")] },
    } as never);
    open(null);
    expect(
      screen.getByText("No open to-dos. Add one with +."),
    ).toBeInTheDocument();
    expect(screen.getByText("Done d1")).toBeInTheDocument();
    expect(screen.getByText("Done d3")).toBeInTheDocument();
  });

  it("does not highlight the to-do open in Project mode's work area", () => {
    useLayoutStore.setState({ openTodo: { projectId: P, todoId: "t1" } });
    open(null);
    expect(screen.getByText("Fix the lamp").closest("[data-open]")).toBeNull();
    useLayoutStore.setState({ openTodo: null });
  });

  it("right-click on a row opens its menu, one at a time", () => {
    useTodoStore.setState({
      todosByProject: {
        [P]: [todo("t1", "Fix the lamp"), todo("t2", "Water the fern")],
      },
    } as never);
    open(null);
    const ev = fireEvent.contextMenu(screen.getByText("Fix the lamp"), {
      clientX: 40,
      clientY: 50,
    });
    expect(ev).toBe(false);
    expect(todoMenu).toHaveBeenLastCalledWith(
      P,
      expect.objectContaining({ id: "t1" }),
    );
    expect(screen.getByRole("menuitem", { name: "Menu item" })).toBeVisible();
    fireEvent.contextMenu(screen.getByText("Water the fern"));
    expect(todoMenu).toHaveBeenLastCalledWith(
      P,
      expect.objectContaining({ id: "t2" }),
    );
    expect(screen.getAllByRole("menu")).toHaveLength(1);
    expect(useFloorStore.getState().popover).not.toBeNull();
  });

  it("item view: back and the title in the header, actions in the footer", () => {
    open("t1");
    const header = document.querySelector("header") as HTMLElement;
    expect(header).toContainElement(
      screen.getByRole("button", { name: "Back to to-dos" }),
    );
    expect(header).toHaveTextContent("Fix the lamp");
    expect(screen.queryByLabelText("Close to-do")).toBeNull();
    const footer = document.querySelector("footer") as HTMLElement;
    expect(footer).toContainElement(
      screen.getByRole("button", { name: "Start agent" }),
    );
    expect(footer).toContainElement(
      screen.getByRole("button", { name: "Archive this to-do" }),
    );
  });
});
