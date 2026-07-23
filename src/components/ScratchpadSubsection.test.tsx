import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The stores pull in the IPC layer; jsdom has no Tauri bridge.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve([])),
  Channel: class {
    onmessage: (message: unknown) => void = () => undefined;
  },
}));

import type { ScratchpadInfo } from "../ipc/types";
import { useProjectStore } from "../state/projectStore";
import { useScratchpadStore } from "../state/scratchpadStore";
import { ScratchpadSubsection } from "./ScratchpadSubsection";

const initialScratchpad = useScratchpadStore.getState();
const initialProject = useProjectStore.getState();

const PROJECT = "proj-1";

function scratchpad(id: string, title: string): ScratchpadInfo {
  return {
    id,
    projectId: PROJECT,
    title,
    content: "",
    archived: false,
    archivedAt: null,
    createdAt: "2024-04-03T12:00:00Z",
    updatedAt: "2024-04-03T12:00:00Z",
    updatedBy: "User",
    version: 1,
    tags: [],
    assignedAgent: null,
  };
}

/** Seed the stores the subsection reads, with spy-able actions. */
function seed(scratchpads: ScratchpadInfo[]) {
  const refresh = vi.fn(() => Promise.resolve());
  const addScratchpad = vi.fn(() =>
    Promise.resolve(scratchpad("new-1", "Untitled")),
  );
  const setScratchpadArchived = vi.fn(() =>
    Promise.resolve(scratchpad("a", "Scratchpad A")),
  );
  const refreshArchived = vi.fn(() => Promise.resolve());
  useScratchpadStore.setState(
    {
      ...initialScratchpad,
      scratchpadsByProject: { [PROJECT]: scratchpads },
      refresh,
      addScratchpad,
      setScratchpadArchived,
      refreshArchived,
    },
    true,
  );
  const setActiveProject = vi.fn();
  useProjectStore.setState({ ...initialProject, setActiveProject }, true);
  return {
    refresh,
    addScratchpad,
    setScratchpadArchived,
    refreshArchived,
    setActiveProject,
  };
}

/** The click target for a scratchpad's title (opens / drives selection). */
function titleOf(text: string): HTMLElement {
  const el = screen.getByText(text).closest("button");
  if (!el) throw new Error(`no title button for "${text}"`);
  return el;
}

