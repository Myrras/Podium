import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Per-command invoke stub; `process_git_branch` is overridable per test.
let gitBranch: string | null = null;

// The store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) =>
    cmd === "process_git_branch"
      ? Promise.resolve(gitBranch)
      : Promise.resolve([]),
  ),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

// xterm needs a real canvas; the pane's header is what's under test.
vi.mock("./TerminalView", () => ({
  TerminalView: () => <div data-testid="terminal-view" />,
}));

import { invoke } from "@tauri-apps/api/core";

import type { ProcessInfo, ProcessKind } from "../ipc/types";
import { useAgentActivityStore } from "../state/agentActivityStore";
import { useProcessStore } from "../state/processStore";
import { TerminalPane } from "./TerminalPane";

const initialProcess = useProcessStore.getState();
const initialActivity = useAgentActivityStore.getState();

function process(id: string, name: string, kind: ProcessKind): ProcessInfo {
  return {
    id,
    projectId: "proj-1",
    name,
    kind,
    status: { state: "running", pid: 1, since: "2024-04-03T12:00:00Z" },
    restartPolicy: "never",
    command: "echo hi",
    worktree: null,
    color: null,
    viewing: false,
  };
}

describe("TerminalPane header actions", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
    useAgentActivityStore.setState(initialActivity, true);
    gitBranch = null;
  });

  it("offers stop and restart for services", () => {
    render(
      <TerminalPane process={process("s1", "dev", { kind: "service" })} />,
    );

    expect(screen.getByRole("button", { name: "Stop dev" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Restart dev" })).toBeVisible();
  });

  it("hides start/stop/restart for agents", () => {
    render(
      <TerminalPane
        process={process("a1", "claude", {
          kind: "agent",
          adapter: "claude-code",
        })}
      />,
    );

    expect(screen.queryByRole("button", { name: "Start claude" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop claude" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Restart claude" })).toBeNull();
  });

  it("shows the git branch and worktree in the header", async () => {
    gitBranch = "podium/fix-login";
    const proc = process("a1", "claude", {
      kind: "agent",
      adapter: "claude-code",
    });
    proc.worktree = "fix-login";
    render(<TerminalPane process={proc} />);

    expect(await screen.findByText("podium/fix-login")).toBeVisible();
    expect(screen.getByText("fix-login")).toBeVisible();
  });

  it("labels a sub-agent and stops it through the assistant", () => {
    const proc = process("sub-1", "Repo sweep", {
      kind: "agent",
      adapter: "claude-code",
    });
    proc.subagent = {
      kind: "full",
      task: "Repo sweep",
      phase: "running",
      endedAt: null,
    };
    render(<TerminalPane process={proc} />);

    expect(screen.getByText("Assistant sub-agent · full")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Stop Repo sweep" }));
    expect(invoke).toHaveBeenCalledWith("assistant_subagent_stop", {
      id: "sub-1",
    });
  });

  it("offers no stop once a sub-agent has ended", () => {
    const proc = process("sub-1", "Repo sweep", {
      kind: "agent",
      adapter: "claude-code",
    });
    proc.subagent = {
      kind: "lookup",
      task: "Repo sweep",
      phase: "done",
      endedAt: "2026-01-01T10:05:00Z",
    };
    render(<TerminalPane process={proc} />);

    expect(screen.getByText("Assistant sub-agent · lookup")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Stop Repo sweep" }),
    ).toBeNull();
  });

  describe("status pill", () => {
    const agent = { kind: "agent", adapter: "claude-code" } as const;
    const seed = (
      id: string,
      activity: "working" | "waiting",
      waitingFor: string | null,
    ) =>
      useAgentActivityStore.setState({
        activity: { [id]: activity },
        details: {
          [id]: {
            activity,
            reported: true,
            waitingFor,
            sinceMs: null,
            subagents: 0,
            shells: 0,
          },
        },
      });

    it("shows Working for a working agent", () => {
      seed("a1", "working", null);
      render(<TerminalPane process={process("a1", "claude", agent)} />);
      expect(screen.getByText("Working")).toBeVisible();
    });

    it("shows Needs you with the reason", () => {
      seed("a1", "waiting", "Allow edit to foo.ts?");
      render(<TerminalPane process={process("a1", "claude", agent)} />);
      expect(screen.getByText("Needs you")).toBeVisible();
      expect(screen.getByText("Allow edit to foo.ts?")).toBeVisible();
    });

    it("shows Exited with the exit detail", () => {
      const proc = process("s1", "dev", { kind: "service" });
      proc.status = {
        state: "exited",
        code: 0,
        crashed: false,
        stopped: false,
        at: "2024-04-03T12:05:00Z",
      };
      render(<TerminalPane process={proc} />);
      expect(screen.getByText("Exited")).toBeVisible();
    });
  });
});
