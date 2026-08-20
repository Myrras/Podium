import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The modal fetches justfile recipes over IPC; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) =>
    cmd === "project_just_recipes"
      ? Promise.resolve(["build", "test"])
      : Promise.resolve([]),
  ),
}));

import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { TerminalCommandsModal } from "./TerminalCommandsModal";

const initial = useTerminalCommandsStore.getState();
const ROOT = "/tmp/fixture-project";

function renderModal(onRun = vi.fn(), onClose = vi.fn()) {
  render(
    <TerminalCommandsModal
      open
      projectId="proj-1"
      root={ROOT}
      onClose={onClose}
      onRun={onRun}
    />,
  );
  return { onRun, onClose };
}

beforeEach(() => {
  localStorage.clear();
  useTerminalCommandsStore.setState({ ...initial, customByRoot: {} }, true);
});

describe("TerminalCommandsModal", () => {
  it("lists the project's justfile recipes", async () => {
    renderModal();
    await waitFor(() =>
      expect(screen.getByText("just build")).toBeInTheDocument(),
    );
    expect(screen.getByText("just test")).toBeInTheDocument();
  });

  it("runs a command and closes", async () => {
    const { onRun, onClose } = renderModal();
    fireEvent.click(
      await screen.findByTitle('Run "just build" in a new terminal'),
    );
    expect(onRun).toHaveBeenCalledWith("just build");
    expect(onClose).toHaveBeenCalled();
  });

  it("adds and removes a custom command, persisting per project root", async () => {
    renderModal();
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
    renderModal();
    await waitFor(() =>
      expect(screen.getByText(/no justfile recipes/i)).toBeInTheDocument(),
    );
  });
});
