import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { TodoInfo } from "../ipc/types";
import { useTodoStore } from "../state/todoStore";
import { ArchiveModal } from "./ArchiveModal";

const initialTodo = useTodoStore.getState();
const PROJECT = "proj-1";

function todo(id: string, text: string, done = false): TodoInfo {
  return {
    id,
    projectId: PROJECT,
    text,
    description: null,
    done,
    createdAt: "2024-04-03T12:00:00Z",
    doneAt: null,
    archived: true,
    archivedAt: "2024-04-05T09:30:00Z",
    links: [],
    comments: [],
    assignedAgent: null,
  };
}

function seed(archived: TodoInfo[]) {
  const refreshArchived = vi.fn(() => Promise.resolve());
  const setTodoArchived = vi.fn(() => Promise.resolve(null));
  const removeTodo = vi.fn(() => Promise.resolve());
  useTodoStore.setState(
    {
      ...initialTodo,
      archivedByProject: { [PROJECT]: archived },
      refreshArchived,
      setTodoArchived,
      removeTodo,
    },
    true,
  );
  return { refreshArchived, setTodoArchived, removeTodo };
}

describe("ArchiveModal", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
  });

  it("loads the archived list when it opens", () => {
    const { refreshArchived } = seed([]);
    render(<ArchiveModal open projectId={PROJECT} onClose={() => undefined} />);
    expect(refreshArchived).toHaveBeenCalledWith(PROJECT);
  });

  it("shows an empty state when there are no archived to-dos", () => {
    seed([]);
    render(<ArchiveModal open projectId={PROJECT} onClose={() => undefined} />);
    expect(screen.getByText("No archived to-dos yet.")).toBeInTheDocument();
  });

  it("lists archived to-dos and restores one", () => {
    const { setTodoArchived } = seed([todo("t1", "Old task", true)]);
    render(<ArchiveModal open projectId={PROJECT} onClose={() => undefined} />);

    expect(screen.getByText("Old task")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: 'Restore "Old task"' }));
    expect(setTodoArchived).toHaveBeenCalledWith(PROJECT, "t1", false);
  });

  it("deletes an archived to-do", () => {
    const { removeTodo } = seed([todo("t1", "Old task", true)]);
    render(<ArchiveModal open projectId={PROJECT} onClose={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: 'Delete "Old task"' }));
    expect(removeTodo).toHaveBeenCalledWith(PROJECT, "t1");
  });

  it("renders nothing while closed", () => {
    seed([todo("t1", "Old task")]);
    const { container } = render(
      <ArchiveModal
        open={false}
        projectId={PROJECT}
        onClose={() => undefined}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
