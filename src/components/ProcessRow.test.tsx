import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

// Activity polling is timer-driven and irrelevant to these assertions.
vi.mock("../lib/useAgentActivity", () => ({ useAgentActivity: () => "idle" }));

import type { ProcessInfo, ProcessKind } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { ProcessRow } from "./ProcessRow";

const initialProcess = useProcessStore.getState();

const PROJECT = "proj-1";

function process(id: string, name: string, kind: ProcessKind): ProcessInfo {
  return {
    id,
    projectId: PROJECT,
    name,
    kind,
    status: { state: "notStarted" },
    restartPolicy: "never",
    command: "echo hi",
    worktree: null,
    color: null,
  };
}

describe("ProcessRow worktree badge", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
  });

  it("shows a branch badge for a process running in a worktree", () => {
    seed();
    const p = {
      ...process("a1", "agent", { kind: "agent", adapter: "claude-code" }),
      worktree: "fix-login",
    };
    render(<ProcessRow process={p} />);
    expect(
      screen.getByRole("img", { name: "Runs in worktree fix-login" }),
    ).toBeInTheDocument();
  });

  it("shows no badge for a project-root process", () => {
    seed();
    render(
      <ProcessRow
        process={process("a2", "agent", {
          kind: "agent",
          adapter: "claude-code",
        })}
      />,
    );
    expect(screen.queryByRole("img", { name: /worktree/i })).toBeNull();
  });
});

describe("ProcessRow session colour", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
  });

  it("tints an agent row with its session colour", () => {
    seed();
    const p = {
      ...process("a1", "agent", { kind: "agent", adapter: "claude-code" }),
      color: "#3e63dd",
    };
    render(<ProcessRow process={p} />);
    const row = screen.getByText("agent").closest('[data-session="true"]');
    expect(row).not.toBeNull();
    expect((row as HTMLElement).style.getPropertyValue("--session-color")).toBe(
      "#3e63dd",
    );
  });

  it("leaves a colourless process untinted", () => {
    seed();
    render(
      <ProcessRow process={process("t1", "term", { kind: "terminal" })} />,
    );
    expect(screen.getByText("term").closest("[data-session]")).toBeNull();
  });
});

function seed() {
  const renameProcess = vi.fn(() => Promise.resolve());
  useProcessStore.setState(
    { ...initialProcess, renameProcess, setActiveProcess: vi.fn() },
    true,
  );
  return { renameProcess };
}

describe("ProcessRow rename", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
  });

  it("renames a terminal via the edit button and Enter", () => {
    const { renameProcess } = seed();
    render(
      <ProcessRow process={process("t1", "term", { kind: "terminal" })} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Rename term" }));
    const input = screen.getByDisplayValue("term");
    fireEvent.change(input, { target: { value: "My Terminal" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(renameProcess).toHaveBeenCalledWith("t1", "My Terminal");
  });

  it("cancels the rename on Escape without calling the store", () => {
    const { renameProcess } = seed();
    render(
      <ProcessRow
        process={process("a1", "claude", {
          kind: "agent",
          adapter: "claude-code",
        })}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Rename claude" }));
    const input = screen.getByDisplayValue("claude");
    fireEvent.change(input, { target: { value: "renamed" } });
    fireEvent.keyDown(input, { key: "Escape" });

    expect(renameProcess).not.toHaveBeenCalled();
    expect(screen.getByText("claude")).toBeInTheDocument();
  });

  it("does not offer rename for config-owned services", () => {
    seed();
    render(<ProcessRow process={process("s1", "dev", { kind: "service" })} />);

    expect(
      screen.queryByRole("button", { name: "Rename dev" }),
    ).not.toBeInTheDocument();
  });
});

describe("ProcessRow lifecycle actions", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
  });

  it("offers start and restart for services", () => {
    seed();
    render(<ProcessRow process={process("s1", "dev", { kind: "service" })} />);

    expect(screen.getByRole("button", { name: "Start dev" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Restart dev" })).toBeVisible();
  });

  it("hides start/stop/restart for agents, keeping remove", () => {
    seed();
    render(
      <ProcessRow
        process={process("a1", "claude", {
          kind: "agent",
          adapter: "claude-code",
        })}
      />,
    );

    expect(screen.queryByRole("button", { name: "Start claude" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop claude" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Restart claude" })).toBeNull();
    expect(screen.getByRole("button", { name: "Remove claude" })).toBeVisible();
  });
});
