import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const recipes = vi.hoisted(() => vi.fn());
vi.mock("../ipc/commands", () => ({ projectJustRecipes: recipes }));

import { useTerminalCommandsStore } from "../state/terminalCommandsStore";
import { useTerminalCommands } from "./terminalCommands";

const project = { id: "p1", root: "/tmp/hook-project" };

beforeEach(() => {
  recipes.mockReset().mockResolvedValue(["build"]);
  useTerminalCommandsStore.setState({ customByRoot: {} });
});

describe("useTerminalCommands", () => {
  it("lists justfile recipes, then saved commands", async () => {
    const { result } = renderHook(() => useTerminalCommands(project));
    await waitFor(() => expect(result.current.commands).toHaveLength(1));
    act(() => result.current.add("pnpm dev", "Dev"));
    expect(result.current.commands).toEqual([
      { command: "just build", name: "build", index: null },
      { command: "pnpm dev", name: "Dev", index: 0 },
    ]);
  });

  it("removes a saved command", () => {
    recipes.mockResolvedValue([]);
    const { result } = renderHook(() => useTerminalCommands(project));
    act(() => result.current.add("ls"));
    expect(result.current.commands[0].name).toBe("ls");
    act(() => result.current.remove(0));
    expect(result.current.commands).toEqual([]);
  });

  it("does not scan while disabled and survives a failed scan", async () => {
    renderHook(() => useTerminalCommands(project, false));
    expect(recipes).not.toHaveBeenCalled();
    recipes.mockRejectedValue(new Error("x"));
    const { result } = renderHook(() => useTerminalCommands(project));
    await waitFor(() => expect(recipes).toHaveBeenCalled());
    expect(result.current.commands).toEqual([]);
  });
});
