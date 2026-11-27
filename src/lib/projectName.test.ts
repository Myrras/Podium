import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { useProjectStore } from "../state/projectStore";
import { commitProjectName } from "./projectName";

describe("commitProjectName", () => {
  it("trims, clears a blank name and skips an unchanged one", () => {
    const renameProject = vi.fn(() => Promise.resolve());
    useProjectStore.setState({ renameProject });
    const project = { id: "p1", name: "Acme" };
    commitProjectName(project, "  Beta ");
    expect(renameProject).toHaveBeenLastCalledWith("p1", "Beta");
    commitProjectName(project, "   ");
    expect(renameProject).toHaveBeenLastCalledWith("p1", null);
    commitProjectName(project, " Acme ");
    expect(renameProject).toHaveBeenCalledTimes(2);
  });
});
