import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const recipes = vi.hoisted(() => vi.fn());
const run = vi.hoisted(() => vi.fn());
vi.mock("../../ipc/commands", () => ({ projectJustRecipes: recipes }));
vi.mock("../../lib/terminals", () => ({ runCommand: run }));
vi.mock("../TerminalCommandsModal", () => ({
  TerminalCommandsModal: ({ open }: { open: boolean }) =>
    open ? <div role="dialog">Commands modal</div> : null,
}));

import type { ProjectInfo } from "../../ipc/types";
import { useFloorStore } from "../../state/floorStore";
import { useProjectStore } from "../../state/projectStore";
import { useTerminalCommandsStore } from "../../state/terminalCommandsStore";
import { CommandsFixturePopover } from "./CommandsFixturePopover";

const project = { id: "p1", name: "Demo", root: "/tmp/cmd-popover" };

function renderPopover() {
  const anchor = document.createElement("button");
  document.body.append(anchor);
  return render(
    <CommandsFixturePopover projectId="p1" itemId={null} anchor={anchor} />,
  );
}

beforeEach(() => {
  run.mockReset();
  recipes.mockReset().mockResolvedValue(["build"]);
  useFloorStore.getState().reset();
  useProjectStore.setState({ projects: [project as ProjectInfo] }, false);
  useTerminalCommandsStore.setState({
    customByRoot: { [project.root]: [{ command: "pnpm dev", name: "Dev" }] },
  });
});

describe("CommandsFixturePopover", () => {
  it("runs a row's command and closes the popover", async () => {
    useFloorStore
      .getState()
      .openPopover({ projectId: "p1", kind: "commands", itemId: null });
    renderPopover();
    fireEvent.click(await screen.findByRole("button", { name: /^build/ }));
    expect(run).toHaveBeenCalledWith(
      expect.objectContaining({ id: "p1" }),
      "just build",
      "build",
    );
    expect(useFloorStore.getState().popover).toBeNull();
  });

  it("shows the command text muted when it differs from the name", async () => {
    renderPopover();
    expect(await screen.findByText("pnpm dev")).toBeTruthy();
    expect(screen.getByText("Dev")).toBeTruthy();
  });

  it("opens the commands modal from Edit commands", async () => {
    renderPopover();
    await screen.findByText("Dev");
    fireEvent.click(screen.getByLabelText("Edit commands"));
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("shows the empty state", async () => {
    recipes.mockResolvedValue([]);
    useTerminalCommandsStore.setState({ customByRoot: {} });
    renderPopover();
    await waitFor(() =>
      expect(
        screen.getByText("No commands yet. Add one with Edit commands."),
      ).toBeTruthy(),
    );
  });
});
