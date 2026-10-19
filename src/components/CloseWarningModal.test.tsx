import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(undefined)),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import { invoke } from "@tauri-apps/api/core";

import type {
  ProcessInfo,
  ProcessKind,
  ProcessStatus,
  SubagentInfo,
} from "../ipc/types";
import { useAssistantStore } from "../state/assistantStore";
import { useProcessStore } from "../state/processStore";
import { useProjectStore } from "../state/projectStore";
import { CloseWarningModal } from "./CloseWarningModal";

const initialProcess = useProcessStore.getState();
const initialProject = useProjectStore.getState();
const initialAssistant = useAssistantStore.getState();

function subagent(overrides: Partial<SubagentInfo> = {}): SubagentInfo {
  return {
    id: "sub-1",
    name: "Ticket check",
    kind: "lookup",
    task: "Ticket check",
    phase: "running",
    status: running,
    createdAt: "2020",
    startedAt: "2020",
    endedAt: null,
    result: null,
    error: null,
    ...overrides,
  };
}

const running: ProcessStatus = { state: "running", pid: 1, since: "2020" };
const exited: ProcessStatus = {
  state: "exited",
  code: 0,
  crashed: false,
  stopped: false,
  at: "2020",
};

function proc(
  id: string,
  name: string,
  kind: ProcessKind,
  status: ProcessStatus,
): ProcessInfo {
  return {
    id,
    projectId: "p1",
    name,
    kind,
    status,
    restartPolicy: "never",
    command: name,
    worktree: null,
    color: null,
  };
}

describe("CloseWarningModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useProcessStore.setState(initialProcess, true);
    useProjectStore.setState(initialProject, true);
    useAssistantStore.setState(initialAssistant, true);
    useProjectStore.setState({
      projects: [
        {
          id: "p1",
          name: "Webshop",
          root: "/tmp/webshop",
          iconInitials: "WS",
          configError: null,
          renamed: false,
          sections: {
            todos: true,
            terminals: true,
            worktrees: true,
            scratchpads: true,
            issues: false,
          },
          color: null,
          nativeInWslMode: false,
        },
      ],
    });
  });

  it("lists only running agents and terminals", () => {
    useProcessStore.setState({
      processes: [
        proc(
          "a1",
          "claude",
          { kind: "agent", adapter: "claude-code" },
          running,
        ),
        proc("t1", "zsh", { kind: "terminal" }, running),
        proc("s1", "dev-server", { kind: "service" }, running),
        proc(
          "a2",
          "old-agent",
          { kind: "agent", adapter: "claude-code" },
          exited,
        ),
      ],
    });
    render(<CloseWarningModal open onClose={() => undefined} />);

    expect(screen.getByText("claude")).toBeInTheDocument();
    expect(screen.getByText("zsh")).toBeInTheDocument();
    // Services and non-running processes are excluded.
    expect(screen.queryByText("dev-server")).not.toBeInTheDocument();
    expect(screen.queryByText("old-agent")).not.toBeInTheDocument();
  });

  it("confirms the close via windowConfirmClose", async () => {
    useProcessStore.setState({
      processes: [proc("t1", "zsh", { kind: "terminal" }, running)],
    });
    render(<CloseWarningModal open onClose={() => undefined} />);

    fireEvent.click(screen.getByRole("button", { name: "Close anyway" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("window_confirm_close"),
    );
  });

  it("cancels without closing the app", () => {
    const onClose = vi.fn();
    useProcessStore.setState({
      processes: [proc("t1", "zsh", { kind: "terminal" }, running)],
    });
    render(<CloseWarningModal open onClose={onClose} />);

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalledWith("window_confirm_close");
  });

  it("names the assistant's sub-agents when only they are running", () => {
    useAssistantStore.setState({
      subagents: [
        subagent(),
        subagent({
          id: "sub-2",
          name: "Repo sweep",
          kind: "full",
          phase: "queued",
          status: { state: "notStarted" },
          startedAt: null,
        }),
        // Ended ones are not interrupted by a quit.
        subagent({ id: "sub-3", name: "Old lookup", phase: "done" }),
      ],
    });
    render(<CloseWarningModal open onClose={() => undefined} />);

    expect(
      screen.getByText(
        "Closing Podium will stop the assistant's 2 sub-agents. Their work will be interrupted.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Ticket check")).toBeInTheDocument();
    expect(
      screen.getByText("Assistant · lookup sub-agent"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Assistant · full sub-agent · queued"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Old lookup")).not.toBeInTheDocument();
  });

  it("counts sub-agents alongside agents and terminals", () => {
    useProcessStore.setState({
      processes: [proc("t1", "zsh", { kind: "terminal" }, running)],
    });
    useAssistantStore.setState({ subagents: [subagent()] });
    render(<CloseWarningModal open onClose={() => undefined} />);

    expect(
      screen.getByText(
        "Closing Podium will stop these 2 processes. Any in-progress work will be interrupted.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("zsh")).toBeInTheDocument();
    expect(screen.getByText("Ticket check")).toBeInTheDocument();
  });
});
