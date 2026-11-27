import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ProjectInfo } from "../../ipc/types";
import { useAgentActivityStore } from "../../state/agentActivityStore";
import { useFloorStore } from "../../state/floorStore";
import { useLayoutStore } from "../../state/layoutStore";
import { useProcessStore } from "../../state/processStore";
import { useProjectStore } from "../../state/projectStore";
import { followCamera } from "./follow";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/commands", () => ({ toIpcError: vi.fn() }));
vi.mock("../terminalRegistry", () => ({ disposeTerminal: vi.fn() }));

const project = (id: string) =>
  ({
    id,
    name: id,
    root: `/fake/${id}`,
    sections: {
      todos: true,
      terminals: true,
      worktrees: true,
      scratchpads: true,
      issues: true,
    },
  }) as ProjectInfo;

beforeEach(() => {
  useFloorStore.getState().reset();
  useProjectStore.setState({ projects: [project("a"), project("b")] });
  useProcessStore.setState({ processes: [] });
  useAgentActivityStore.setState({ details: {} });
  useLayoutStore.setState({ floorPositions: {} });
});

describe("followCamera", () => {
  it("fires on camera moves and on room moves, not on other store changes", () => {
    const cb = vi.fn();
    const off = followCamera(cb);

    useFloorStore.getState().setCamera({ x: 5, y: 0, z: 1 }, { animate: true });
    expect(cb).toHaveBeenLastCalledWith(true);

    cb.mockClear();
    useAgentActivityStore.setState({ details: {} });
    useFloorStore.getState().setHoverDesk("x");
    expect(cb).not.toHaveBeenCalled();

    // A room moved (a drag drop, or a push by a grown room): re-place.
    useLayoutStore.getState().setFloorPosition("/fake/b", [4000, 4000]);
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenLastCalledWith(false);

    off();
    useLayoutStore.getState().setFloorPosition("/fake/b", [8000, 8000]);
    useFloorStore.getState().setCamera({ x: 9, y: 0, z: 1 });
    expect(cb).toHaveBeenCalledTimes(1);
  });
});
