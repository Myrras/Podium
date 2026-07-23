import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The store pulls in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { ScratchpadInfo } from "../ipc/types";
import { useScratchpadStore } from "../state/scratchpadStore";
import { ScratchpadArchiveModal } from "./ScratchpadArchiveModal";

const initial = useScratchpadStore.getState();
const PROJECT = "proj-1";

function scratchpad(id: string, title: string): ScratchpadInfo {
  return {
    id,
    projectId: PROJECT,
    title,
    content: "",
    archived: true,
    archivedAt: "2024-04-05T09:30:00Z",
    createdAt: "2024-04-03T12:00:00Z",
    updatedAt: "2024-04-03T12:00:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: null,
  };
}

function seed(archived: ScratchpadInfo[]) {
  const refreshArchived = vi.fn(() => Promise.resolve());
  const setScratchpadArchived = vi.fn(() => Promise.resolve(null));
  const removeScratchpad = vi.fn(() => Promise.resolve());
  useScratchpadStore.setState(
    {
      ...initial,
      archivedByProject: { [PROJECT]: archived },
      refreshArchived,
      setScratchpadArchived,
      removeScratchpad,
    },
    true,
  );
  return { refreshArchived, setScratchpadArchived, removeScratchpad };
}

describe("ScratchpadArchiveModal", () => {
  beforeEach(() => {
    useScratchpadStore.setState(initial, true);
  });

  it("loads the archived list when it opens", () => {
    const { refreshArchived } = seed([]);
    render(
      <ScratchpadArchiveModal
        open
        projectId={PROJECT}
        onClose={() => undefined}
      />,
    );
    expect(refreshArchived).toHaveBeenCalledWith(PROJECT);
  });

  it("restores an archived scratchpad", () => {
    const { setScratchpadArchived } = seed([scratchpad("s1", "Old notes")]);
    render(
      <ScratchpadArchiveModal
        open
        projectId={PROJECT}
        onClose={() => undefined}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: 'Restore "Old notes"' }),
    );
    expect(setScratchpadArchived).toHaveBeenCalledWith(PROJECT, "s1", false);
  });

  it("deletes an archived scratchpad (parity with to-dos)", () => {
    const { removeScratchpad } = seed([scratchpad("s1", "Old notes")]);
    render(
      <ScratchpadArchiveModal
        open
        projectId={PROJECT}
        onClose={() => undefined}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: 'Delete "Old notes"' }));
    expect(removeScratchpad).toHaveBeenCalledWith(PROJECT, "s1");
  });
});