describe("ScratchpadSubsection", () => {
  beforeEach(() => {
    useScratchpadStore.setState(initialScratchpad, true);
    useProjectStore.setState(initialProject, true);
  });

  it("renders_scratchpad_list_for_project", () => {
    seed([scratchpad("a", "Scratchpad A"), scratchpad("b", "Scratchpad B")]);
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    expect(screen.getByText("Scratchpad A")).toBeInTheDocument();
    expect(screen.getByText("Scratchpad B")).toBeInTheDocument();
  });

  it("clicking_add_creates_scratchpad_and_opens_detail", async () => {
    const { addScratchpad } = seed([]);
    const onOpenScratchpad = vi.fn();
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={onOpenScratchpad}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "New scratchpad" }));

    expect(addScratchpad).toHaveBeenCalledWith(PROJECT);
    await waitFor(() =>
      expect(onOpenScratchpad).toHaveBeenCalledWith(PROJECT, "new-1"),
    );
  });

  it("opens a scratchpad on a plain title click", () => {
    seed([scratchpad("a", "Scratchpad A")]);
    const onOpenScratchpad = vi.fn();
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={onOpenScratchpad}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(titleOf("Scratchpad A"));
    expect(onOpenScratchpad).toHaveBeenCalledWith(PROJECT, "a");
  });

  it("shows the empty hint when there are no scratchpads", () => {
    seed([]);
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    expect(screen.getByText("No scratchpads yet.")).toBeInTheDocument();
  });

  it("archives a scratchpad via its row action", () => {
    const { setScratchpadArchived } = seed([scratchpad("a", "Scratchpad A")]);
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText('Archive scratchpad "Scratchpad A"'));
    expect(setScratchpadArchived).toHaveBeenCalledWith(PROJECT, "a", true);
  });

  it("opens the archive modal from the header button", async () => {
    const { refreshArchived } = seed([scratchpad("a", "Scratchpad A")]);
    render(
      <ScratchpadSubsection
        projectId={PROJECT}
        onOpenScratchpad={vi.fn()}
        onPickAgent={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByLabelText("View archived scratchpads"));
    await waitFor(() => expect(refreshArchived).toHaveBeenCalledWith(PROJECT));
    expect(screen.getByText("Archived scratchpads")).toBeInTheDocument();
  });

  describe("spawn-agent hover button", () => {
    it("always opens the agent picker for a single scratchpad, never spawns directly", () => {
      const { setActiveProject } = seed([scratchpad("a", "Scratchpad A")]);
      const onPickAgent = vi.fn();
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={vi.fn()}
          onPickAgent={onPickAgent}
        />,
      );

      fireEvent.click(
        screen.getByRole("button", {
          name: 'Start an agent on "Scratchpad A"',
        }),
      );

      expect(onPickAgent).toHaveBeenCalledWith(PROJECT, ["a"], "Scratchpad A");
      expect(setActiveProject).toHaveBeenCalledWith(PROJECT);
    });
  });

  describe("multi-select", () => {
    it("opens a scratchpad on a plain click, without selecting", () => {
      seed([scratchpad("a", "Scratchpad A"), scratchpad("b", "Scratchpad B")]);
      const onOpenScratchpad = vi.fn();
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={onOpenScratchpad}
          onPickAgent={vi.fn()}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"));

      expect(onOpenScratchpad).toHaveBeenCalledWith(PROJECT, "a");
      expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
    });

    it("toggles a selection with Cmd/Ctrl+click and shows the selection bar", () => {
      seed([
        scratchpad("a", "Scratchpad A"),
        scratchpad("b", "Scratchpad B"),
        scratchpad("c", "Scratchpad C"),
      ]);
      const onOpenScratchpad = vi.fn();
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={onOpenScratchpad}
          onPickAgent={vi.fn()}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      fireEvent.click(titleOf("Scratchpad C"), { ctrlKey: true });

      // Selecting must not open the detail pane.
      expect(onOpenScratchpad).not.toHaveBeenCalled();
      expect(
        screen.getByText("Start agent on 2 scratchpads"),
      ).toBeInTheDocument();

      // Toggling one off drops the bar below 2.
      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
    });

    it("extends a range with Shift+click", () => {
      seed([
        scratchpad("a", "Scratchpad A"),
        scratchpad("b", "Scratchpad B"),
        scratchpad("c", "Scratchpad C"),
      ]);
      const onPickAgent = vi.fn();
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={vi.fn()}
          onPickAgent={onPickAgent}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      fireEvent.click(titleOf("Scratchpad C"), { shiftKey: true });

      fireEvent.click(screen.getByText("Start agent on 3 scratchpads"));
      // A group has no single sensible name — blank, so the agent self-names.
      expect(onPickAgent).toHaveBeenCalledWith(PROJECT, ["a", "b", "c"], "");
    });

    it("the selection bar's action opens the picker for all selected ids and clears the selection", () => {
      const { setActiveProject } = seed([
        scratchpad("a", "Scratchpad A"),
        scratchpad("b", "Scratchpad B"),
      ]);
      const onPickAgent = vi.fn();
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={vi.fn()}
          onPickAgent={onPickAgent}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      fireEvent.click(titleOf("Scratchpad B"), { metaKey: true });
      fireEvent.click(screen.getByText("Start agent on 2 scratchpads"));

      expect(onPickAgent).toHaveBeenCalledWith(PROJECT, ["a", "b"], "");
      expect(setActiveProject).toHaveBeenCalledWith(PROJECT);
      expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
    });

    it("hides the bar with only one selected", () => {
      seed([scratchpad("a", "Scratchpad A"), scratchpad("b", "Scratchpad B")]);
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={vi.fn()}
          onPickAgent={vi.fn()}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });

      expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
    });

    it("clears the selection via the clear button", () => {
      seed([scratchpad("a", "Scratchpad A"), scratchpad("b", "Scratchpad B")]);
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={vi.fn()}
          onPickAgent={vi.fn()}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      fireEvent.click(titleOf("Scratchpad B"), { metaKey: true });
      expect(
        screen.getByText("Start agent on 2 scratchpads"),
      ).toBeInTheDocument();

      fireEvent.click(screen.getByLabelText("Clear selection"));
      expect(screen.queryByText(/Start agent on/)).not.toBeInTheDocument();
    });

    it("keeps the selection when opening another scratchpad with a plain click", () => {
      const onOpenScratchpad = vi.fn();
      seed([
        scratchpad("a", "Scratchpad A"),
        scratchpad("b", "Scratchpad B"),
        scratchpad("c", "Scratchpad C"),
      ]);
      render(
        <ScratchpadSubsection
          projectId={PROJECT}
          onOpenScratchpad={onOpenScratchpad}
          onPickAgent={vi.fn()}
        />,
      );

      fireEvent.click(titleOf("Scratchpad A"), { metaKey: true });
      fireEvent.click(titleOf("Scratchpad B"), { metaKey: true });
      expect(
        screen.getByText("Start agent on 2 scratchpads"),
      ).toBeInTheDocument();

      fireEvent.click(titleOf("Scratchpad C"));

      expect(onOpenScratchpad).toHaveBeenCalledWith(PROJECT, "c");
      expect(
        screen.getByText("Start agent on 2 scratchpads"),
      ).toBeInTheDocument();
    });
  });
});
