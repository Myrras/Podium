import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { TodoInfo } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
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
});
