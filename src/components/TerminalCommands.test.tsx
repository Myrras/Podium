import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The component fetches justfile recipes over IPC; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) =>
    cmd === "project_just_recipes"
      ? Promise.resolve(["build", "test"])
      : Promise.resolve([]),
  ),
}));

import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { TerminalCommands } from "./TerminalCommands";

const initial = useTerminalCommandsStore.getState();
const ROOT = "/tmp/fixture-project";

function renderCommands(onRun = vi.fn()) {
  render(<TerminalCommands projectId="proj-1" root={ROOT} onRun={onRun} />);
  return onRun;
}

beforeEach(() => {
  localStorage.clear();
  useTerminalCommandsStore.setState(
    { ...initial, customByRoot: {}, expandedByRoot: {} },
    true,
  );
});

describe("TerminalCommands", () => {
  it("starts collapsed and expands to show justfile recipes", async () => {
    renderCommands();
    const header = screen.getByRole("button", { name: /commands/i });
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("just build")).not.toBeInTheDocument();

    fireEvent.click(header);
    expect(header).toHaveAttribute("aria-expanded", "true");
    await waitFor(() =>
      expect(screen.getByText("just build")).toBeInTheDocument(),
    );
    expect(screen.getByText("just test")).toBeInTheDocument();
  });

  it("runs a command via the onRun callback", async () => {
    const onRun = renderCommands();
    fireEvent.click(screen.getByRole("button", { name: /commands/i }));
    const row = await screen.findByTitle('Run "just build" in a new terminal');
    fireEvent.click(row);
    expect(onRun).toHaveBeenCalledWith("just build");
  });

  it("adds and removes a custom command, persisting per project root", async () => {
    renderCommands();
    fireEvent.click(screen.getByRole("button", { name: "Add command" }));
    const input = await screen.findByLabelText("New command");
    fireEvent.change(input, { target: { value: "pnpm dev" } });
    fireEvent.submit(input.closest("form")!);

    expect(screen.getByText("pnpm dev")).toBeInTheDocument();
    expect(useTerminalCommandsStore.getState().customByRoot[ROOT]).toEqual([
      "pnpm dev",
    ]);
    expect(localStorage.getItem("podium.terminalCommands")).toContain(
      "pnpm dev",
    );

    fireEvent.click(
      screen.getByRole("button", { name: 'Remove command "pnpm dev"' }),
    );
    expect(screen.queryByText("pnpm dev")).not.toBeInTheDocument();
    expect(useTerminalCommandsStore.getState().customByRoot[ROOT]).toEqual([]);
  });

  it("shows a hint when there are no commands at all", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockResolvedValueOnce([]);
    renderCommands();
    fireEvent.click(screen.getByRole("button", { name: /commands/i }));
    await waitFor(() =>
      expect(screen.getByText(/no justfile recipes/i)).toBeInTheDocument(),
    );
  });
});
