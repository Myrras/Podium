import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { TodoInfo } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { useTodoStore } from "../state/todoStore";
import { TodoSubsection } from "./TodoSubsection";

const initialTodo = useTodoStore.getState();
const initialProcess = useProcessStore.getState();
const initialProject = useProjectStore.getState();

const PROJECT = "proj-1";

function todo(id: string, text: string): TodoInfo {
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
    assignedAgent: null,
  };
}

/** Seed the three stores the subsection reads, with spy-able actions. */
function seed(todos: TodoInfo[]) {
  const spawnAgent = vi.fn(() => Promise.resolve(null));
  const setActiveProject = vi.fn();
  const setTodoArchived = vi.fn(() => Promise.resolve(null));
  const refreshArchived = vi.fn(() => Promise.resolve());
  useTodoStore.setState(
    {
      ...initialTodo,
      todosByProject: { [PROJECT]: todos },
      refresh: vi.fn(() => Promise.resolve()),
      addTodo: vi.fn(() => Promise.resolve(true)),
      setTodoDone: vi.fn(() => Promise.resolve()),
      removeTodo: vi.fn(() => Promise.resolve()),
      setTodoArchived,
      refreshArchived,
    },
    true,
  );
  useProcessStore.setState({ ...initialProcess, spawnAgent }, true);
  useProjectStore.setState({ ...initialProject, setActiveProject }, true);
  return { spawnAgent, setActiveProject, setTodoArchived, refreshArchived };
}

/** The click target for a to-do's title (opens / drives selection). */
function titleOf(text: string): HTMLElement {
  const el = screen.getByText(text).closest("button");
  if (!el) throw new Error(`no title button for "${text}"`);
  return el;
}

