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

import type { WorktreeInfo } from "../ipc/types";
import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { TerminalCommandsModal } from "./TerminalCommandsModal";

const initial = useTerminalCommandsStore.getState();
const ROOT = "/tmp/fixture-project";
const PROJECT = "proj-1";

function wt(name: string): WorktreeInfo {
  return {
    name,
    path: `/repo/.podium/worktrees/${name}`,
    branch: `podium/${name}`,
    inUse: false,
  };
}

function renderModal(
  onRun = vi.fn(),
  onClose = vi.fn(),
  worktree?: WorktreeInfo | null,
) {
  render(
    <TerminalCommandsModal
      open
      projectId={PROJECT}
      root={ROOT}
      worktree={worktree}
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
    // Recipes launch a terminal named after the recipe, not `just <recipe>`.
    expect(onRun).toHaveBeenCalledWith("just build", "build");
    expect(onClose).toHaveBeenCalled();
  });

  it("adds and removes a custom command, persisting per project root", async () => {
    renderModal();
    const input = await screen.findByLabelText("New command");
    fireEvent.change(input, { target: { value: "pnpm dev" } });
    fireEvent.submit(input.closest("form")!);

    expect(screen.getByText("pnpm dev")).toBeInTheDocument();
    expect(useTerminalCommandsStore.getState().customByRoot[ROOT]).toEqual([
      { command: "pnpm dev" },
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

  it("names a custom command and launches the terminal under that name", async () => {
    const { onRun } = renderModal();
    fireEvent.change(await screen.findByLabelText("Command name"), {
      target: { value: "Web dev" },
    });
    const input = screen.getByLabelText("New command");
    fireEvent.change(input, { target: { value: "pnpm dev" } });
    fireEvent.submit(input.closest("form")!);

    expect(useTerminalCommandsStore.getState().customByRoot[ROOT]).toEqual([
      { command: "pnpm dev", name: "Web dev" },
    ]);
    // Both the name and the command line show; the terminal takes the name.
    expect(screen.getByText("Web dev")).toBeInTheDocument();
    fireEvent.click(screen.getByTitle('Run "pnpm dev" in a new terminal'));
    expect(onRun).toHaveBeenCalledWith("pnpm dev", "Web dev");
  });

  it("edits an existing custom command in place", async () => {
    useTerminalCommandsStore.setState({
      customByRoot: { [ROOT]: [{ command: "pnpm dev" }] },
    });
    renderModal();

    fireEvent.click(
      await screen.findByRole("button", { name: 'Edit command "pnpm dev"' }),
    );
    const command = screen.getByLabelText("Command");
    expect((command as HTMLInputElement).value).toBe("pnpm dev");
    fireEvent.change(screen.getByLabelText("Command name"), {
      target: { value: "API" },
    });
    fireEvent.change(command, { target: { value: "pnpm dev:api" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(useTerminalCommandsStore.getState().customByRoot[ROOT]).toEqual([
      { command: "pnpm dev:api", name: "API" },
    ]);
    // Back to add mode with an empty draft.
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
  });

  it("migrates commands stored as bare strings", async () => {
    localStorage.setItem(
      "podium.terminalCommands",
      JSON.stringify({ customByRoot: { [ROOT]: ["pnpm dev"] } }),
    );
    // The store reads localStorage at module init, so re-import it to run the
    // loader against the legacy payload.
    vi.resetModules();
    const fresh = await import("../state/terminalCommandsStore");
    expect(
      fresh.useTerminalCommandsStore.getState().customByRoot[ROOT],
    ).toEqual([{ command: "pnpm dev" }]);
  });

  it("shows a hint when there are no commands at all", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockResolvedValueOnce([]);
    renderModal();
    await waitFor(() =>
      expect(screen.getByText(/no justfile recipes/i)).toBeInTheDocument(),
    );
  });

  it("titles the modal plainly when opened for the project root", async () => {
    renderModal();
    await screen.findByText("just build");
    expect(screen.getByText("Commands")).toBeInTheDocument();
  });

  it("titles the modal with the worktree name and runs commands there", async () => {
    const { onRun } = renderModal(undefined, undefined, wt("fix-login"));

    expect(screen.getByText("Commands — fix-login")).toBeInTheDocument();
    fireEvent.click(
      await screen.findByTitle('Run "just build" in a new terminal'),
    );

    expect(onRun).toHaveBeenCalledWith(
      "just build",
      "build",
      ".podium/worktrees/fix-login",
    );
  });

  it("runs a command with no cwd when opened for the project root", async () => {
    const { onRun } = renderModal(undefined, undefined, null);

    fireEvent.click(
      await screen.findByTitle('Run "just build" in a new terminal'),
    );

    expect(onRun).toHaveBeenCalledWith("just build", "build");
  });
});
