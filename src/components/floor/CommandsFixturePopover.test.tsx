import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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

  describe("search", () => {
    const search = () => screen.getByLabelText("Search commands");
    const type = (v: string) =>
      fireEvent.change(search(), { target: { value: v } });

    it("focuses the field on open", async () => {
      renderPopover();
      await screen.findByText("Dev");
      expect(document.activeElement).toBe(search());
    });

    it("filters by name and by command text, case-insensitively", async () => {
      renderPopover();
      await screen.findByText("Dev");
      type("BUI");
      expect(screen.queryByText("Dev")).toBeNull();
      expect(screen.getByText("build")).toBeTruthy();
      type("pnpm");
      expect(screen.getByText("Dev")).toBeTruthy();
      expect(screen.queryByText("build")).toBeNull();
    });

    it("shows a no-match message", async () => {
      renderPopover();
      await screen.findByText("Dev");
      type("zzz");
      expect(screen.getByText('No commands match "zzz".')).toBeTruthy();
    });

    it("moves with arrows (wrapping) and runs the highlighted row on Enter", async () => {
      renderPopover();
      await screen.findByText("Dev");
      fireEvent.keyDown(search(), { key: "ArrowDown" });
      fireEvent.keyDown(search(), { key: "ArrowDown" });
      fireEvent.keyDown(search(), { key: "ArrowDown" }); // wraps to the first
      fireEvent.keyDown(search(), { key: "ArrowUp" }); // wraps to the last
      fireEvent.keyDown(search(), { key: "Enter" });
      expect(run).toHaveBeenCalledTimes(1);
      const rows = ["just build", "pnpm dev"];
      expect(rows).toContain(run.mock.calls[0][1]);
      expect(run.mock.calls[0][1]).toBe("pnpm dev");
    });

    it("keeps the highlight on the same command when recipes load in front", async () => {
      let resolve!: (r: string[]) => void;
      recipes.mockReturnValue(new Promise((r) => (resolve = r)));
      renderPopover();
      await screen.findByText("Dev");
      fireEvent.keyDown(search(), { key: "ArrowDown" });
      await act(async () => resolve(["build", "test"]));
      expect(screen.getByText("build")).toBeTruthy();
      fireEvent.keyDown(search(), { key: "Enter" });
      expect(run).toHaveBeenCalledWith(
        expect.objectContaining({ id: "p1" }),
        "pnpm dev",
        "Dev",
      );
    });

    it("clears the highlight when its command goes away", async () => {
      renderPopover();
      await screen.findByText("Dev");
      fireEvent.keyDown(search(), { key: "ArrowUp" }); // the last row: Dev
      expect(search().getAttribute("aria-activedescendant")).not.toBeNull();
      act(() =>
        useTerminalCommandsStore.setState({
          customByRoot: { [project.root]: [] },
        }),
      );
      expect(search().getAttribute("aria-activedescendant")).toBeNull();
      fireEvent.keyDown(search(), { key: "Enter" });
      expect(run).toHaveBeenCalledWith(
        expect.objectContaining({ id: "p1" }),
        "just build",
        "build",
      );
    });

    it("runs the first match on Enter without a highlight", async () => {
      renderPopover();
      await screen.findByText("Dev");
      type("dev");
      fireEvent.keyDown(search(), { key: "Enter" });
      expect(run).toHaveBeenCalledWith(
        expect.objectContaining({ id: "p1" }),
        "pnpm dev",
        "Dev",
      );
    });

    it("Esc clears the text first, then lets Esc close the popover", async () => {
      const onKey = vi.fn();
      document.addEventListener("keydown", onKey);
      useFloorStore
        .getState()
        .openPopover({ projectId: "p1", kind: "commands", itemId: null });
      renderPopover();
      await screen.findByText("Dev");
      type("dev");
      fireEvent.keyDown(search(), { key: "Escape" });
      expect((search() as HTMLInputElement).value).toBe("");
      expect(onKey).not.toHaveBeenCalled();
      expect(useFloorStore.getState().popover).not.toBeNull();
      fireEvent.keyDown(search(), { key: "Escape" });
      expect(useFloorStore.getState().popover).toBeNull();
      document.removeEventListener("keydown", onKey);
    });
  });

  it("gives the name priority and the full text in the title", async () => {
    renderPopover();
    const name = await screen.findByText("Dev");
    const cmd = screen.getByText("pnpm dev");
    expect(name.className).toMatch(/name/);
    expect(name.className).not.toMatch(/command/);
    expect(cmd.className).toMatch(/command/);
    const title = name.closest("button")?.getAttribute("title");
    expect(title).toContain("Dev");
    expect(title).toContain("pnpm dev");
  });
});
