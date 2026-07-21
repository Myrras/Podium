import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

const { openUrlMock } = vi.hoisted(() => ({ openUrlMock: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: openUrlMock }));

import type { TodoComment, TodoInfo } from "../ipc/types";
import { useLayoutStore } from "../state/layoutStore";
import { useTodoStore } from "../state/todoStore";
import { TodoDetailPane } from "./TodoDetailPane";

const initialTodo = useTodoStore.getState();
const initialLayout = useLayoutStore.getState();

const PROJECT = "proj-1";
const TODO = "todo-1";

function comment(overrides: Partial<TodoComment> = {}): TodoComment {
  return {
    id: "comment-1",
    author: "claude",
    text: "Started on the export.",
    createdAt: "2024-04-03T13:00:00Z",
    editedAt: null,
    ...overrides,
  };
}

function todo(overrides: Partial<TodoInfo> = {}): TodoInfo {
  return {
    id: TODO,
    projectId: PROJECT,
    text: "Wire the export button",
    description: "Add a CSV export to the reports view.",
    done: false,
    createdAt: "2024-04-03T12:00:00Z",
    doneAt: null,
    archived: false,
    archivedAt: null,
    links: [],
    comments: [],
    assignedAgent: null,
    ...overrides,
  };
}

/** Seed the todo store with one to-do and spy-able mutation actions. */
function seed(overrides: Partial<TodoInfo> = {}) {
  const updateTodo = vi.fn(() => Promise.resolve(null));
  const commentTodo = vi.fn(() => Promise.resolve(null));
  const editComment = vi.fn(() => Promise.resolve(null));
  const removeComment = vi.fn(() => Promise.resolve(null));
  const removeLink = vi.fn(() => Promise.resolve(null));
  const setTodoDone = vi.fn(() => Promise.resolve());
  useTodoStore.setState(
    {
      ...initialTodo,
      todosByProject: { [PROJECT]: [todo(overrides)] },
      updateTodo,
      commentTodo,
      editComment,
      removeComment,
      removeLink,
      setTodoDone,
    },
    true,
  );
  return {
    updateTodo,
    commentTodo,
    editComment,
    removeComment,
    removeLink,
    setTodoDone,
  };
}

describe("TodoDetailPane", () => {
  beforeEach(() => {
    useTodoStore.setState(initialTodo, true);
    useLayoutStore.setState(initialLayout, true);
    openUrlMock.mockClear();
  });

  it("renders the to-do title, description, and comments", () => {
    seed({ comments: [comment()] });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    expect(screen.getByText("Wire the export button")).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("Add a CSV export to the reports view."),
    ).toBeInTheDocument();
    expect(screen.getByText("claude")).toBeInTheDocument();
    expect(screen.getByText("Started on the export.")).toBeInTheDocument();
  });

  it("renders comment text as markdown", () => {
    seed({
      comments: [comment({ text: "Done **bold** and `code`." })],
    });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(screen.getByText("code").tagName).toBe("CODE");
  });

  it("marks edited comments", () => {
    seed({
      comments: [comment({ editedAt: "2024-04-03T14:00:00Z" })],
    });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    expect(screen.getByText(/edited/)).toBeInTheDocument();
  });

  it("edits a comment via the inline editor", () => {
    const { editComment } = seed({ comments: [comment()] });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    fireEvent.click(screen.getByLabelText("Edit comment"));
    fireEvent.change(screen.getByRole("textbox", { name: "Edit comment" }), {
      target: { value: "Finished the export." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(editComment).toHaveBeenCalledWith(
      PROJECT,
      TODO,
      "comment-1",
      "Finished the export.",
    );
  });

  it("deletes a comment after confirming", () => {
    const { removeComment } = seed({ comments: [comment()] });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    // First click asks for confirmation; nothing removed yet.
    fireEvent.click(screen.getByLabelText("Delete comment"));
    expect(removeComment).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(removeComment).toHaveBeenCalledWith(PROJECT, TODO, "comment-1");
  });

  it("shows the empty-comments hint when there are none", () => {
    seed();
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    expect(
      screen.getByText("No comments yet. Add one to track progress."),
    ).toBeInTheDocument();
  });

  it("reveals Save only when the description is edited, then saves it", () => {
    const { updateTodo } = seed();
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();

    fireEvent.change(
      screen.getByDisplayValue("Add a CSV export to the reports view."),
      { target: { value: "Export to CSV and XLSX." } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(updateTodo).toHaveBeenCalledWith(PROJECT, TODO, {
      description: "Export to CSV and XLSX.",
    });
  });

  it("adds a comment via the composer", () => {
    const { commentTodo } = seed();
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    fireEvent.change(screen.getByPlaceholderText("Add a comment…"), {
      target: { value: "Reviewed the design." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Comment" }));

    expect(commentTodo).toHaveBeenCalledWith(
      PROJECT,
      TODO,
      "Reviewed the design.",
    );
  });

  it("toggles done from the header checkbox", () => {
    const { setTodoDone } = seed();
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    fireEvent.click(
      screen.getByLabelText('Mark "Wire the export button" as done'),
    );
    expect(setTodoDone).toHaveBeenCalledWith(PROJECT, TODO, true);
  });

  it("closes the pane via the close button", () => {
    const clearOpenTodo = vi.fn();
    seed();
    useLayoutStore.setState({ clearOpenTodo });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    fireEvent.click(screen.getByLabelText("Close to-do"));
    expect(clearOpenTodo).toHaveBeenCalled();
  });

  it("closes itself when the to-do no longer exists", () => {
    const clearOpenTodo = vi.fn();
    useTodoStore.setState(
      { ...initialTodo, todosByProject: { [PROJECT]: [] } },
      true,
    );
    useLayoutStore.setState({ clearOpenTodo });
    const { container } = render(
      <TodoDetailPane projectId={PROJECT} todoId={TODO} />,
    );

    expect(container).toBeEmptyDOMElement();
    expect(clearOpenTodo).toHaveBeenCalled();
  });

  it("renders pinned links and removes one via its (x)", () => {
    const { removeLink } = seed({
      links: [
        {
          id: "link-1",
          label: "#42 Fix login",
          url: "https://gitlab.example.com/acme/web/-/issues/42",
          createdAt: "2024-04-03T12:00:00Z",
        },
      ],
    });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    const anchor = screen.getByRole("link", { name: "#42 Fix login" });
    expect(anchor).toHaveAttribute(
      "href",
      "https://gitlab.example.com/acme/web/-/issues/42",
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Remove link #42 Fix login" }),
    );
    expect(removeLink).toHaveBeenCalledWith(PROJECT, TODO, "link-1");
  });

  it("opens a pinned link via the OS default browser instead of navigating the webview", () => {
    seed({
      links: [
        {
          id: "link-1",
          label: "#42 Fix login",
          url: "https://gitlab.example.com/acme/web/-/issues/42",
          createdAt: "2024-04-03T12:00:00Z",
        },
      ],
    });
    render(<TodoDetailPane projectId={PROJECT} todoId={TODO} />);

    const anchor = screen.getByRole("link", { name: "#42 Fix login" });
    const event = fireEvent.click(anchor);

    expect(event).toBe(false); // preventDefault() was called, so the webview never navigates
    expect(openUrlMock).toHaveBeenCalledWith(
      "https://gitlab.example.com/acme/web/-/issues/42",
    );
  });
});