describe("TodoSubsection multi-select", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
    useProcessStore.setState(initialProcess, true);
    useProjectStore.setState(initialProject, true);
  });

  it("marks a Cmd/Ctrl-clicked row multiselect, not open", () => {
    seed([todo("a", "Todo A"), todo("b", "Todo B")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );
    fireEvent.click(titleOf("Todo A"), { metaKey: true });
    const row = screen.getByText("Todo A").closest("[data-multiselect]");
    expect(row).not.toBeNull();
    expect((row as HTMLElement).getAttribute("data-open")).toBeNull();
  });

  it("opens a to-do on a plain click, without selecting", () => {
    seed([todo("a", "Todo A"), todo("b", "Todo B")]);
    const onOpenTodo = vi.fn();
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={onOpenTodo}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Todo A"));

    expect(onOpenTodo).toHaveBeenCalledWith(PROJECT, "a");
    expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
  });

  it("spawns one agent on all Cmd/Ctrl-selected to-dos", () => {
    const { spawnAgent } = seed([
      todo("a", "Todo A"),
      todo("b", "Todo B"),
      todo("c", "Todo C"),
    ]);
    const onOpenTodo = vi.fn();
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={onOpenTodo}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Todo A"), { metaKey: true });
    fireEvent.click(titleOf("Todo C"), { ctrlKey: true });

    // Selecting must not open the detail pane.
    expect(onOpenTodo).not.toHaveBeenCalled();

    const bar = screen.getByText("Start agent on 2 to-dos");
    fireEvent.click(bar);

    // Ids are passed in list order as one combined task.
    expect(spawnAgent).toHaveBeenCalledWith(PROJECT, { todoIds: ["a", "c"] });
    // The bar clears after spawning.
    expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
  });

  it("keeps the selection when opening another to-do with a plain click", () => {
    const { spawnAgent } = seed([
      todo("a", "Todo A"),
      todo("b", "Todo B"),
      todo("c", "Todo C"),
    ]);
    const onOpenTodo = vi.fn();
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={onOpenTodo}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Todo A"), { metaKey: true });
    fireEvent.click(titleOf("Todo B"), { metaKey: true });
    expect(screen.getByText("Start agent on 2 to-dos")).toBeInTheDocument();

    // A plain click only views the to-do; the selection must survive.
    fireEvent.click(titleOf("Todo C"));

    expect(onOpenTodo).toHaveBeenCalledWith(PROJECT, "c");
    expect(screen.getByText("Start agent on 2 to-dos")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Start agent on 2 to-dos"));
    expect(spawnAgent).toHaveBeenCalledWith(PROJECT, { todoIds: ["a", "b"] });
  });

  it("extends a range with Shift+click", () => {
    const { spawnAgent } = seed([
      todo("a", "Todo A"),
      todo("b", "Todo B"),
      todo("c", "Todo C"),
    ]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Todo A"), { metaKey: true });
    fireEvent.click(titleOf("Todo C"), { shiftKey: true });

    fireEvent.click(screen.getByText("Start agent on 3 to-dos"));
    expect(spawnAgent).toHaveBeenCalledWith(PROJECT, {
      todoIds: ["a", "b", "c"],
    });
  });

  it("hides the bar with only one selected", () => {
    seed([todo("a", "Todo A"), todo("b", "Todo B")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Todo A"), { metaKey: true });

    expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
  });
});

describe("TodoSubsection spawn button", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
    useProcessStore.setState(initialProcess, true);
    useProjectStore.setState(initialProject, true);
  });

  it("spawns the default agent on a plain click", () => {
    const { spawnAgent } = seed([todo("a", "Todo A")]);
    const onPickAgent = vi.fn();
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={onPickAgent}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: 'Start an agent on "Todo A"' }),
    );

    expect(spawnAgent).toHaveBeenCalledWith(PROJECT, { todoIds: ["a"] });
    expect(onPickAgent).not.toHaveBeenCalled();
  });

  it("opens the agent picker on Cmd/Ctrl+click, prefilled with the to-do", () => {
    const { spawnAgent } = seed([todo("a", "Todo A")]);
    const onPickAgent = vi.fn();
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={onPickAgent}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: 'Start an agent on "Todo A"' }),
      { metaKey: true },
    );

    expect(onPickAgent).toHaveBeenCalledWith(PROJECT, ["a"], "Todo A");
    // Cmd/Ctrl+click defers to the picker; it must not spawn immediately.
    expect(spawnAgent).not.toHaveBeenCalled();
  });

  it("archives a to-do from its row action", () => {
    const { setTodoArchived } = seed([todo("a", "Todo A")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: 'Archive to-do "Todo A"' }),
    );
    expect(setTodoArchived).toHaveBeenCalledWith(PROJECT, "a", true);
  });

  it("opens the archive modal from the header, loading the list", () => {
    const { refreshArchived } = seed([todo("a", "Todo A")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "View archived to-dos" }),
    );
    expect(
      screen.getByRole("dialog", { name: "Archived to-dos" }),
    ).toBeInTheDocument();
    expect(refreshArchived).toHaveBeenCalledWith(PROJECT);
  });
});

/** A to-do owned by a session, tinted with that session's colour. */
function assignedTodo(id: string, text: string, color: string): TodoInfo {
  return {
    ...todo(id, text),
    assignedAgent: { processId: "sess-1", name: "session", color },
  };
}

describe("TodoSubsection assigned to-dos", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
    useProcessStore.setState(initialProcess, true);
    useProjectStore.setState(initialProject, true);
  });

  it("hides the spawn button and tints the row with the session colour", () => {
    seed([assignedTodo("a", "Todo A", "#3e63dd")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    // No spawning a second agent onto an owned to-do.
    expect(
      screen.queryByRole("button", { name: 'Start an agent on "Todo A"' }),
    ).not.toBeInTheDocument();

    // The row is marked assigned and carries the session colour var.
    const row = screen.getByText("Todo A").closest('[data-assigned="true"]');
    expect(row).not.toBeNull();
    expect((row as HTMLElement).style.getPropertyValue("--session-color")).toBe(
      "#3e63dd",
    );
  });

  it("never joins a multi-select — any click just opens it", () => {
    const onOpenTodo = vi.fn();
    seed([assignedTodo("a", "Todo A", "#30a46c"), todo("b", "Todo B")]);
    render(
      <TodoSubsection
        projectId={PROJECT}
        onOpenTodo={onOpenTodo}
        onPickAgent={vi.fn()}
      />,
    );

    // Cmd/Ctrl+click on an assigned row opens it instead of selecting.
    fireEvent.click(titleOf("Todo A"), { metaKey: true });
    expect(onOpenTodo).toHaveBeenCalledWith(PROJECT, "a");
    expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
  });
});
