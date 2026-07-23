import { render, screen } from "@testing-library/react";
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

// xterm needs a real canvas; the pane's header is what's under test.
vi.mock("./TerminalView", () => ({
  TerminalView: () => <div data-testid="terminal-view" />,
}));

import type { ProcessInfo, ProcessKind } from "../ipc/types";
import { useProcessStore } from "../state/processStore";
import { TerminalPane } from "./TerminalPane";

const initialProcess = useProcessStore.getState();

function process(id: string, name: string, kind: ProcessKind): ProcessInfo {
  return {
    id,
    projectId: "proj-1",
    name,
    kind,
    status: { state: "running", pid: 1, since: "2024-04-03T12:00:00Z" },
    restartPolicy: "never",
    command: "echo hi",
  };
}

describe("TerminalPane header actions", () => {
  beforeEach(() => {
    useProcessStore.setState(initialProcess, true);
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
});
