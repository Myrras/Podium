import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { ProcessInfo } from "../ipc/types";
import { useLayoutStore } from "./layoutStore";
import { useProcessStore } from "./processStore";

const initialProcess = useProcessStore.getState();
const initialLayout = useLayoutStore.getState();

function agent(overrides: Partial<ProcessInfo> = {}): ProcessInfo {
  return {
    id: "00000000-0000-4000-8000-000000000001",
    projectId: "proj-1",
    name: "export-agent",
    kind: { kind: "agent", adapter: "claude-code" },
    status: { state: "running", pid: 4242, since: "2024-04-03T12:00:00Z" },
    restartPolicy: "never",
    command: "claude",
    worktree: null,
    color: null,
    ...overrides,
  } as ProcessInfo;
}

describe("processStore.resumeTodoSession", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
    useLayoutStore.setState(initialLayout, true);
    invokeMock.mockReset();
  });

  it("resumes the session and focuses its agent over the open to-do", async () => {
    const info = agent();
    invokeMock.mockResolvedValue(info);
    useLayoutStore.setState({
      openTodo: { projectId: "proj-1", todoId: "todo-1" },
    });

    const result = await useProcessStore
      .getState()
      .resumeTodoSession("proj-1", "todo-1", info.id);

    expect(invokeMock).toHaveBeenCalledWith("todo_session_resume", {
      projectId: "proj-1",
      todoId: "todo-1",
      sessionId: info.id,
    });
    expect(result).toEqual(info);
    expect(useProcessStore.getState().activeProcessId).toBe(info.id);
    expect(useProcessStore.getState().processes).toEqual([info]);
    expect(useLayoutStore.getState().openTodo).toBeNull();
  });

  it("replaces an exited row that the resume restarted in place", async () => {
    const exited = agent({
      status: {
        state: "exited",
        code: 0,
        crashed: false,
        stopped: true,
      } as unknown as ProcessInfo["status"],
    });
    const running = agent();
    useProcessStore.setState({ processes: [exited] });
    invokeMock.mockResolvedValue(running);

    await useProcessStore
      .getState()
      .resumeTodoSession("proj-1", "todo-1", running.id);

    expect(useProcessStore.getState().processes).toEqual([running]);
  });

  it("returns null and focuses nothing when the resume fails", async () => {
    invokeMock.mockRejectedValue({
      kind: "sessionNotFound",
      message: "that session's conversation is no longer stored",
    });

    const result = await useProcessStore
      .getState()
      .resumeTodoSession("proj-1", "todo-1", "s-1");

    expect(result).toBeNull();
    expect(useProcessStore.getState().activeProcessId).toBeNull();
  });
});
