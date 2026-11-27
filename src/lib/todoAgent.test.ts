import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../ipc/commands", () => ({}));

import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { startAgentOnTodo } from "./todoAgent";

const todo = { id: "todo-1", text: "Fix the login form" };
const spawnAgent = vi.fn(() => Promise.resolve());
const pick = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  useProcessStore.setState({ spawnAgent } as never);
  useProjectStore.setState({ activeProjectId: null });
});

describe("startAgentOnTodo", () => {
  it("spawns the default agent on a plain click and from a menu", () => {
    startAgentOnTodo("proj-1", todo, { metaKey: false, ctrlKey: false }, pick);
    startAgentOnTodo("proj-1", todo);
    expect(spawnAgent).toHaveBeenCalledTimes(2);
    expect(spawnAgent).toHaveBeenCalledWith("proj-1", { todoIds: ["todo-1"] });
    expect(pick).not.toHaveBeenCalled();
    expect(useProjectStore.getState().activeProjectId).toBe("proj-1");
  });

  it.each([
    ["Cmd", { metaKey: true, ctrlKey: false }],
    ["Ctrl", { metaKey: false, ctrlKey: true }],
  ])("opens the New agent screen on %s-click", (_, click) => {
    startAgentOnTodo("proj-1", todo, click, pick);
    expect(pick).toHaveBeenCalledWith(
      "proj-1",
      ["todo-1"],
      "Fix the login form",
    );
    expect(spawnAgent).not.toHaveBeenCalled();
    expect(useProjectStore.getState().activeProjectId).toBe("proj-1");
  });
});
