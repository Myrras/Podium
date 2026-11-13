import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
// xterm does not run in jsdom; a textarea stands in for its focus target.
vi.mock("../TerminalPane", () => ({
  TerminalPane: ({ process }: { process: { name: string } }) => (
    <div>
      Pane of {process.name}
      <textarea aria-label="terminal input" />
    </div>
  ),
}));

import type {
  ProcessInfo,
  ProcessKind,
  ProcessStatus,
  SubagentInfo,
} from "../../ipc/types";
import {
  UNTRACKED_ACTIVITY,
  useAgentActivityStore,
} from "../../state/agentActivityStore";
import { useAssistantStore } from "../../state/assistantStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { FloorTerminalModal } from "./FloorTerminalModal";

const RUNNING: ProcessStatus = { state: "running", pid: 1, since: "now" };
const EXITED: ProcessStatus = {
  state: "exited",
  code: 0,
  crashed: false,
  stopped: true,
  at: "2026-01-01T10:00:00Z",
};
const AGENT: ProcessKind = { kind: "agent", adapter: "claude-code" };

function proc(
  id: string,
  name: string,
  status: ProcessStatus = RUNNING,
  kind: ProcessKind = AGENT,
): ProcessInfo {
  return {
    id,
    projectId: "proj-1",
    name,
    kind,
    status,
    restartPolicy: "never",
    command: "run",
    worktree: null,
    color: null,
    viewing: false,
  };
}

const renameProcess = vi.fn((_id: string, _name: string) => Promise.resolve());
const startProcess = vi.fn((_id: string) => Promise.resolve());
const stopProcess = vi.fn((_id: string) => Promise.resolve());
const removeProcess = vi.fn((_id: string) => Promise.resolve());
const setActiveProcess = vi.fn();

function seed(processes: ProcessInfo[], waiting: string[] = []) {
  useProcessStore.setState(
    {
      ...useProcessStore.getInitialState(),
      processes,
      renameProcess,
      startProcess,
      stopProcess,
      removeProcess,
      setActiveProcess,
    },
    true,
  );
  useAgentActivityStore.setState(
    {
      ...useAgentActivityStore.getInitialState(),
      details: Object.fromEntries(
        waiting.map((id) => [
          id,
          { ...UNTRACKED_ACTIVITY, activity: "waiting" as const },
        ]),
      ),
    },
    true,
  );
}

function open(id: string) {
  act(() => useFloorStore.getState().openTerminal(id));
}

beforeEach(() => {
  vi.clearAllMocks();
  useFloorStore.setState(useFloorStore.getInitialState(), true);
  useLayoutStore.setState({ openSubagent: null });
  useAssistantStore.setState({ subagents: [] });
  seed([
    proc("a1", "Fix login"),
    proc("t1", "Shell", RUNNING, { kind: "terminal" }),
  ]);
});

describe("FloorTerminalModal", () => {
  it("renders nothing while no terminal is open", () => {
    render(<FloorTerminalModal />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the process's pane and closes with × or a click beside it", () => {
    render(<FloorTerminalModal />);
    open("a1");
    expect(
      screen.getByRole("dialog", { name: "Fix login" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Pane of Fix login")).toBeInTheDocument();

    fireEvent.mouseDown(screen.getByRole("dialog"));
    expect(useFloorStore.getState().terminalId).toBe("a1");
    fireEvent.mouseDown(screen.getByRole("presentation"));
    expect(useFloorStore.getState().terminalId).toBeNull();

    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Close terminal" }));
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("does not close on Esc with focus on a header button", () => {
    render(<FloorTerminalModal />);
    open("a1");
    const close = screen.getByRole("button", { name: "Close terminal" });
    close.focus();
    fireEvent.keyDown(close, { key: "Escape" });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(useFloorStore.getState().terminalId).toBe("a1");
  });

  it("keeps Tab inside the overlay", () => {
    render(<FloorTerminalModal />);
    open("a1");
    const first = screen.getByRole("button", { name: "Fix login" });
    const last = screen.getByLabelText("terminal input");

    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(first).toHaveFocus();

    fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();
  });

  it("stops a running agent", () => {
    render(<FloorTerminalModal />);
    open("a1");
    expect(screen.queryByRole("button", { name: "Resume session" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(stopProcess).toHaveBeenCalledWith("a1");
  });

  it("resumes or clears an ended agent", async () => {
    seed([proc("a1", "Fix login", EXITED)]);
    render(<FloorTerminalModal />);
    open("a1");
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Resume session" }));
    expect(startProcess).toHaveBeenCalledWith("a1");

    fireEvent.click(screen.getByRole("button", { name: "Clear the desk" }));
    expect(removeProcess).toHaveBeenCalledWith("a1");
    await act(async () => {});
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("renames the agent by clicking its name; Esc only cancels the edit", () => {
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Fix login" }));
    const input = screen.getByLabelText("Agent name");
    fireEvent.change(input, { target: { value: "Draft" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(renameProcess).not.toHaveBeenCalled();
    expect(useFloorStore.getState().terminalId).toBe("a1");

    fireEvent.click(screen.getByRole("button", { name: "Fix login" }));
    fireEvent.change(screen.getByLabelText("Agent name"), {
      target: { value: "  Fix signup " },
    });
    fireEvent.keyDown(screen.getByLabelText("Agent name"), { key: "Enter" });
    expect(renameProcess).toHaveBeenCalledWith("a1", "Fix signup");
  });

  it("has no agent actions for a terminal", () => {
    render(<FloorTerminalModal />);
    open("t1");
    expect(screen.getByText("Shell")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Shell" })).toBeNull();
  });

  it("steps to the next agent that needs you, wrapping round", () => {
    seed(
      [
        proc("w0", "Write docs"),
        proc("a1", "Fix login"),
        proc("i2", "Idle one"),
        proc("w3", "Add tests"),
        proc("t4", "Waiting shell", RUNNING, { kind: "terminal" }),
      ],
      ["w0", "w3", "t4"],
    );
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(
      screen.getByRole("button", { name: "Next that needs you: Add tests" }),
    );
    expect(useFloorStore.getState().terminalId).toBe("w3");
    expect(setActiveProcess).toHaveBeenCalledWith("w3");
    expect(
      screen.getByRole("button", { name: "Next that needs you: Write docs" }),
    ).toBeInTheDocument();
  });

  it("hides Next when nobody else needs you", () => {
    seed([proc("a1", "Fix login")], ["a1"]);
    render(<FloorTerminalModal />);
    open("a1");
    expect(screen.queryByText(/Next that needs you/)).toBeNull();
  });

  it("closes when the process is removed elsewhere", () => {
    render(<FloorTerminalModal />);
    open("a1");
    act(() => useProcessStore.setState({ processes: [] }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("shows an open assistant sub-agent without agent actions", () => {
    const sub: SubagentInfo = {
      id: "sub-1",
      kind: "full",
      name: "Repo sweep",
      task: "Repo sweep",
      phase: "running",
      status: RUNNING,
      createdAt: "2026-01-01T10:00:00Z",
      startedAt: "2026-01-01T10:00:00Z",
      endedAt: null,
      expiresAt: null,
      result: null,
      error: null,
      question: null,
    };
    seed([proc("a1", "Fix login"), proc("w2", "Write docs")], ["w2"]);
    useAssistantStore.setState({ subagents: [sub] });
    render(<FloorTerminalModal />);
    act(() => useLayoutStore.setState({ openSubagent: "sub-1" }));

    expect(screen.getByText("Pane of Repo sweep")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    expect(screen.queryByText(/Next that needs you/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close terminal" }));
    expect(useLayoutStore.getState().openSubagent).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
