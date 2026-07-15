import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { AssignedAgent, TodoInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useProjectStore } from "../state/projectStore";
import { useTodoStore } from "../state/todoStore";
import { AgentTodoList } from "./AgentTodoList";

const initialTodo = useTodoStore.getState();

const PROJECT = "proj-1";
const AGENT = "agent-1";

function todo(
  id: string,
  text: string,
  assignedAgent: AssignedAgent | null,
): TodoInfo {
  return {
    id,
    projectId: PROJECT,
    text,
    description: null,
    done: false,
    createdAt: "2024-04-03T12:00:00Z",
    doneAt: null,
    archived: false,
    archivedAt: null,
    links: [],
    comments: [],
    assignedAgent,
  };
}

function seed(todos: TodoInfo[]) {
  const unassignTodo = vi.fn(() => Promise.resolve());
  useTodoStore.setState(
    { ...initialTodo, todosByProject: { [PROJECT]: todos }, unassignTodo },
    true,
  );
  return { unassignTodo };
}

describe("AgentTodoList", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
  });

  it("lists only the to-dos assigned to this agent", () => {
    seed([
      todo("a", "Wire auth", { processId: AGENT, name: "claude" }),
      todo("b", "Write tests", { processId: "other", name: "beta" }),
      todo("c", "Unassigned", null),
    ]);
    render(<AgentTodoList projectId={PROJECT} processId={AGENT} />);

    expect(screen.getByText("Wire auth")).toBeInTheDocument();
    expect(screen.queryByText("Write tests")).not.toBeInTheDocument();
    expect(screen.queryByText("Unassigned")).not.toBeInTheDocument();
  });

  it("renders nothing when the agent has no assigned to-dos", () => {
    seed([todo("b", "Write tests", { processId: "other", name: "beta" })]);
    const { container } = render(
      <AgentTodoList projectId={PROJECT} processId={AGENT} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("unassigns the to-do when the (x) is clicked", () => {
    const { unassignTodo } = seed([
      todo("a", "Wire auth", { processId: AGENT, name: "claude" }),
    ]);
    render(<AgentTodoList projectId={PROJECT} processId={AGENT} />);

    fireEvent.click(
      screen.getByRole("button", { name: /Stop this agent and unassign/ }),
    );

    expect(unassignTodo).toHaveBeenCalledWith(PROJECT, "a");
  });

  it("opens the to-do in the work area when its text is clicked", () => {
    seed([todo("a", "Wire auth", { processId: AGENT, name: "claude" })]);
    const openTodoInWorkArea = vi.spyOn(
      useLayoutStore.getState(),
      "openTodoInWorkArea",
    );
    const setActiveProject = vi.spyOn(
      useProjectStore.getState(),
      "setActiveProject",
    );
    render(<AgentTodoList projectId={PROJECT} processId={AGENT} />);

    fireEvent.click(screen.getByRole("button", { name: "Wire auth" }));

    expect(setActiveProject).toHaveBeenCalledWith(PROJECT);
    expect(openTodoInWorkArea).toHaveBeenCalledWith(PROJECT, "a");
  });
});
