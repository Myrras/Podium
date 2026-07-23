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
  };
}

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
