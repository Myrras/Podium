import { act, fireEvent, render, screen, within } from "@testing-library/react";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
// xterm does not run in jsdom; a textarea stands in for its focus target.
// The mock renders the bar the modal hands over and keeps the last props.
let paneProps: ComponentProps<typeof TerminalPane>;
// Names of the processes a pane mounted for, in order.
const mounts: string[] = [];
vi.mock("../TerminalPane", async () => {
  const { useState } = await import("react");
  return { TerminalPane: MockPane };
  function MockPane(props: ComponentProps<typeof TerminalPane>) {
    paneProps = props;
    // The initialiser runs once per mount.
    useState(() => mounts.push(props.process.name));
    return (
      <div>
        {props.leading}
        <span>Pane of {props.process.name}</span>
        {props.trailing}
        <textarea aria-label="terminal input" />
      </div>
    );
  }
});

import { invoke } from "@tauri-apps/api/core";

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
import type { TerminalPane } from "../TerminalPane";
import { FloorTerminalModal } from "./FloorTerminalModal";

const RUNNING: ProcessStatus = { state: "running", pid: 1, since: "now" };
const EXITED: ProcessStatus = {
  state: "exited",
  code: 0,
  crashed: false,
  stopped: true,
  at: "2026-01-01T10:00:00Z",
};
const NOT_STARTED: ProcessStatus = { state: "notStarted" };
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
const restartProcess = vi.fn((_id: string) => Promise.resolve());
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
      restartProcess,
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

const confirmDialog = () => screen.getByRole("alertdialog");
// The header's buttons, in order (the mocked pane renders leading, then trailing).
const names = () =>
  screen
    .getAllByRole("button")
    .map((b) => b.getAttribute("aria-label") ?? b.textContent);

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
    const hide = screen.getByRole("button", { name: "Hide Fix login" });
    expect(hide).toHaveAttribute("title", "Hide (keeps running)");
    fireEvent.click(hide);
    expect(useFloorStore.getState().terminalId).toBeNull();
    expect(stopProcess).not.toHaveBeenCalled();
    expect(removeProcess).not.toHaveBeenCalled();
  });

  it("does not close on Esc with focus on a header button", () => {
    render(<FloorTerminalModal />);
    open("a1");
    const close = screen.getByRole("button", { name: "Hide Fix login" });
    close.focus();
    fireEvent.keyDown(close, { key: "Escape" });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(useFloorStore.getState().terminalId).toBe("a1");
  });

  it("keeps Tab inside the overlay", () => {
    render(<FloorTerminalModal />);
    open("a1");
    const first = screen.getByRole("button", { name: "Stop Fix login" });
    const last = screen.getByLabelText("terminal input");

    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(first).toHaveFocus();

    fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
    expect(last).toHaveFocus();
  });

  it("shows the name once: only the pane's bar, no header of its own", () => {
    render(<FloorTerminalModal />);
    open("a1");
    expect(screen.getAllByText(/Fix login/)).toHaveLength(1);
    expect(screen.queryByRole("banner")).toBeNull();
    expect(paneProps.hideActions).toBe(true);
  });

  it("asks before stopping a running agent", () => {
    render(<FloorTerminalModal />);
    open("a1");
    expect(
      screen.queryByRole("button", { name: "Resume Fix login" }),
    ).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Stop Fix login" }));
    expect(confirmDialog()).toHaveAccessibleName("Stop Fix login?");
    expect(confirmDialog()).toHaveAccessibleDescription(
      "The session ends. You can resume it later.",
    );
    expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(stopProcess).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Stop Fix login" }));
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Stop" }),
    );
    expect(stopProcess).toHaveBeenCalledWith("a1");
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("does not stop an agent that exited while the Stop confirm was open", () => {
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Stop Fix login" }));
    act(() =>
      useProcessStore.setState({
        processes: [proc("a1", "Fix login", EXITED)],
      }),
    );
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Stop" }),
    );
    expect(stopProcess).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("keeps the modal open when Close cannot stop the process", async () => {
    vi.mocked(invoke).mockImplementationOnce(() =>
      Promise.reject({ kind: "other", message: "nope" }),
    );
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Close agent" }));
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Close" }),
    );
    await act(async () => {});
    expect(removeProcess).not.toHaveBeenCalled();
    expect(useFloorStore.getState().terminalId).toBe("a1");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("cancels the confirm on Esc without closing the modal", () => {
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Stop Fix login" }));
    const notCancelled = fireEvent.keyDown(
      screen.getByRole("button", { name: "Cancel" }),
      { key: "Escape" },
    );
    document.removeEventListener("keydown", outer);

    expect(notCancelled).toBe(false);
    expect(outer).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(stopProcess).not.toHaveBeenCalled();
    expect(useFloorStore.getState().terminalId).toBe("a1");
  });

  it("keeps Tab inside the confirm", () => {
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Stop Fix login" }));
    const cancel = screen.getByRole("button", { name: "Cancel" });
    const stop = within(confirmDialog()).getByRole("button", { name: "Stop" });

    fireEvent.keyDown(stop, { key: "Tab" });
    expect(cancel).toHaveFocus();
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(stop).toHaveFocus();
  });

  it("gives a running agent Stop, Close agent and ×, no Restart", () => {
    render(<FloorTerminalModal />);
    open("a1");
    expect(names()).toEqual([
      "Stop Fix login",
      "Close agent",
      "Hide Fix login",
    ]);
  });

  it("resumes an ended or never-started agent without asking", () => {
    for (const status of [EXITED, NOT_STARTED]) {
      seed([proc("a1", "Fix login", status)]);
      const { unmount } = render(<FloorTerminalModal />);
      open("a1");
      expect(names()).toEqual([
        "Resume Fix login",
        "Close agent",
        "Hide Fix login",
      ]);
      fireEvent.click(screen.getByRole("button", { name: "Resume Fix login" }));
      expect(startProcess).toHaveBeenCalledWith("a1");
      expect(screen.queryByRole("alertdialog")).toBeNull();
      unmount();
      act(() => useFloorStore.getState().closeTerminal());
    }
  });

  it("closes a running agent after asking: stop, remove, hide", async () => {
    const order: string[] = [];
    // Close stops through the IPC, so a failed stop can keep the process.
    vi.mocked(invoke).mockImplementationOnce(async (cmd) => {
      order.push(cmd === "process_stop" ? "stop" : cmd);
    });
    removeProcess.mockImplementationOnce(async () => void order.push("remove"));
    render(<FloorTerminalModal />);
    open("a1");
    fireEvent.click(screen.getByRole("button", { name: "Close agent" }));
    expect(confirmDialog()).toHaveAccessibleName("Close Fix login?");
    expect(confirmDialog()).toHaveAccessibleDescription(
      "The agent stops and its desk is removed from the floor.",
    );
    expect(removeProcess).not.toHaveBeenCalled();
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Close" }),
    );
    await act(async () => {});
    expect(order).toEqual(["stop", "remove"]);
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("closes an ended or never-started agent by removing it only", async () => {
    for (const status of [EXITED, NOT_STARTED]) {
      vi.clearAllMocks();
      seed([proc("a1", "Fix login", status)]);
      const { unmount } = render(<FloorTerminalModal />);
      open("a1");
      fireEvent.click(screen.getByRole("button", { name: "Close agent" }));
      fireEvent.click(
        within(confirmDialog()).getByRole("button", { name: "Close" }),
      );
      await act(async () => {});
      expect(invoke).not.toHaveBeenCalled();
      expect(removeProcess).toHaveBeenCalledWith("a1");
      expect(useFloorStore.getState().terminalId).toBeNull();
      unmount();
    }
  });

  it("renames through the pane; a blank name keeps the old one", () => {
    render(<FloorTerminalModal />);
    open("a1");
    act(() => paneProps.onRename?.(null));
    expect(renameProcess).not.toHaveBeenCalled();
    act(() => paneProps.onRename?.("Fix signup"));
    expect(renameProcess).toHaveBeenCalledWith("a1", "Fix signup");
  });

  it("mounts a fresh pane per shown process, so no rename carries over", () => {
    mounts.length = 0;
    render(<FloorTerminalModal />);
    open("a1");
    open("t1");
    expect(screen.getByText("Pane of Shell")).toBeInTheDocument();
    expect(mounts).toEqual(["Fix login", "Shell"]);
  });

  it("gives a running terminal Stop, Restart, Close terminal and ×", () => {
    render(<FloorTerminalModal />);
    open("t1");
    expect(names()).toEqual([
      "Stop Shell",
      "Restart Shell",
      "Close terminal",
      "Hide Shell",
    ]);
  });

  it("starts a stopped or never-started terminal without asking", () => {
    seed([proc("t2", "Build", NOT_STARTED, { kind: "terminal" })]);
    const { unmount } = render(<FloorTerminalModal />);
    open("t2");
    expect(names()).toEqual(["Start Build", "Close terminal", "Hide Build"]);
    unmount();

    seed([proc("t1", "Shell", EXITED, { kind: "terminal" })]);
    render(<FloorTerminalModal />);
    open("t1");
    expect(names()).toEqual(["Start Shell", "Close terminal", "Hide Shell"]);
    fireEvent.click(screen.getByRole("button", { name: "Start Shell" }));
    expect(startProcess).toHaveBeenCalledWith("t1");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(paneProps.onRename).toBeDefined();
  });

  it("stops a running terminal after asking and restarts it directly", () => {
    render(<FloorTerminalModal />);
    open("t1");
    fireEvent.click(screen.getByRole("button", { name: "Restart Shell" }));
    expect(restartProcess).toHaveBeenCalledWith("t1");

    fireEvent.click(screen.getByRole("button", { name: "Stop Shell" }));
    expect(stopProcess).not.toHaveBeenCalled();
    expect(confirmDialog()).toHaveAccessibleDescription(
      "The process stops. You can start it again later.",
    );
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Stop" }),
    );
    expect(stopProcess).toHaveBeenCalledWith("t1");
  });

  it("closes a running terminal fully after asking: stop, remove, close", async () => {
    const order: string[] = [];
    // Close stops through the IPC, so a failed stop can keep the process.
    vi.mocked(invoke).mockImplementationOnce(async (cmd) => {
      order.push(cmd === "process_stop" ? "stop" : cmd);
    });
    removeProcess.mockImplementationOnce(async () => void order.push("remove"));
    render(<FloorTerminalModal />);
    open("t1");
    fireEvent.click(screen.getByRole("button", { name: "Close terminal" }));
    expect(confirmDialog()).toHaveAccessibleName("Close Shell?");
    expect(confirmDialog()).toHaveAccessibleDescription(
      "The terminal stops and is removed, with its output.",
    );
    expect(removeProcess).not.toHaveBeenCalled();
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Close" }),
    );
    await act(async () => {});
    expect(order).toEqual(["stop", "remove"]);
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("does not stop a stopped terminal before removing it", async () => {
    seed([proc("t1", "Shell", EXITED, { kind: "terminal" })]);
    render(<FloorTerminalModal />);
    open("t1");
    fireEvent.click(screen.getByRole("button", { name: "Close terminal" }));
    fireEvent.click(
      within(confirmDialog()).getByRole("button", { name: "Close" }),
    );
    await act(async () => {});
    expect(invoke).not.toHaveBeenCalled();
    expect(removeProcess).toHaveBeenCalledWith("t1");
  });

  it("gives a service Stop and Restart but no Close", () => {
    seed([proc("s1", "Web", RUNNING, { kind: "service" } as ProcessKind)]);
    const { unmount } = render(<FloorTerminalModal />);
    open("s1");
    expect(names()).toEqual(["Stop Web", "Restart Web", "Hide Web"]);
    unmount();

    seed([proc("s1", "Web", EXITED, { kind: "service" } as ProcessKind)]);
    render(<FloorTerminalModal />);
    open("s1");
    expect(names()).toEqual(["Start Web", "Hide Web"]);
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
    const next = screen.getByRole("button", { name: "Next: Add tests" });
    expect(next).toHaveAttribute("title", "Next that needs you: Add tests");
    fireEvent.click(next);
    expect(useFloorStore.getState().terminalId).toBe("w3");
    expect(setActiveProcess).toHaveBeenCalledWith("w3");
    expect(
      screen.getByRole("button", { name: "Next: Write docs" }),
    ).toBeInTheDocument();
  });

  it("hides Next when nobody else needs you", () => {
    seed([proc("a1", "Fix login")], ["a1"]);
    render(<FloorTerminalModal />);
    open("a1");
    expect(screen.queryByRole("button", { name: /^Next/ })).toBeNull();
  });

  it("closes when the process is removed elsewhere", () => {
    render(<FloorTerminalModal />);
    open("a1");
    act(() => useProcessStore.setState({ processes: [] }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useFloorStore.getState().terminalId).toBeNull();
  });

  it("gives an open assistant sub-agent only ×", () => {
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
    expect(names()).toEqual(["Hide Repo sweep"]);
    expect(paneProps.leading).toBeUndefined();
    expect(paneProps.onRename).toBeUndefined();

    fireEvent.click(screen.getByRole("button", { name: "Hide Repo sweep" }));
    expect(useLayoutStore.getState().openSubagent).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
